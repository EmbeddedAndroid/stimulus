use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use lp_core::{
    ToolError,
    ops::{self, Dispatcher, OpSpec},
};
use lp_project::{Capture, CaptureStore, MeasurementKind, Project};
use lp_sim::{SimSeed, StartState};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{
    Arc, Mutex, RwLock, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use crate::acquisitions::AcquisitionBackend;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconnectRealError {
    Open(String),
    Setup(String),
}

impl ReconnectRealError {
    pub fn is_setup(&self) -> bool {
        matches!(self, Self::Setup(_))
    }
}

impl std::fmt::Display for ReconnectRealError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open(error) => write!(formatter, "open real backend: {error}"),
            Self::Setup(error) => write!(formatter, "apply setup after reconnect: {error}"),
        }
    }
}

/// User/agent-facing instruction when software recovery of a wedged command
/// channel is exhausted. Shared by the recovery worker, /api/health, and the
/// acquire error so the same actionable message reaches every surface.
pub const REPLUG_HINT: &str = "The logic analyzer stopped responding and automatic recovery failed. \
     Unplug and replug its USB cable.";

/// True when an error indicates the LogicPort command channel has wedged at the
/// USB/FT245/FPGA level -- the device is still enumerated but stops answering
/// commands -- as opposed to a benign protocol-framing error that a single
/// retry or the link-level resync already handles. These are the faults that
/// warrant escalating to a software USB device reset and, if that fails, asking
/// the user to replug the cable.
///
/// Deliberately narrow: a packet-number mismatch, an unexpected opcode, or a
/// setup/argument error is NOT a wedge (the link self-heals those, and forcing
/// a device reset on them would turn a recoverable fault into churn).
pub fn is_command_wedge(message: &str) -> bool {
    // Zero bytes back from a command read: the FT245 device->host FIFO is dead
    // (e.g. "timed out waiting for 3 protocol bytes; got 0").
    (message.contains("timed out waiting for") && message.contains("got 0"))
        // usbfs cancelled the transfer mid-flight: stale endpoint state.
        || message.contains("transfer was cancelled")
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Context>,
}
struct Context {
    self_ref: Weak<Context>,
    device_kind: &'static str,
    mcp: lp_mcp::McpServer,
    project: RwLock<Project>,
    project_path: RwLock<Option<PathBuf>>,
    recent_projects: Mutex<VecDeque<PathBuf>>,
    exit_requested: AtomicBool,
    captures: CaptureStore,
    acquisition: Mutex<Box<dyn crate::acquisitions::AcquisitionBackend>>,
    device_connected: AtomicBool,
    device_error: RwLock<Option<String>>,
    // Set when automatic (software-only) recovery of a wedged command channel
    // has been exhausted and the device is still enumerated: the only remaining
    // fix is a manual USB replug. Surfaced to the user/agent so no one is left
    // staring at an opaque 502. Cleared automatically once a reconnect succeeds
    // (e.g. after the user replugs).
    device_replug_required: AtomicBool,
    device_epoch: std::sync::atomic::AtomicU64,
    acquisition_status: RwLock<crate::acquisitions::AcqStatus>,
    recurring: AtomicBool,
    events: Mutex<VecDeque<crate::events::Event>>,
    event_tx: broadcast::Sender<crate::events::Event>,
    next_event_seq: std::sync::atomic::AtomicU64,
}
impl AppState {
    pub fn new() -> Self {
        let seed = SimSeed {
            start_state: StartState::Warm { image: 7 },
            ..SimSeed::default()
        };
        let mut backend = crate::acquisitions::SimBackend::new(seed);
        let project = Project::new("1970-01-01T00:00:00Z");
        // Default simulator setup is structurally valid by construction.
        let _ = backend.apply_setup(&project.settings);
        Self::with_initialized_backend(Box::new(backend), "sim", project)
    }
    pub fn real() -> Result<Self, String> {
        let mut backend = crate::acquisitions::RealBackend::open_transport()?;
        backend.establish_session()?;
        let project = Project::new("1970-01-01T00:00:00Z");
        backend
            .apply_setup(&project.settings)
            .map_err(|error| format!("apply default setup to real backend: {error}"))?;
        Ok(Self::with_initialized_backend(
            Box::new(backend),
            "real",
            project,
        ))
    }
    pub fn real_pending(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let state = Self::with_initialized_backend(
            Box::new(crate::acquisitions::OfflineBackend::new(reason.clone())),
            "real",
            Project::new("1970-01-01T00:00:00Z"),
        );
        state.inner.device_connected.store(false, Ordering::Release);
        if let Ok(mut error) = state.inner.device_error.write() {
            *error = Some(reason);
        }
        state
    }
    pub fn reconnect_real(&self) -> Result<(), ReconnectRealError> {
        let mut backend =
            crate::acquisitions::RealBackend::open_transport().map_err(ReconnectRealError::Open)?;
        let settings = self
            .inner
            .project
            .read()
            .map_err(|_| ReconnectRealError::Open("project lock poisoned".to_owned()))?
            .settings
            .clone();
        let session = backend.establish_session();
        let mut acquisition = self
            .inner
            .acquisition
            .lock()
            .map_err(|_| ReconnectRealError::Open("acquisition lock poisoned".to_owned()))?;
        // Install the live backend before setup. If setup fails, retaining this
        // exact FTDI session prevents the recovery worker from repeatedly
        // reopening it and injecting C3/0x61 commands into a parser whose
        // state is the evidence we need to preserve.
        *acquisition = Box::new(backend);
        if let Err(error) = session {
            let error = ReconnectRealError::Setup(format!("establish FPGA session: {error}"));
            self.inner.device_connected.store(false, Ordering::Release);
            if let Ok(mut current) = self.inner.device_error.write() {
                *current = Some(error.to_string());
            }
            self.emit(
                "device",
                json!({"state":"session_failed","error":error.to_string(),"session_retained":true}),
            );
            return Err(error);
        }
        if let Err(error) = acquisition.apply_setup(&settings) {
            let error = ReconnectRealError::Setup(error.to_string());
            self.inner.device_connected.store(false, Ordering::Release);
            if let Ok(mut current) = self.inner.device_error.write() {
                *current = Some(error.to_string());
            }
            self.emit(
                "device",
                json!({"state":"setup_failed","error":error.to_string(),"session_retained":true}),
            );
            return Err(error);
        }
        drop(acquisition);
        self.inner.device_connected.store(true, Ordering::Release);
        // A successful reconnect (software reset that took, or a physical
        // replug) clears any outstanding replug request.
        self.inner
            .device_replug_required
            .store(false, Ordering::Release);
        if let Ok(mut error) = self.inner.device_error.write() {
            *error = None;
        }
        let epoch = self
            .inner
            .device_epoch
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        self.emit("device", json!({"state":"connected","device_epoch":epoch}));
        Ok(())
    }
    pub fn record_connection_error(&self, reason: impl Into<String>) {
        self.inner.device_connected.store(false, Ordering::Release);
        if let Ok(mut error) = self.inner.device_error.write() {
            *error = Some(reason.into());
        }
    }
    pub fn check_real_connection(&self) -> Result<(), String> {
        self.inner
            .acquisition
            .lock()
            .map_err(|_| "acquisition lock poisoned".to_owned())?
            .health_check()
            .map_err(|error| error.to_string())
    }
    pub fn disconnect_real(&self, reason: impl Into<String>) -> Result<(), String> {
        let reason = reason.into();
        *self
            .inner
            .acquisition
            .lock()
            .map_err(|_| "acquisition lock poisoned".to_owned())? =
            Box::new(crate::acquisitions::OfflineBackend::new(reason.clone()));
        self.record_connection_error(reason.clone());
        self.emit("device", json!({"state":"disconnected","error":reason}));
        Ok(())
    }
    /// Software-only recovery of a wedged command channel has been exhausted
    /// while the device is still enumerated: the only remaining fix is a manual
    /// USB replug. Park the backend Offline so nothing pokes the wedged parser,
    /// and surface a clear instruction. The flag clears automatically the next
    /// time a reconnect succeeds (after the replug, or a software reset that
    /// finally takes -- the recovery worker keeps trying).
    pub fn require_replug(&self, reason: impl Into<String>) {
        let reason = reason.into();
        if let Ok(mut backend) = self.inner.acquisition.lock() {
            *backend = Box::new(crate::acquisitions::OfflineBackend::new(reason.clone()));
        }
        self.record_connection_error(reason.clone());
        self.inner
            .device_replug_required
            .store(true, Ordering::Release);
        self.emit("device", json!({"state":"needs_replug","error":reason}));
    }
    pub fn needs_replug(&self) -> bool {
        self.inner.device_replug_required.load(Ordering::Acquire)
    }
    /// Trigger automatic recovery after an operation hit a command-path wedge:
    /// park the backend Offline so the recovery worker reopens (and, per
    /// `is_command_wedge`, escalates to a software USB reset) instead of leaving
    /// every subsequent request to time out against the dead channel.
    pub fn begin_recovery(&self, reason: impl Into<String>) {
        let _ = self.disconnect_real(reason);
    }
    fn with_initialized_backend(
        backend: Box<dyn crate::acquisitions::AcquisitionBackend>,
        device_kind: &'static str,
        project: Project,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            inner: Arc::new_cyclic(|self_ref| Context {
                self_ref: self_ref.clone(),
                device_kind,
                mcp: lp_mcp::McpServer::default(),
                project: RwLock::new(project),
                project_path: RwLock::new(None),
                recent_projects: Mutex::new(VecDeque::with_capacity(10)),
                exit_requested: AtomicBool::new(false),
                captures: CaptureStore::new(16).unwrap_or_else(|e| panic!("{e}")),
                acquisition: Mutex::new(backend),
                device_connected: AtomicBool::new(true),
                device_error: RwLock::new(None),
                device_replug_required: AtomicBool::new(false),
                device_epoch: std::sync::atomic::AtomicU64::new(1),
                acquisition_status: RwLock::new(crate::acquisitions::AcqStatus::default()),
                recurring: AtomicBool::new(false),
                events: Mutex::new(VecDeque::with_capacity(1000)),
                event_tx,
                next_event_seq: std::sync::atomic::AtomicU64::new(1),
            }),
        }
    }
    pub fn insert_capture(&self, capture: Capture) -> Result<Capture, ApiError> {
        let capture = self
            .inner
            .captures
            .insert(capture)
            .map_err(|e| api_error("INTERNAL", e.to_string()))?;
        self.emit(
            "capture_ready",
            json!({"capture":capture,"acq_count":capture.seq}),
        );
        Ok(capture)
    }
    pub(crate) fn captures(&self) -> &CaptureStore {
        &self.inner.captures
    }
    pub(crate) fn clear_captures(&self) -> Result<(), ApiError> {
        self.inner
            .captures
            .clear()
            .map_err(|error| api_error("INTERNAL", error.to_string()))
    }
    pub(crate) fn acquire_single(
        &self,
    ) -> Result<Capture, lp_device::acquisition::AcquisitionError> {
        self.inner
            .acquisition
            .lock()
            .map_err(|_| {
                lp_device::acquisition::AcquisitionError::Capture(
                    "acquisition lock poisoned".into(),
                )
            })?
            .single()
    }
    pub(crate) fn halt_acquisition(
        &self,
    ) -> Result<lp_proto::status::Phase, lp_device::acquisition::AcquisitionError> {
        self.inner
            .acquisition
            .lock()
            .map_err(|_| {
                lp_device::acquisition::AcquisitionError::Capture(
                    "acquisition lock poisoned".into(),
                )
            })?
            .halt()
    }
    pub(crate) fn trigger_immediate(
        &self,
    ) -> Result<lp_proto::status::Phase, lp_device::acquisition::AcquisitionError> {
        self.inner
            .acquisition
            .lock()
            .map_err(|_| {
                lp_device::acquisition::AcquisitionError::Capture(
                    "acquisition lock poisoned".into(),
                )
            })?
            .immediate()
    }
    pub(crate) fn acquisition_status(&self) -> Result<crate::acquisitions::AcqStatus, ApiError> {
        self.inner
            .acquisition_status
            .read()
            .map(|status| status.clone())
            .map_err(|_| api_error("INTERNAL", "acquisition status lock poisoned"))
    }
    pub(crate) fn set_acq_state(&self, value: &'static str) -> Result<(), ApiError> {
        self.inner
            .acquisition_status
            .write()
            .map_err(|_| api_error("INTERNAL", "acquisition status lock poisoned"))?
            .state = value;
        Ok(())
    }
    pub(crate) fn set_capture_ready(&self, samples: u64) -> Result<(), ApiError> {
        let mut status = self
            .inner
            .acquisition_status
            .write()
            .map_err(|_| api_error("INTERNAL", "acquisition status lock poisoned"))?;
        status.state = "ready";
        status.samples = samples;
        status.acq_count = status.acq_count.saturating_add(1);
        status.buffer_fill_pct = 100.0;
        let payload = json!({
            "state":status.state,
            "acq_count":status.acq_count,
            "samples":status.samples,
            "recurring":status.recurring
        });
        drop(status);
        self.emit("status", payload);
        Ok(())
    }
    pub(crate) fn start_recurring(&self, max_runs: Option<u64>) -> Result<(), ApiError> {
        if self
            .inner
            .recurring
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(api_error(
                "ACQ_BUSY",
                "recurring acquisition is already running",
            ));
        }
        {
            let mut status = self
                .inner
                .acquisition_status
                .write()
                .map_err(|_| api_error("INTERNAL", "acquisition status lock poisoned"))?;
            status.state = "prefill";
            status.recurring = true;
        }
        let state = self.clone();
        std::thread::spawn(move || state.recurring_loop(max_runs));
        Ok(())
    }
    fn recurring_loop(self, max_runs: Option<u64>) {
        let mut runs = 0_u64;
        while self.inner.recurring.load(Ordering::Acquire)
            && max_runs.is_none_or(|limit| runs < limit)
        {
            let result = self.acquire_single();
            match result {
                Ok(capture) => {
                    let samples = capture.expanded_len();
                    if self.insert_capture(capture).is_err()
                        || self.set_capture_ready(samples).is_err()
                    {
                        self.set_acquisition_error("failed to store recurring capture");
                        break;
                    }
                    runs = runs.saturating_add(1);
                }
                Err(error) => {
                    self.set_acquisition_error(&error.to_string());
                    break;
                }
            }
            if self.inner.recurring.load(Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let stopped = !self.inner.recurring.swap(false, Ordering::AcqRel);
        if let Ok(mut status) = self.inner.acquisition_status.write() {
            status.recurring = false;
            if status.state != "error" {
                status.state = if stopped { "halted" } else { "ready" };
            }
        }
    }
    fn set_acquisition_error(&self, message: &str) {
        if let Ok(mut status) = self.inner.acquisition_status.write() {
            status.state = "error";
            status.recurring = false;
            status.warnings.push(message.to_owned());
        }
        self.inner.recurring.store(false, Ordering::Release);
    }
    pub(crate) fn stop_recurring(&self) -> bool {
        self.inner.recurring.swap(false, Ordering::AcqRel)
    }
    pub(crate) fn is_recurring(&self) -> bool {
        self.inner.recurring.load(Ordering::Acquire)
    }
    pub(crate) fn setup(&self) -> Result<lp_project::Settings, ApiError> {
        self.inner
            .project
            .read()
            .map(|project| project.settings.clone())
            .map_err(|_| api_error("INTERNAL", "project lock poisoned"))
    }
    pub(crate) fn apply_setup(&self, settings: lp_project::Settings) -> Result<bool, ApiError> {
        if self.is_recurring() {
            return Err(api_error("ACQ_BUSY", "cannot change setup while acquiring"));
        }
        self.validate_setup(&settings)?;
        let reconfigured = self
            .inner
            .acquisition
            .lock()
            .map_err(|_| api_error("INTERNAL", "acquisition lock poisoned"))?
            .apply_setup(&settings)
            .map_err(crate::acquisitions::acquisition_error)?;
        self.inner
            .project
            .write()
            .map_err(|_| api_error("INTERNAL", "project lock poisoned"))?
            .settings = settings;
        self.emit("setup", json!({"setup":self.setup()?}));
        Ok(reconfigured)
    }
    pub(crate) fn validate_setup(&self, settings: &lp_project::Settings) -> Result<(), ApiError> {
        crate::acquisitions::validate_setup_settings(settings)
            .map_err(crate::acquisitions::acquisition_error)?;
        let mut candidate = self
            .inner
            .project
            .read()
            .map_err(|_| api_error("INTERNAL", "project lock poisoned"))?
            .clone();
        candidate.settings = settings.clone();
        candidate
            .validate()
            .map_err(|error| api_error("INVALID_ARG", error.to_string()))
    }
    pub(crate) fn emit(&self, kind: &'static str, data: Value) {
        let seq = self.inner.next_event_seq.fetch_add(1, Ordering::Relaxed);
        let event = crate::events::Event::new(seq, kind, data);
        if let Ok(mut events) = self.inner.events.lock() {
            if events.len() == 1000 {
                events.pop_front();
            }
            events.push_back(event.clone());
        }
        let _ = self.inner.event_tx.send(event);
    }
    pub(crate) fn events_since(
        &self,
        since_seq: u64,
        limit: usize,
    ) -> Result<Vec<crate::events::Event>, ApiError> {
        self.inner
            .events
            .lock()
            .map(|events| {
                events
                    .iter()
                    .filter(|event| event.seq > since_seq)
                    .take(limit)
                    .cloned()
                    .collect()
            })
            .map_err(|_| api_error("INTERNAL", "event ring lock poisoned"))
    }
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<crate::events::Event> {
        self.inner.event_tx.subscribe()
    }
}
impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run_mcp_stdio(state: &AppState) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    lp_mcp::server::run_stdio(state.inner.as_ref(), stdin.lock(), stdout.lock())
}

pub fn router(state: AppState) -> Router {
    let web_root = std::env::var("LP_WEB").unwrap_or_else(|_| "web/dist".to_owned());
    let index = std::path::Path::new(&web_root).join("index.html");
    let web = tower_http::services::ServeDir::new(web_root)
        .append_index_html_on_directories(true)
        .not_found_service(tower_http::services::ServeFile::new(index));
    Router::new()
        .route("/api/health", get(health))
        .route("/api/ops", get(op_list))
        .route("/api/ops/{id}/schema", get(op_schema))
        .route("/api/ops/{id}", post(op_call))
        .route("/api/project", get(project_get).put(project_put))
        .route("/mcp", get(mcp_get).post(mcp_post))
        .merge(crate::acquisitions::routes())
        .merge(crate::captures::routes())
        .merge(crate::setup::routes())
        .merge(crate::events::routes())
        .fallback_service(web)
        .with_state(state)
}
async fn mcp_post(State(state): State<AppState>, Json(request): Json<Value>) -> Json<Value> {
    Json(state.inner.mcp.handle(state.inner.as_ref(), request))
}
async fn mcp_get() -> Response {
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/event-stream"),
            (axum::http::header::CACHE_CONTROL, "no-cache"),
        ],
        ": logicport MCP event stream\nretry: 1000\n\n",
    )
        .into_response()
}
async fn health(State(state): State<AppState>) -> Json<Value> {
    let connected = state.inner.device_connected.load(Ordering::Acquire);
    let needs_replug = state.inner.device_replug_required.load(Ordering::Acquire);
    let device = if connected {
        "connected"
    } else if needs_replug {
        "needs_replug"
    } else {
        "disconnected"
    };
    let mut body = json!({
        "ok":true,
        "version":env!("CARGO_PKG_VERSION"),
        "device":device,
        "needs_replug":needs_replug,
    });
    if needs_replug {
        body["hint"] = json!(REPLUG_HINT);
    }
    Json(body)
}
async fn op_list() -> Json<&'static [OpSpec]> {
    Json(ops::registry())
}
async fn op_schema(Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let op = ops::find(&id)
        .ok_or_else(|| api_error("UNKNOWN_OP", format!("unknown operation: {id}")))?;
    Ok(Json(
        json!({"id":op.id,"params":op.params,"result":op.result}),
    ))
}
async fn op_call(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(params): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    ops::dispatch(state.inner.as_ref(), &id, params)
        .map(Json)
        .map_err(ApiError)
}
async fn project_get(State(state): State<AppState>) -> Result<Json<Project>, ApiError> {
    state
        .inner
        .project
        .read()
        .map(|v| Json(v.clone()))
        .map_err(|_| api_error("INTERNAL", "project lock poisoned"))
}
async fn project_put(
    State(state): State<AppState>,
    Json(project): Json<Project>,
) -> Result<Json<Project>, ApiError> {
    project
        .validate()
        .map_err(|e| api_error("INVALID_ARG", e.to_string()))?;
    *state
        .inner
        .project
        .write()
        .map_err(|_| api_error("INTERNAL", "project lock poisoned"))? = project.clone();
    Ok(Json(project))
}

impl Dispatcher for Context {
    fn call(&self, op: &OpSpec, params: Value) -> Result<Value, ToolError> {
        match op.id.as_str() {
            "meta.ops_list" => serde_json::to_value(ops::registry()).map_err(json_error),
            "file.new" => self.file_new(),
            "file.open" => self.file_open(&params),
            "file.save" => self.file_save(None),
            "file.save_as" => {
                let path = required_path(&params)?;
                self.file_save(Some(path))
            }
            "file.recent.list" => self.file_recent_list(),
            "file.recent.open" => self.file_recent_open(&params),
            "file.save_on_exit.set" => {
                let mut settings = self.current_settings()?;
                settings.options.save_on_exit = required_bool(&params, &["enabled", "value"])?;
                self.apply_project_settings_only(settings.clone())?;
                Ok(json!({"save_on_exit":settings.options.save_on_exit}))
            }
            "file.close" => self.file_close(),
            "file.readonly.get" => self.file_readonly(),
            "file.exit" => {
                self.exit_requested.store(true, Ordering::Release);
                Ok(json!({"exit_requested":true,"daemon_stopped":false}))
            }
            "project.get" => self
                .project
                .read()
                .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))
                .and_then(|p| serde_json::to_value(&*p).map_err(json_error)),
            "project.put" => {
                let project: Project = serde_json::from_value(params)
                    .map_err(|e| tool_error("INVALID_ARG", e.to_string()))?;
                project
                    .validate()
                    .map_err(|e| tool_error("INVALID_ARG", e.to_string()))?;
                *self
                    .project
                    .write()
                    .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? =
                    project.clone();
                serde_json::to_value(project).map_err(json_error)
            }
            "project.import_lpf" => {
                let path = params
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| tool_error("INVALID_ARG", "path must be a string"))?;
                let document = lp_lpf::load(path)
                    .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
                let mut project = document
                    .to_project("1970-01-01T00:00:00Z", Some(path.to_owned()))
                    .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
                self.captures.clear().map_err(store_error)?;
                if let Some(capture) = project.capture.take() {
                    project.capture = Some(self.captures.insert(capture).map_err(store_error)?);
                }
                *self
                    .project
                    .write()
                    .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? =
                    project.clone();
                serde_json::to_value(project).map_err(json_error)
            }
            "signals.list" | "signals.dialog.open" => self.project_snapshot().map(|project| {
                json!({
                    "open":op.id == "signals.dialog.open",
                    "signals":project.signals
                })
            }),
            "signals.rename" | "signals.rename_all" | "signals.dialog.ok" => {
                self.mutate_signals(op.id.as_str(), &params)
            }
            "groups.list" | "groups.select.dialog.open" => self.project_snapshot().map(|project| {
                json!({
                    "open":op.id == "groups.select.dialog.open",
                    "groups":project.groups
                })
            }),
            "groups.dialog.open" => self.group_dialog(&params),
            "groups.validate" => {
                let group: lp_project::Group =
                    serde_json::from_value(params.get("group").cloned().unwrap_or(params))
                        .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
                validate_group(&group)?;
                Ok(json!({"valid":true,"group":group}))
            }
            id @ ("groups.create"
            | "groups.edit"
            | "groups.copy"
            | "groups.delete"
            | "groups.rename"
            | "groups.members.add"
            | "groups.members.remove"
            | "groups.reverse_display_order"
            | "groups.dialog.ok") => self.mutate_groups(id, &params),
            "rows.list" => self
                .project_snapshot()
                .map(|project| json!({"rows":project.rows})),
            id @ ("rows.add.signal"
            | "rows.add.group"
            | "rows.add.interpreter"
            | "rows.insert.signal"
            | "rows.insert.group"
            | "rows.insert.interpreter"
            | "rows.remove"
            | "rows.remove.signal"
            | "rows.remove.group"
            | "rows.remove.interpreter"
            | "rows.remove_all"
            | "rows.add_all"
            | "rows.reorder"
            | "rows.expand"
            | "rows.collapse"
            | "rows.expand_all"
            | "rows.collapse_all"
            | "rows.toggle_expand"
            | "rows.height.set"
            | "row.style.set"
            | "row.color.set"
            | "row.color.default"
            | "group.radix.set"
            | "group.signed.set"
            | "group.wire_order.set"
            | "group.display_order.set") => self.mutate_rows(id, &params),
            "row.hover_value" => self.row_value(&params),
            "group.value_at" => self.group_value(&params),
            "interp.list" => self
                .project_snapshot()
                .map(|project| json!({ "interpreters": project.interpreters })),
            "interp.frames" => self.interpreter_frames(&params),
            "project.notes" | "notes.get" | "notes.open" => {
                self.notes_snapshot(op.id == "notes.open")
            }
            "notes.set" => self.set_notes(&params),
            "device.status" => {
                let connected = self.device_connected.load(Ordering::Acquire);
                let error = self
                    .device_error
                    .read()
                    .map_err(|_| tool_error("INTERNAL", "device error lock poisoned"))?
                    .clone();
                Ok(json!({
                    "state":if connected { "connected" } else { "disconnected" },
                    "backend":self.device_kind,
                    "fpga":if connected { "configured" } else { "unavailable" },
                    "fpga_image":if connected { Some(7) } else { None },
                    "fpga_image_id":if connected { Some(23) } else { None },
                    "device_epoch":self.device_epoch.load(Ordering::Acquire),
                    "usb_error_count":0,
                    "error":error
                }))
            }
            "sample.get" => self
                .project
                .read()
                .map(|project| json!({"sample":project.settings.sample}))
                .map_err(|_| tool_error("INTERNAL", "project lock poisoned")),
            id @ ("sample.mode.set"
            | "sample.rate.set"
            | "sample.rate.step_up"
            | "sample.rate.step_down"
            | "sample.rate.units.set"
            | "sample.state.clock.set"
            | "sample.state.edge.set"
            | "sample.state.window.set"
            | "sample.state.qualifier.enable"
            | "sample.state.qualifier.polarity"
            | "sample.state.declared_rate.set"
            | "sample.state.declared_units.set"
            | "sample.compression.set"
            | "sample.prefill_timeout.set"
            | "sample.postfill_timeout.set"
            | "sample.pretrigger_buffer.set") => self.mutate_sample(id, &params),
            "sample.validate" => {
                let settings = self.settings_from_params(params)?;
                crate::acquisitions::validate_setup_settings(&settings)
                    .map_err(crate::acquisitions::acquisition_tool_error)?;
                Ok(json!({"valid":true,"setup":settings}))
            }
            "sample.apply" => {
                let settings = self.settings_from_params(params)?;
                let reconfigured = self.apply_settings_from_dispatch(settings.clone())?;
                Ok(json!({"setup":settings,"warnings":[],"hardware_reconfigure":reconfigured}))
            }
            "sample.dialog.open" => self
                .project
                .read()
                .map(|project| json!({"open":true,"sample":project.settings.sample}))
                .map_err(|_| tool_error("INTERNAL", "project lock poisoned")),
            "sample.dialog.ok" | "sample.dialog.apply" => {
                let settings = self.settings_from_params(params)?;
                let reconfigured = self.apply_settings_from_dispatch(settings.clone())?;
                Ok(json!({
                    "open":op.id == "sample.dialog.apply",
                    "sample":settings.sample,
                    "hardware_reconfigure":reconfigured
                }))
            }
            "threshold.set" | "threshold.step_up" | "threshold.step_down" => {
                let mut settings = self.current_settings()?;
                let threshold = if op.id == "threshold.set" {
                    required_f64(&params, &["threshold_v", "volts", "value"])?
                } else if op.id == "threshold.step_up" {
                    settings.threshold_v + 0.05
                } else {
                    settings.threshold_v - 0.05
                };
                if !(-6.0..=6.0).contains(&threshold) {
                    return Err(tool_error(
                        "INVALID_ARG",
                        "threshold_v must be between -6 V and +6 V",
                    ));
                }
                settings.threshold_v = (threshold * 20.0).round() / 20.0;
                let reconfigured = self.apply_settings_from_dispatch(settings.clone())?;
                Ok(json!({"threshold_v":settings.threshold_v,"hardware_reconfigure":reconfigured}))
            }
            "logicsense.get" | "logicsense.dialog.open" => {
                self.current_settings().map(|settings| {
                    json!({
                        "open":op.id == "logicsense.dialog.open",
                        "inverted":settings.logic_sense.inverted
                    })
                })
            }
            "logicsense.set" => {
                let mut settings = self.current_settings()?;
                let channel = required_u8(&params, &["channel", "wire"])?;
                let inverted = required_bool(&params, &["inverted", "enabled"])?;
                let target = settings
                    .logic_sense
                    .inverted
                    .get_mut(usize::from(channel))
                    .ok_or_else(|| tool_error("INVALID_ARG", "channel must be in 0..33"))?;
                *target = inverted;
                let reconfigured = self.apply_settings_from_dispatch(settings.clone())?;
                Ok(
                    json!({"channel":channel,"inverted":inverted,"hardware_reconfigure":reconfigured}),
                )
            }
            "logicsense.set_all" => {
                let mut settings = self.current_settings()?;
                let inverted = required_bool(&params, &["inverted", "enabled"])?;
                settings.logic_sense.inverted.fill(inverted);
                let reconfigured = self.apply_settings_from_dispatch(settings.clone())?;
                Ok(
                    json!({"inverted":settings.logic_sense.inverted,"hardware_reconfigure":reconfigured}),
                )
            }
            "logicsense.dialog.ok" => {
                let mut settings = self.current_settings()?;
                let values = params
                    .get("inverted")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        tool_error("INVALID_ARG", "inverted must be a 34-element boolean array")
                    })?;
                if values.len() != 34 {
                    return Err(tool_error(
                        "INVALID_ARG",
                        "inverted must contain exactly 34 values",
                    ));
                }
                settings.logic_sense.inverted = values
                    .iter()
                    .enumerate()
                    .map(|(channel, value)| {
                        value.as_bool().ok_or_else(|| {
                            tool_error(
                                "INVALID_ARG",
                                format!("inverted[{channel}] must be boolean"),
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let reconfigured = self.apply_settings_from_dispatch(settings.clone())?;
                Ok(
                    json!({"open":false,"inverted":settings.logic_sense.inverted,"hardware_reconfigure":reconfigured}),
                )
            }
            "options.optimization.set"
            | "options.save_on_exit.set"
            | "options.extended_rates.set" => {
                let mut settings = self.current_settings()?;
                match op.id.as_str() {
                    "options.optimization.set" => {
                        let value = required_str(&params, &["optimization", "value"])?;
                        if value.is_empty() {
                            return Err(tool_error(
                                "INVALID_ARG",
                                "optimization must not be empty",
                            ));
                        }
                        settings.options.optimization = value.to_owned();
                    }
                    "options.save_on_exit.set" => {
                        settings.options.save_on_exit =
                            required_bool(&params, &["save_on_exit", "enabled"])?;
                    }
                    _ => {
                        settings.options.extended_rates =
                            required_bool(&params, &["extended_rates", "enabled"])?;
                    }
                }
                self.apply_project_settings_only(settings.clone())?;
                Ok(json!({"options":settings.options,"hardware_reconfigure":false}))
            }
            "options.keep_on_top.set" => {
                let enabled = required_bool(&params, &["keep_on_top", "enabled"])?;
                Ok(json!({
                    "accepted":true,
                    "enabled":enabled,
                    "effective":false,
                    "reason":"browser_window_control_unavailable"
                }))
            }
            "acq.single" => {
                if self.recurring.load(Ordering::Acquire) {
                    return Err(tool_error("ACQ_BUSY", "recurring acquisition is running"));
                }
                if params
                    .get("clear_before")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    self.captures.clear().map_err(store_error)?;
                }
                self.acquisition_status
                    .write()
                    .map_err(|_| tool_error("INTERNAL", "acquisition status lock poisoned"))?
                    .state = "readback";
                let capture = self
                    .acquisition
                    .lock()
                    .map_err(|_| tool_error("INTERNAL", "acquisition lock poisoned"))?
                    .single()
                    .map_err(crate::acquisitions::acquisition_tool_error)?;
                let capture = self.captures.insert(capture).map_err(store_error)?;
                let samples = capture.expanded_len();
                let mut status = self
                    .acquisition_status
                    .write()
                    .map_err(|_| tool_error("INTERNAL", "acquisition status lock poisoned"))?;
                status.state = "ready";
                status.samples = samples;
                status.acq_count = status.acq_count.saturating_add(1);
                status.buffer_fill_pct = 100.0;
                Ok(json!({"capture":capture,"expanded_len":samples}))
            }
            "acq.status" => self
                .acquisition_status
                .read()
                .map_err(|_| tool_error("INTERNAL", "acquisition status lock poisoned"))
                .and_then(|status| serde_json::to_value(&*status).map_err(json_error)),
            "acq.recurring.start" => {
                let max_runs = optional_u64(&params, &["max_runs"])?;
                if max_runs == Some(0) {
                    return Err(tool_error("INVALID_ARG", "max_runs must be positive"));
                }
                self.start_recurring_from_dispatch(max_runs)?;
                Ok(json!({"recurring":true,"max_runs":max_runs}))
            }
            "acq.recurring.stop" => {
                let was_running = self.recurring.swap(false, Ordering::AcqRel);
                let mut status = self
                    .acquisition_status
                    .write()
                    .map_err(|_| tool_error("INTERNAL", "acquisition status lock poisoned"))?;
                status.recurring = false;
                status.state = "halted";
                Ok(json!({"stopped":was_running}))
            }
            "acq.wait" => {
                let timeout = Duration::from_millis(
                    optional_u64(&params, &["timeout_ms"])?
                        .unwrap_or(30_000)
                        .min(300_000),
                );
                let target = first_param(&params, &["for", "target"])
                    .map(|value| {
                        value
                            .as_str()
                            .ok_or_else(|| tool_error("INVALID_ARG", "for must be a string"))
                    })
                    .transpose()?
                    .unwrap_or("ready");
                self.wait_for_acquisition(target, timeout)
            }
            "acq.halt" => {
                self.recurring.store(false, Ordering::Release);
                let phase = self
                    .acquisition
                    .lock()
                    .map_err(|_| tool_error("INTERNAL", "acquisition lock poisoned"))?
                    .halt()
                    .map_err(crate::acquisitions::acquisition_tool_error)?;
                self.acquisition_status
                    .write()
                    .map_err(|_| tool_error("INTERNAL", "acquisition status lock poisoned"))?
                    .state = "halted";
                Ok(json!({"phase":format!("{phase:?}"),"state":"halted"}))
            }
            "acq.trigger_immediate" => {
                let phase = self
                    .acquisition
                    .lock()
                    .map_err(|_| tool_error("INTERNAL", "acquisition lock poisoned"))?
                    .immediate()
                    .map_err(crate::acquisitions::acquisition_tool_error)?;
                Ok(json!({"phase_after":format!("{phase:?}"),"strobe":true}))
            }
            "status.phase.get" => self.status_snapshot().map(|status| {
                json!({
                    "phase":status.state,
                    "hw_status_byte":status.hw_status_byte,
                    "recurring":status.recurring
                })
            }),
            "status.stats.get" => {
                let status = self.status_snapshot()?;
                let settings = self.current_settings()?;
                Ok(json!({
                    "acq_count":status.acq_count,
                    "samples":status.samples,
                    "usb_error_count":settings.usb_error_count
                }))
            }
            "status.buffer_indicator.get" => self.status_snapshot().map(|status| {
                json!({
                    "buffer_fill_pct":status.buffer_fill_pct,
                    "samples":status.samples
                })
            }),
            "status.warnings.get" => self
                .status_snapshot()
                .map(|status| json!({"warnings":status.warnings})),
            "status.measurements.get" => self.measurement_status(),
            "status.get" => {
                let status = self.status_snapshot()?;
                let settings = self.current_settings()?;
                let measurements = self.measurement_status()?;
                Ok(json!({
                    "phase":status.state,
                    "hw_status_byte":status.hw_status_byte,
                    "acq_count":status.acq_count,
                    "samples":status.samples,
                    "buffer_fill_pct":status.buffer_fill_pct,
                    "recurring":status.recurring,
                    "warnings":status.warnings,
                    "usb_error_count":settings.usb_error_count,
                    "measurements":measurements["measurements"]
                }))
            }
            "export.format.set" | "export.radix.set" | "export.target.set" => {
                let mut settings = self.current_settings()?;
                if op.id == "export.format.set" {
                    settings.export.format =
                        validate_export_format(required_str(&params, &["format", "value"])?)?
                            .to_owned();
                } else if op.id == "export.radix.set" {
                    settings.export.radix =
                        validate_radix(required_str(&params, &["radix", "value"])?)?.to_owned();
                } else {
                    settings.export.target_path =
                        optional_string(&params, &["target_path", "path"])?;
                }
                self.apply_project_settings_only(settings.clone())?;
                Ok(json!({"export":settings.export}))
            }
            "export.dialog.open" => self
                .current_settings()
                .map(|settings| json!({"open":true,"export":settings.export})),
            "export.dialog.ok" => {
                let mut settings = self.current_settings()?;
                if let Some(value) = first_param(&params, &["format"]) {
                    settings.export.format =
                        validate_export_format(value.as_str().ok_or_else(|| {
                            tool_error("INVALID_ARG", "format must be a string")
                        })?)?
                        .to_owned();
                }
                if let Some(value) = first_param(&params, &["radix"]) {
                    settings.export.radix = validate_radix(
                        value
                            .as_str()
                            .ok_or_else(|| tool_error("INVALID_ARG", "radix must be a string"))?,
                    )?
                    .to_owned();
                }
                if first_param(&params, &["target_path", "path"]).is_some() {
                    settings.export.target_path =
                        optional_string(&params, &["target_path", "path"])?;
                }
                self.apply_project_settings_only(settings.clone())?;
                Ok(json!({"open":false,"export":settings.export}))
            }
            "export.run" | "export.vcd" | "export.txt" | "export.json" => {
                let capture = self.capture_param(&params, "capture_id")?;
                let settings = self.current_settings()?;
                let kind = match op.id.as_str() {
                    "export.vcd" => "vcd",
                    "export.txt" => "txt",
                    "export.json" => "json",
                    _ => settings.export.format.as_str(),
                };
                export_payload(&capture, kind, settings.export.target_path)
            }
            "view.get" => self.project_snapshot().map(|project| {
                json!({
                    "view":project.view,
                    "options":project.settings.options,
                    "cursors":project.cursors
                })
            }),
            id @ ("view.set"
            | "view.graticule.toggle"
            | "view.show_trigger.toggle"
            | "view.show_cursors.set"
            | "view.show_cursors.all"
            | "view.show_cursors.none"
            | "view.cursor_qty.set"
            | "view.color_scheme.set"
            | "view.alt_background.enable"
            | "view.alt_background.adjust"
            | "view.waveforms_in_front.toggle"
            | "view.large_waveforms.toggle"
            | "view.sample_reference.set"
            | "view.reference_position.set"
            | "view.scale_relative.set"
            | "view.units.set"
            | "view.scale_factor.set"
            | "view.reference_offset.set"
            | "view.panel.waveforms"
            | "view.panel.statelist"
            | "view.panel.notes"
            | "view.theme.set"
            | "view.control_rows.set") => self.mutate_view(id, &params),
            id @ ("view.zoom.in"
            | "view.zoom.out"
            | "view.zoom.all"
            | "view.zoom.at"
            | "view.zoom.out_at"
            | "view.scroll.by"
            | "view.scroll.drag"
            | "view.scroll.large"
            | "view.scroll.small"
            | "view.scroll.key_left"
            | "view.scroll.key_right"
            | "view.scroll_to.begin"
            | "view.scroll_to.trigger"
            | "view.scroll_to.end"
            | "view.scroll_to.cursor.a"
            | "view.scroll_to.cursor.b"
            | "view.scroll_to.cursor.c"
            | "view.scroll_to.cursor.d"
            | "view.scroll_to.cursor.e"
            | "view.scroll_to.cursor.f"
            | "view.next_edge"
            | "view.prev_edge"
            | "view.next_edge.row"
            | "view.prev_edge.row") => self.navigate_view(id, &params),
            "cursor.get" => self.cursor_snapshot(),
            id @ ("cursor.place.a"
            | "cursor.place.b"
            | "cursor.place.c"
            | "cursor.place.d"
            | "cursor.place.e"
            | "cursor.place.f"
            | "cursor.place_all"
            | "cursor.drag"
            | "cursor.set"
            | "cursor.snap.toggle"
            | "cursor.tracking.set.a"
            | "cursor.tracking.set.b"
            | "cursor.tracking.set.c"
            | "cursor.tracking.set.d"
            | "cursor.tracking.set.e"
            | "cursor.tracking.set.f"
            | "cursor.tracking.interlock_all"
            | "cursor.tracking.release_all") => self.mutate_cursors(id, &params),
            "measure.get" => self.measurement_status(),
            "measure.compute" => self.compute_measurement(&params),
            "measure.dialog.open" => self.measurement_dialog(),
            "measure.panel.click" => self.measurement_panel_click(&params),
            id @ ("measure.slot.type.set"
            | "measure.slot.left.set"
            | "measure.slot.right.set"
            | "measure.slot.source.set"
            | "measure.dialog.ok") => self.mutate_measurements(id, &params),
            "statelist.get" => self.statelist_get(&params),
            id @ ("statelist.format.set"
            | "statelist.relative.set"
            | "statelist.column.reorder"
            | "statelist.column.format.set"
            | "statelist.scroll.page_up"
            | "statelist.scroll.page_down"
            | "statelist.scroll.key"
            | "statelist.scroll.drag"
            | "statelist.place_cursor") => self.mutate_statelist(id, &params),
            "help.contents" => Ok(help_contents()),
            "help.language" => Ok(json!({"current":"en","supported":["en"]})),
            "help.website" => Ok(json!({
                "url":"https://www.pctestinstruments.com/",
                "opened":false
            })),
            "help.about" => Ok(json!({
                "name":"LogicPort for Linux",
                "version":env!("CARGO_PKG_VERSION"),
                "license":"private interoperability implementation",
                "device":"Intronix LogicPort LA1034"
            })),
            "help.shortcuts" => Ok(help_shortcuts()),
            "capture.list" => self
                .captures
                .list(param_usize(&params, "limit", 50, 200)?)
                .map_err(|e| tool_error("INTERNAL", e.to_string()))
                .and_then(|captures| {
                    captures
                        .into_iter()
                        .map(|capture| self.capture_envelope(capture))
                        .collect::<Result<Vec<_>, _>>()
                        .map(|captures| json!({"captures":captures}))
                }),
            "capture.get" => self
                .capture_param(&params, "capture_id")
                .and_then(|capture| self.capture_envelope(capture)),
            "capture.summary" => self
                .capture_param(&params, "capture_id")
                .and_then(|capture| lp_project::summarize(&capture).map_err(capture_error))
                .and_then(|summary| serde_json::to_value(summary).map_err(json_error)),
            "capture.export" => {
                let capture = self.capture_param(&params, "capture_id")?;
                let kind = param_str(&params, "kind").unwrap_or("csv");
                export_payload(&capture, kind, None)
            }
            "capture.search" => {
                let capture = self.capture_param(&params, "capture_id")?;
                let input = param_str(&params, "q")
                    .or_else(|| param_str(&params, "query"))
                    .ok_or_else(|| tool_error("INVALID_ARG", "q or query must be a string"))?;
                let query = lp_project::search::parse(input)
                    .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
                let matches = lp_project::search::execute(
                    &capture,
                    &query,
                    &lp_project::search::Bindings::default(),
                    param_usize(&params, "limit", 100, 1000)?,
                )
                .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
                Ok(json!({
                    "capture_id":capture.id,
                    "matches":matches.into_iter().map(|item| json!({"start":item.start,"end":item.end})).collect::<Vec<_>>()
                }))
            }
            "capture.diff" => {
                let a = self.capture_param(&params, "a")?;
                let b = self.capture_param(&params, "b")?;
                let channels = params
                    .get("channels")
                    .and_then(Value::as_u64)
                    .unwrap_or(lp_project::capture::CHANNEL_MASK);
                serde_json::to_value(lp_project::diff(&a, &b, channels)).map_err(json_error)
            }
            "capture.measure" => {
                let capture = self.capture_param(&params, "capture_id")?;
                let kind: MeasurementKind = serde_json::from_value(
                    params
                        .get("type")
                        .cloned()
                        .ok_or_else(|| tool_error("INVALID_ARG", "type is required"))?,
                )
                .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
                let channel = parse_channel(params.get("source"))?;
                let left = params.get("left").and_then(Value::as_u64).unwrap_or(0);
                let right = params
                    .get("right")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| capture.expanded_len());
                lp_project::measure(&capture, kind, channel, left, right)
                    .map_err(capture_error)
                    .and_then(|measurement| serde_json::to_value(measurement).map_err(json_error))
            }
            "capture.state_list" => {
                let capture = self.capture_param(&params, "capture_id")?;
                let offset = param_usize(&params, "offset", 0, usize::MAX)?;
                let limit = param_usize(&params, "limit", 200, 2000)?;
                let mut sample = 0_u64;
                let rows = capture.runs.iter().enumerate().filter_map(|(index, run)| {
                    let start = sample;
                    sample = sample.saturating_add(run.count);
                    (index >= offset && index < offset.saturating_add(limit)).then(|| json!({
                        "index":index,"sample":start,"time_s":start as f64 * capture.sample_period_s,
                        "value":run.data,"count":run.count
                    }))
                }).collect::<Vec<_>>();
                let next_cursor = (offset + rows.len() < capture.runs.len())
                    .then(|| format!("{}:state:{}", capture.id, offset + rows.len()));
                Ok(json!({"capture_id":capture.id,"rows":rows,"next_cursor":next_cursor}))
            }
            "capture.pin" => {
                let capture = self.capture_param(&params, "capture_id")?;
                let pinned = params
                    .get("pinned")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                self.captures.pin(capture.id, pinned).map_err(store_error)?;
                Ok(json!({"capture_id":capture.id,"pinned":pinned}))
            }
            "capture.delete" => {
                let capture = self.capture_param(&params, "capture_id")?;
                self.captures.remove(capture.id).map_err(store_error)?;
                Ok(json!({"capture_id":capture.id,"deleted":true}))
            }
            _ => Err(tool_error(
                "NOT_SUPPORTED",
                format!("operation {} is registered but not implemented", op.id),
            )),
        }
    }
}

impl Context {
    fn file_new(&self) -> Result<Value, ToolError> {
        let project = Project::new("1970-01-01T00:00:00Z");
        self.captures.clear().map_err(store_error)?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        *self
            .project_path
            .write()
            .map_err(|_| tool_error("INTERNAL", "project path lock poisoned"))? = None;
        Ok(json!({"project":project,"path":Value::Null,"created":true}))
    }

    fn file_open(&self, params: &Value) -> Result<Value, ToolError> {
        let path = required_path(params)?;
        self.open_project_path(path)
    }

    fn open_project_path(&self, path: PathBuf) -> Result<Value, ToolError> {
        let is_lpf = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lpf"));
        let mut project = if is_lpf {
            lp_lpf::load(&path)
                .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?
                .to_project(
                    "1970-01-01T00:00:00Z",
                    Some(path.to_string_lossy().into_owned()),
                )
                .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?
        } else {
            Project::load(&path).map_err(|error| tool_error("INVALID_ARG", error.to_string()))?
        };
        project
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        self.captures.clear().map_err(store_error)?;
        if let Some(capture) = project.capture.take() {
            project.capture = Some(self.captures.insert(capture).map_err(store_error)?);
        }
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        *self
            .project_path
            .write()
            .map_err(|_| tool_error("INTERNAL", "project path lock poisoned"))? =
            (!is_lpf).then_some(path.clone());
        self.remember_project(path.clone())?;
        Ok(json!({
            "project":project,
            "path":path,
            "imported":is_lpf,
            "save_requires_path":is_lpf
        }))
    }

    fn file_save(&self, requested_path: Option<PathBuf>) -> Result<Value, ToolError> {
        let path = match requested_path {
            Some(path) => path,
            None => self
                .project_path
                .read()
                .map_err(|_| tool_error("INTERNAL", "project path lock poisoned"))?
                .clone()
                .ok_or_else(|| {
                    tool_error(
                        "PATH_REQUIRED",
                        "project has no native path; use file.save_as",
                    )
                })?,
        };
        validate_native_project_path(&path)?;
        let mut project = self.project_snapshot()?;
        if project.read_only {
            return Err(tool_error("READ_ONLY", "project is read-only"));
        }
        project.source.kind = lp_project::SourceKind::Native;
        project.source.path = Some(path.to_string_lossy().into_owned());
        project
            .save_atomic(&path)
            .map_err(|error| tool_error("IO_ERROR", error.to_string()))?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project;
        *self
            .project_path
            .write()
            .map_err(|_| tool_error("INTERNAL", "project path lock poisoned"))? =
            Some(path.clone());
        self.remember_project(path.clone())?;
        Ok(json!({"saved":true,"path":path}))
    }

    fn file_recent_list(&self) -> Result<Value, ToolError> {
        let recent = self
            .recent_projects
            .lock()
            .map_err(|_| tool_error("INTERNAL", "recent project lock poisoned"))?
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        Ok(json!({"recent":recent}))
    }

    fn file_recent_open(&self, params: &Value) -> Result<Value, ToolError> {
        let path = if let Some(value) = params.get("path") {
            value
                .as_str()
                .map(PathBuf::from)
                .ok_or_else(|| tool_error("INVALID_ARG", "path must be a string"))?
        } else {
            let index = param_usize(params, "index", 0, 9)?;
            self.recent_projects
                .lock()
                .map_err(|_| tool_error("INTERNAL", "recent project lock poisoned"))?
                .get(index)
                .cloned()
                .ok_or_else(|| tool_error("INVALID_ARG", "recent project index is absent"))?
        };
        self.open_project_path(path)
    }

    fn file_close(&self) -> Result<Value, ToolError> {
        let path = self
            .project_path
            .read()
            .map_err(|_| tool_error("INTERNAL", "project path lock poisoned"))?
            .clone();
        self.file_new()?;
        Ok(json!({"closed":true,"previous_path":path}))
    }

    fn file_readonly(&self) -> Result<Value, ToolError> {
        let project = self.project_snapshot()?;
        let path = self
            .project_path
            .read()
            .map_err(|_| tool_error("INTERNAL", "project path lock poisoned"))?
            .clone();
        Ok(json!({"read_only":project.read_only,"path":path}))
    }

    fn remember_project(&self, path: PathBuf) -> Result<(), ToolError> {
        let mut recent = self
            .recent_projects
            .lock()
            .map_err(|_| tool_error("INTERNAL", "recent project lock poisoned"))?;
        recent.retain(|existing| existing != &path);
        recent.push_front(path);
        recent.truncate(10);
        Ok(())
    }

    fn mutate_signals(&self, id: &str, params: &Value) -> Result<Value, ToolError> {
        let mut project = self.project_snapshot()?;
        if id == "signals.rename" {
            let wire = required_u8(params, &["wire", "channel"])?;
            let name = required_nonempty_str(params, &["name"])?;
            let signal = project
                .signals
                .get_mut(usize::from(wire))
                .filter(|signal| signal.wire == wire)
                .ok_or_else(|| tool_error("INVALID_ARG", "wire must be in 0..33"))?;
            signal.name = name.to_owned();
        } else {
            let names = if let Some(names) = params.get("names").and_then(Value::as_array) {
                names
                    .iter()
                    .enumerate()
                    .map(|(wire, value)| {
                        value
                            .as_str()
                            .filter(|name| !name.is_empty())
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                tool_error(
                                    "INVALID_ARG",
                                    format!("names[{wire}] must be a non-empty string"),
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else if let Some(signals) = params.get("signals") {
                let signals: Vec<lp_project::Signal> = serde_json::from_value(signals.clone())
                    .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
                signals.into_iter().map(|signal| signal.name).collect()
            } else {
                return Err(tool_error(
                    "INVALID_ARG",
                    "names or signals must be provided",
                ));
            };
            if names.len() != 34 {
                return Err(tool_error(
                    "INVALID_ARG",
                    "exactly 34 signal names are required",
                ));
            }
            for (signal, name) in project.signals.iter_mut().zip(names) {
                if name.is_empty() {
                    return Err(tool_error("INVALID_ARG", "signal names must not be empty"));
                }
                signal.name = name;
            }
        }
        project
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        Ok(json!({"open":false,"signals":project.signals}))
    }

    fn group_dialog(&self, params: &Value) -> Result<Value, ToolError> {
        let project = self.project_snapshot()?;
        let group = param_str(params, "id")
            .map(|id| {
                project
                    .groups
                    .iter()
                    .find(|group| group.id == id)
                    .cloned()
                    .ok_or_else(|| tool_error("UNKNOWN_GROUP", format!("unknown group {id}")))
            })
            .transpose()?;
        Ok(json!({"open":true,"group":group,"groups":project.groups}))
    }

    fn mutate_groups(&self, id: &str, params: &Value) -> Result<Value, ToolError> {
        let mut project = self.project_snapshot()?;
        let selected;
        match id {
            "groups.create" => {
                let group = group_from_params(params, &project.groups)?;
                ensure_unique_group(&project.groups, &group, None)?;
                selected = Some(group.clone());
                project.groups.push(group);
            }
            "groups.dialog.ok" => {
                let group = group_from_params(params, &project.groups)?;
                validate_group(&group)?;
                if let Some(index) = project.groups.iter().position(|item| item.id == group.id) {
                    ensure_unique_group(&project.groups, &group, Some(index))?;
                    project.groups[index] = group.clone();
                } else {
                    ensure_unique_group(&project.groups, &group, None)?;
                    project.groups.push(group.clone());
                }
                selected = Some(group);
            }
            "groups.edit" => {
                let group_id = required_str(params, &["id"])?;
                let index = group_index(&project.groups, group_id)?;
                let mut group = project.groups[index].clone();
                patch_group(&mut group, params)?;
                validate_group(&group)?;
                ensure_unique_group(&project.groups, &group, Some(index))?;
                project.groups[index] = group.clone();
                selected = Some(group);
            }
            "groups.copy" => {
                let group_id = required_str(params, &["id"])?;
                let source = project.groups[group_index(&project.groups, group_id)?].clone();
                let mut group = source;
                group.id = params
                    .get("new_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| next_group_id(&project.groups));
                group.name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{} Copy", group.name));
                validate_group(&group)?;
                ensure_unique_group(&project.groups, &group, None)?;
                selected = Some(group.clone());
                project.groups.push(group);
            }
            "groups.delete" => {
                let group_id = required_str(params, &["id"])?;
                let index = group_index(&project.groups, group_id)?;
                selected = Some(project.groups.remove(index));
                project
                    .rows
                    .retain(|row| !(row.kind == "group" && row.reference == group_id));
            }
            "groups.rename" => {
                let group_id = required_str(params, &["id"])?;
                let index = group_index(&project.groups, group_id)?;
                let mut group = project.groups[index].clone();
                group.name = required_nonempty_str(params, &["name"])?.to_owned();
                ensure_unique_group(&project.groups, &group, Some(index))?;
                project.groups[index] = group.clone();
                selected = Some(group);
            }
            "groups.members.add" | "groups.members.remove" => {
                let group_id = required_str(params, &["id"])?;
                let index = group_index(&project.groups, group_id)?;
                let wires = parse_wires_param(params)?;
                let group = &mut project.groups[index];
                if id == "groups.members.add" {
                    for wire in wires {
                        if !group.wires.contains(&wire) {
                            group.wires.push(wire);
                        }
                    }
                } else {
                    group.wires.retain(|wire| !wires.contains(wire));
                }
                validate_group(group)?;
                selected = Some(group.clone());
            }
            "groups.reverse_display_order" => {
                let group_id = required_str(params, &["id"])?;
                let index = group_index(&project.groups, group_id)?;
                let group = &mut project.groups[index];
                group.display_order = if group.display_order == "high_top" {
                    "high_bottom".to_owned()
                } else {
                    "high_top".to_owned()
                };
                selected = Some(group.clone());
            }
            _ => {
                return Err(tool_error(
                    "INTERNAL",
                    format!("unknown group mutation: {id}"),
                ));
            }
        }
        project
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        Ok(json!({
            "open":false,
            "group":selected,
            "deleted":id == "groups.delete",
            "groups":project.groups
        }))
    }

    fn mutate_rows(&self, id: &str, params: &Value) -> Result<Value, ToolError> {
        let mut project = self.project_snapshot()?;
        let mut affected = None;
        match id {
            "rows.add.signal"
            | "rows.add.group"
            | "rows.add.interpreter"
            | "rows.insert.signal"
            | "rows.insert.group"
            | "rows.insert.interpreter" => {
                let kind = if id.ends_with(".signal") {
                    "signal"
                } else if id.ends_with(".group") {
                    "group"
                } else {
                    "interpreter"
                };
                let row = row_from_params(params, kind, &project)?;
                ensure_unique_row(&project.rows, &row.id)?;
                let index = if id.starts_with("rows.insert.") {
                    param_usize(params, "index", project.rows.len(), project.rows.len())?
                } else {
                    project.rows.len()
                };
                affected = Some(row.clone());
                project.rows.insert(index, row);
            }
            "rows.remove"
            | "rows.remove.signal"
            | "rows.remove.group"
            | "rows.remove.interpreter" => {
                let row_id = required_str(params, &["id"])?;
                let index = row_index(&project.rows, row_id)?;
                if let Some(expected) = id.strip_prefix("rows.remove.")
                    && project.rows[index].kind != expected
                {
                    return Err(tool_error(
                        "INVALID_ARG",
                        format!("row {row_id} is not a {expected}"),
                    ));
                }
                affected = Some(project.rows.remove(index));
            }
            "rows.remove_all" => {
                project.rows.clear();
            }
            "rows.add_all" => {
                for signal in &project.signals {
                    let reference = signal.wire_name.clone();
                    if project
                        .rows
                        .iter()
                        .any(|row| row.kind == "signal" && row.reference == reference)
                    {
                        continue;
                    }
                    let id = next_row_id(&project.rows);
                    project.rows.push(default_row(id, "signal", reference));
                }
            }
            "rows.reorder" => {
                let ids = params
                    .get("ids")
                    .and_then(Value::as_array)
                    .ok_or_else(|| tool_error("INVALID_ARG", "ids must be an array"))?;
                if ids.len() != project.rows.len() {
                    return Err(tool_error(
                        "INVALID_ARG",
                        "reorder must contain every row exactly once",
                    ));
                }
                let mut reordered = Vec::with_capacity(project.rows.len());
                for value in ids {
                    let row_id = value
                        .as_str()
                        .ok_or_else(|| tool_error("INVALID_ARG", "row ids must be strings"))?;
                    let row = project
                        .rows
                        .iter()
                        .find(|row| row.id == row_id)
                        .cloned()
                        .ok_or_else(|| {
                            tool_error("UNKNOWN_ROW", format!("unknown row {row_id}"))
                        })?;
                    if reordered
                        .iter()
                        .any(|item: &lp_project::Row| item.id == row.id)
                    {
                        return Err(tool_error("INVALID_ARG", "reorder row ids must be unique"));
                    }
                    reordered.push(row);
                }
                project.rows = reordered;
            }
            "rows.expand" | "rows.collapse" | "rows.toggle_expand" => {
                let row_id = required_str(params, &["id"])?;
                let index = row_index(&project.rows, row_id)?;
                let row = &mut project.rows[index];
                row.expanded = if id == "rows.toggle_expand" {
                    !row.expanded
                } else {
                    id == "rows.expand"
                };
                affected = Some(row.clone());
            }
            "rows.expand_all" | "rows.collapse_all" => {
                let expanded = id == "rows.expand_all";
                for row in &mut project.rows {
                    if matches!(row.kind.as_str(), "group" | "interpreter") {
                        row.expanded = expanded;
                    }
                }
            }
            "rows.height.set" => {
                let row_id = required_str(params, &["id"])?;
                let height = required_u64(params, &["height_px", "height", "value"])?;
                if !(8..=4096).contains(&height) {
                    return Err(tool_error("INVALID_ARG", "row height must be 8..4096 px"));
                }
                let index = row_index(&project.rows, row_id)?;
                let row = &mut project.rows[index];
                row.height_px = height as u32;
                affected = Some(row.clone());
            }
            "row.style.set" => {
                let row_id = required_str(params, &["id"])?;
                let style = required_str(params, &["style"])?;
                if !matches!(style, "digital" | "analog") {
                    return Err(tool_error(
                        "INVALID_ARG",
                        "row style must be digital or analog",
                    ));
                }
                let index = row_index(&project.rows, row_id)?;
                project.rows[index].style = style.to_owned();
                sync_row_style(&mut project, index, style);
                affected = Some(project.rows[index].clone());
            }
            "row.color.set" | "row.color.default" => {
                let row_id = required_str(params, &["id"])?;
                let color = if id == "row.color.default" {
                    "default"
                } else {
                    required_nonempty_str(params, &["color"])?
                };
                let index = row_index(&project.rows, row_id)?;
                project.rows[index].color = color.to_owned();
                sync_row_color(&mut project, index, color);
                affected = Some(project.rows[index].clone());
            }
            "group.radix.set"
            | "group.signed.set"
            | "group.wire_order.set"
            | "group.display_order.set" => {
                let group_id = required_str(params, &["id", "group_id"])?;
                let index = group_index(&project.groups, group_id)?;
                let group = &mut project.groups[index];
                match id {
                    "group.radix.set" => {
                        group.radix =
                            validate_radix(required_str(params, &["radix", "value"])?)?.to_owned();
                    }
                    "group.signed.set" => {
                        group.signed = required_bool(params, &["signed", "enabled"])?;
                    }
                    "group.wire_order.set" => {
                        group.wire_order =
                            required_nonempty_str(params, &["wire_order", "value"])?.to_owned();
                    }
                    _ => {
                        group.display_order =
                            required_nonempty_str(params, &["display_order", "value"])?.to_owned();
                    }
                }
                validate_group(group)?;
            }
            _ => {
                return Err(tool_error(
                    "INTERNAL",
                    format!("unknown row mutation: {id}"),
                ));
            }
        }
        project
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        Ok(json!({"row":affected,"rows":project.rows,"groups":project.groups}))
    }

    fn row_value(&self, params: &Value) -> Result<Value, ToolError> {
        let project = self.project_snapshot()?;
        let row_id = required_str(params, &["id", "row_id"])?;
        let row = project
            .rows
            .iter()
            .find(|row| row.id == row_id)
            .ok_or_else(|| tool_error("UNKNOWN_ROW", format!("unknown row {row_id}")))?;
        let sample = required_u64(params, &["sample"])?;
        let capture = self
            .captures
            .latest()
            .map_err(store_error)?
            .ok_or_else(no_capture)?;
        match row.kind.as_str() {
            "signal" => {
                let channel = parse_channel(Some(&Value::String(row.reference.clone())))?;
                let data = capture
                    .sample_at(sample)
                    .ok_or_else(|| tool_error("INVALID_ARG", "sample is outside capture"))?;
                Ok(json!({
                    "row_id":row.id,
                    "sample":sample,
                    "value":(data >> channel) & 1,
                    "formatted":if (data >> channel) & 1 == 0 {"L"} else {"H"}
                }))
            }
            "group" => group_value_at(&project, &capture, &row.reference, sample),
            _ => Err(tool_error(
                "NOT_SUPPORTED",
                "interpreter row values require the decoder pipeline",
            )),
        }
    }

    fn group_value(&self, params: &Value) -> Result<Value, ToolError> {
        let project = self.project_snapshot()?;
        let group_id = required_str(params, &["id", "group_id"])?;
        let sample = required_u64(params, &["sample"])?;
        let capture = self
            .captures
            .latest()
            .map_err(store_error)?
            .ok_or_else(no_capture)?;
        group_value_at(&project, &capture, group_id, sample)
    }

    // Decode a project interpreter's frames from the latest capture. Channels
    // come from the interpreter's wire assignment; protocols that need no
    // configuration (I2C, 1-Wire) decode directly, SPI uses mode-0 8-bit, and
    // UART takes an optional `baud` parameter (default 9600). The imported LPF
    // parameter blob is intentionally not interpreted.
    fn interpreter_frames(&self, params: &Value) -> Result<Value, ToolError> {
        let project = self.project_snapshot()?;
        let id = required_str(params, &["id", "interpreter_id"])?;
        let interpreter = project
            .interpreters
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| tool_error("UNKNOWN_INTERP", format!("unknown interpreter {id}")))?;
        let capture = self
            .captures
            .latest()
            .map_err(store_error)?
            .ok_or_else(no_capture)?;
        let rate = if capture.sample_period_s > 0.0 {
            (1.0 / capture.sample_period_s).round() as u64
        } else {
            1
        };
        let line = |index: usize| -> Vec<bool> {
            interpreter
                .wires
                .get(index)
                .map(|&wire| channel_levels(&capture, wire))
                .unwrap_or_default()
        };
        let frames: Vec<Value> = match interpreter.kind.as_str() {
            "i2c" => decode_i2c_frames(&line(1), &line(0)),
            "onewire" => decode_onewire_frames(&line(0), rate),
            "spi" => decode_spi_frames(&line(1), &line(0)),
            "uart" => {
                let baud = params.get("baud").and_then(Value::as_u64).unwrap_or(9600) as u32;
                decode_uart_frames(&line(0), rate, baud)
            }
            "can" => {
                let bitrate = params
                    .get("bitrate")
                    .or_else(|| params.get("baud"))
                    .and_then(Value::as_u64)
                    .unwrap_or(500_000) as u32;
                decode_can_frames(&line(0), rate, bitrate)
            }
            "parallel" => {
                let clock = line(0);
                // The LPF lists the data wires most-significant first, while
                // decode_parallel takes them least-significant first.
                let data: Vec<Vec<bool>> = interpreter
                    .wires
                    .iter()
                    .skip(1)
                    .rev()
                    .map(|&wire| channel_levels(&capture, wire))
                    .collect();
                decode_parallel_frames(&clock, &data)
            }
            "iso7816" => {
                let baud = params.get("baud").and_then(Value::as_u64).unwrap_or(9600) as u32;
                let inverse = params.get("inverse").and_then(Value::as_bool).unwrap_or(false);
                decode_iso7816_frames(&line(0), rate, baud, inverse)
            }
            other => {
                return Ok(json!({
                    "id": id,
                    "kind": other,
                    "supported": false,
                    "frames": [],
                    "note": format!("live decoding for {other} is not wired yet"),
                }));
            }
        };
        Ok(json!({ "id": id, "kind": interpreter.kind, "supported": true, "frames": frames }))
    }

    fn start_recurring_from_dispatch(&self, max_runs: Option<u64>) -> Result<(), ToolError> {
        let inner = self
            .self_ref
            .upgrade()
            .ok_or_else(|| tool_error("INTERNAL", "application state is shutting down"))?;
        AppState { inner }
            .start_recurring(max_runs)
            .map_err(|error| error.0)
    }

    fn wait_for_acquisition(&self, target: &str, timeout: Duration) -> Result<Value, ToolError> {
        if !matches!(target, "ready" | "idle" | "phase_change") {
            return Err(tool_error(
                "INVALID_ARG",
                format!("unknown wait target: {target}"),
            ));
        }
        let initial = self.status_snapshot()?;
        let deadline = Instant::now() + timeout;
        loop {
            let current = self.status_snapshot()?;
            let reached = match target {
                "ready" => current.state == "ready",
                "idle" => {
                    !current.recurring && matches!(current.state, "idle" | "ready" | "halted")
                }
                "phase_change" => {
                    current.state != initial.state || current.acq_count != initial.acq_count
                }
                _ => false,
            };
            if reached {
                return serde_json::to_value(current).map_err(json_error);
            }
            if Instant::now() >= deadline {
                return Err(tool_error(
                    "ACQ_TIMEOUT",
                    format!("wait for {target} timed out"),
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn status_snapshot(&self) -> Result<crate::acquisitions::AcqStatus, ToolError> {
        self.acquisition_status
            .read()
            .map_err(|_| tool_error("INTERNAL", "acquisition status lock poisoned"))
            .map(|status| status.clone())
    }

    fn measurement_status(&self) -> Result<Value, ToolError> {
        let project = self
            .project
            .read()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))?
            .clone();
        let capture = self.captures.latest().map_err(store_error)?;
        let measurements = project
            .measurements
            .iter()
            .map(|slot| {
                capture.as_ref().map_or_else(
                    || json!({"slot":slot,"result":null,"reason":"no_capture"}),
                    |capture| match measurement_for_slot(&project, capture, slot) {
                        Ok(result) => json!({"slot":slot,"result":result}),
                        Err(error) => json!({"slot":slot,"result":null,"error":error}),
                    },
                )
            })
            .collect::<Vec<_>>();
        Ok(json!({"measurements":measurements,"capture_id":capture.map(|capture| capture.id)}))
    }

    fn compute_measurement(&self, params: &Value) -> Result<Value, ToolError> {
        let project = self.project_snapshot()?;
        let slot = measurement_slot_index(params)?;
        let capture = self
            .captures
            .latest()
            .map_err(store_error)?
            .ok_or_else(no_capture)?;
        let result = measurement_for_slot(&project, &capture, &project.measurements[slot])?;
        Ok(json!({
            "slot": project.measurements[slot],
            "result": result,
            "capture_id": capture.id
        }))
    }

    fn measurement_dialog(&self) -> Result<Value, ToolError> {
        let project = self.project_snapshot()?;
        Ok(json!({
            "open": true,
            "measurements": project.measurements,
            "types": [
                "frequency", "period", "interval", "rate", "transitions", "cycles",
                "duty", "inverse_duty", "positive_width", "negative_width"
            ],
            "references": ["trigger", "reference", "A", "B", "C", "D", "E", "F"],
            "sources": project.signals.iter().map(|signal| signal.wire_name.clone()).collect::<Vec<_>>()
        }))
    }

    fn measurement_panel_click(&self, params: &Value) -> Result<Value, ToolError> {
        let slot = measurement_slot_index(params)?;
        let project = self.project_snapshot()?;
        let capture = self.captures.latest().map_err(store_error)?;
        let result = capture
            .as_ref()
            .map(|capture| measurement_for_slot(&project, capture, &project.measurements[slot]))
            .transpose()?;
        Ok(json!({
            "selected_slot": slot,
            "slot": project.measurements[slot],
            "result": result,
            "capture_id": capture.map(|capture| capture.id)
        }))
    }

    fn mutate_measurements(&self, id: &str, params: &Value) -> Result<Value, ToolError> {
        let mut project = self.project_snapshot()?;
        if id == "measure.dialog.ok"
            && let Some(value) = params.get("measurements")
        {
            let measurements =
                serde_json::from_value::<Vec<lp_project::MeasurementSlot>>(value.clone())
                    .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
            if measurements.len() != 4 {
                return Err(tool_error(
                    "INVALID_ARG",
                    "measurements must contain exactly four slots",
                ));
            }
            for (index, slot) in measurements.iter().enumerate() {
                validate_measurement_slot(slot, index)?;
            }
            project.measurements = measurements;
        } else {
            let index = measurement_slot_index(params)?;
            let slot = &mut project.measurements[index];
            match id {
                "measure.slot.type.set" => {
                    slot.kind =
                        measurement_kind_name(required_str(params, &["type", "kind", "value"])?)?;
                }
                "measure.slot.left.set" => {
                    slot.left = measurement_reference_name(required_str(
                        params,
                        &["left", "reference", "value"],
                    )?)?;
                }
                "measure.slot.right.set" => {
                    slot.right = measurement_reference_name(required_str(
                        params,
                        &["right", "reference", "value"],
                    )?)?;
                }
                "measure.slot.source.set" => {
                    slot.source = channel_name(parse_channel(first_param(
                        params,
                        &["source", "channel", "value"],
                    ))?);
                }
                "measure.dialog.ok" => patch_measurement_slot(slot, params)?,
                _ => {
                    return Err(tool_error(
                        "INTERNAL",
                        format!("unknown measurement mutation: {id}"),
                    ));
                }
            }
            validate_measurement_slot(slot, index)?;
        }
        project
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        Ok(json!({"open":false,"measurements":project.measurements}))
    }

    fn notes_snapshot(&self, open: bool) -> Result<Value, ToolError> {
        let project = self.project_snapshot()?;
        Ok(json!({
            "open": open,
            "notes": project.notes,
            "length": project.notes.chars().count()
        }))
    }

    fn set_notes(&self, params: &Value) -> Result<Value, ToolError> {
        const MAX_NOTES_BYTES: usize = 1_048_576;
        let notes = required_str(params, &["notes", "text", "value"])?;
        if notes.len() > MAX_NOTES_BYTES {
            return Err(tool_error(
                "INVALID_ARG",
                format!("notes exceed the {MAX_NOTES_BYTES}-byte limit"),
            ));
        }
        let mut project = self.project_snapshot()?;
        project.notes = notes.to_owned();
        project
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        Ok(json!({
            "open": false,
            "notes": project.notes,
            "length": project.notes.chars().count()
        }))
    }

    fn current_settings(&self) -> Result<lp_project::Settings, ToolError> {
        self.project
            .read()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))
            .map(|project| project.settings.clone())
    }

    fn project_snapshot(&self) -> Result<Project, ToolError> {
        self.project
            .read()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))
            .map(|project| project.clone())
    }

    fn cursor_snapshot(&self) -> Result<Value, ToolError> {
        let project = self.project_snapshot()?;
        Ok(json!({
            "cursors": project.cursors,
            "snap": project.settings.options.cursor_snap,
            "show_cursors": project.settings.options.show_cursors,
            "cursor_qty": project.settings.options.cursor_qty
        }))
    }

    fn mutate_cursors(&self, id: &str, params: &Value) -> Result<Value, ToolError> {
        let mut project = self.project_snapshot()?;
        let capture = self.captures.latest().map_err(store_error)?;
        let period = capture
            .as_ref()
            .map(|capture| capture.sample_period_s)
            .unwrap_or_else(|| 1.0 / project.settings.sample.rate_hz.max(1) as f64);
        let reference = capture
            .as_ref()
            .map_or(0, |capture| capture.reference_sample);

        if let Some(suffix) = id.strip_prefix("cursor.place.") {
            let cursor_id = cursor_id_from_suffix(suffix)?;
            let offset = cursor_offset_from_params(params, reference, period, true)?;
            move_cursor_and_followers(&mut project, cursor_id, offset, period)?;
        } else if id == "cursor.place_all" {
            if let Some(values) = params.get("positions").and_then(Value::as_array) {
                if values.len() != project.cursors.len() {
                    return Err(tool_error(
                        "INVALID_ARG",
                        "positions must contain exactly six cursor positions",
                    ));
                }
                let mut offsets = Vec::with_capacity(values.len());
                for value in values {
                    offsets.push(cursor_offset_from_params(value, reference, period, true)?);
                }
                for (cursor, offset) in project.cursors.iter_mut().zip(offsets) {
                    set_cursor_offset(cursor, offset, period);
                }
            } else {
                let offset = cursor_offset_from_params(params, reference, period, true)?;
                for cursor in &mut project.cursors {
                    set_cursor_offset(cursor, offset, period);
                }
            }
        } else if id == "cursor.drag" {
            let cursor_id = cursor_id_param(params)?;
            let index = cursor_index(&project, cursor_id)?;
            let delta = cursor_delta_from_params(params, period)?;
            let offset = project.cursors[index].offset_samples.saturating_add(delta);
            move_cursor_and_followers(&mut project, cursor_id, offset, period)?;
        } else if id == "cursor.set" {
            let enabled = optional_bool(params, &["enabled", "visible"])?
                .unwrap_or(!project.settings.options.show_cursors);
            if let Some(value) = params.get("id") {
                let cursor_id = parse_cursor_id(value)?;
                let index = cursor_index(&project, cursor_id)?;
                project.cursors[index].visible = enabled;
            } else {
                for cursor in &mut project.cursors {
                    cursor.visible = enabled;
                }
            }
        } else if id == "cursor.snap.toggle" {
            project.settings.options.cursor_snap = optional_bool(params, &["enabled"])?
                .unwrap_or(!project.settings.options.cursor_snap);
        } else if let Some(suffix) = id.strip_prefix("cursor.tracking.set.") {
            let cursor_id = cursor_id_from_suffix(suffix)?;
            let target = match first_param(params, &["tracks", "target", "cursor"]) {
                None | Some(Value::Null) => None,
                Some(value) => Some(parse_cursor_id(value)?),
            };
            set_cursor_tracking(&mut project, cursor_id, target)?;
        } else if id == "cursor.tracking.interlock_all" {
            let target = first_param(params, &["target", "cursor"])
                .map(parse_cursor_id)
                .transpose()?
                .unwrap_or('A');
            for cursor in &mut project.cursors {
                cursor.tracks = (cursor.id != target).then_some(target);
            }
        } else if id == "cursor.tracking.release_all" {
            for cursor in &mut project.cursors {
                cursor.tracks = None;
            }
        } else {
            return Err(tool_error(
                "INTERNAL",
                format!("unknown cursor mutation: {id}"),
            ));
        }

        let visible = project
            .cursors
            .iter()
            .filter(|cursor| cursor.visible)
            .count();
        project.settings.options.cursor_qty =
            u8::try_from(visible).map_err(|_| tool_error("INTERNAL", "cursor count overflow"))?;
        project.settings.options.show_cursors = visible != 0;
        project
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        Ok(json!({
            "cursors": project.cursors,
            "snap": project.settings.options.cursor_snap,
            "show_cursors": project.settings.options.show_cursors,
            "cursor_qty": project.settings.options.cursor_qty
        }))
    }

    fn statelist_get(&self, params: &Value) -> Result<Value, ToolError> {
        let project = self.project_snapshot()?;
        let config = statelist_config(&project.view.statelist);
        let offset = params
            .get("offset")
            .map(|_| param_usize(params, "offset", 0, usize::MAX))
            .transpose()?
            .unwrap_or_else(|| statelist_scroll_row(&config));
        let limit = param_usize(params, "limit", 25, 2000)?;
        let capture = self.captures.latest().map_err(store_error)?;
        statelist_response(&project, capture.as_ref(), config, offset, limit)
    }

    fn mutate_statelist(&self, id: &str, params: &Value) -> Result<Value, ToolError> {
        let mut project = self.project_snapshot()?;
        let capture = self.captures.latest().map_err(store_error)?;
        let mut config = statelist_config(&project.view.statelist);
        let page_size = param_usize(params, "page_size", 25, 2000)?;
        let row_count = capture.as_ref().map_or(0, |capture| capture.runs.len());

        match id {
            "statelist.format.set" => {
                project.settings.options.statelist_format =
                    validate_statelist_format(required_str(params, &["format", "value"])?)?
                        .to_owned();
            }
            "statelist.relative.set" => {
                config["relative"] = Value::Bool(required_bool(params, &["relative", "enabled"])?);
            }
            "statelist.column.reorder" => {
                let columns = parse_statelist_columns(params.get("columns"))?;
                config["columns"] = serde_json::to_value(columns).map_err(json_error)?;
            }
            "statelist.column.format.set" => {
                let column = required_str(params, &["column", "id"])?;
                validate_statelist_column(column)?;
                let format =
                    validate_statelist_format(required_str(params, &["format", "value"])?)?;
                let formats = config
                    .get_mut("column_formats")
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| tool_error("INTERNAL", "state-list formats are invalid"))?;
                formats.insert(column.to_owned(), Value::String(format.to_owned()));
            }
            "statelist.scroll.page_up" | "statelist.scroll.page_down" => {
                let current = statelist_scroll_row(&config);
                let next = if id.ends_with("page_up") {
                    current.saturating_sub(page_size)
                } else {
                    current.saturating_add(page_size)
                };
                set_statelist_scroll_row(&mut config, clamp_statelist_row(next, row_count));
            }
            "statelist.scroll.key" => {
                let current = statelist_scroll_row(&config);
                let key = required_str(params, &["key"])?.to_ascii_lowercase();
                let next = match key.as_str() {
                    "arrowup" | "up" => current.saturating_sub(1),
                    "arrowdown" | "down" => current.saturating_add(1),
                    "pageup" => current.saturating_sub(page_size),
                    "pagedown" => current.saturating_add(page_size),
                    "home" => 0,
                    "end" => row_count.saturating_sub(1),
                    _ => {
                        return Err(tool_error(
                            "INVALID_ARG",
                            format!("unsupported state-list key: {key}"),
                        ));
                    }
                };
                set_statelist_scroll_row(&mut config, clamp_statelist_row(next, row_count));
            }
            "statelist.scroll.drag" => {
                let row = required_u64(params, &["row", "index", "offset"])?;
                let row = usize::try_from(row)
                    .map_err(|_| tool_error("INVALID_ARG", "state-list row is too large"))?;
                set_statelist_scroll_row(&mut config, clamp_statelist_row(row, row_count));
            }
            "statelist.place_cursor" => {
                let capture = capture.as_ref().ok_or_else(no_capture)?;
                let row = required_u64(params, &["row", "index"])?;
                let row = usize::try_from(row)
                    .map_err(|_| tool_error("INVALID_ARG", "state-list row is too large"))?;
                let sample = statelist_row_sample(capture, row)?;
                let cursor_id = first_param(params, &["cursor", "id"])
                    .map(parse_cursor_id)
                    .transpose()?
                    .unwrap_or('A');
                let offset = i128::from(sample)
                    .saturating_sub(i128::from(capture.reference_sample))
                    .clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                    as i64;
                move_cursor_and_followers(
                    &mut project,
                    cursor_id,
                    offset,
                    capture.sample_period_s,
                )?;
                set_statelist_scroll_row(&mut config, row);
            }
            _ => {
                return Err(tool_error(
                    "INTERNAL",
                    format!("unknown state-list mutation: {id}"),
                ));
            }
        }
        project.view.statelist = config.clone();
        project
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        statelist_response(
            &project,
            capture.as_ref(),
            config.clone(),
            statelist_scroll_row(&config),
            page_size,
        )
    }

    fn mutate_view(&self, id: &str, params: &Value) -> Result<Value, ToolError> {
        let mut project = self.project_snapshot()?;
        match id {
            "view.set" => {
                let value = params.get("view").unwrap_or(params);
                project.view = serde_json::from_value(value.clone())
                    .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
            }
            "view.graticule.toggle" => {
                project.settings.options.show_graticule = optional_bool(params, &["enabled"])?
                    .unwrap_or(!project.settings.options.show_graticule);
            }
            "view.show_trigger.toggle" => {
                project.settings.options.show_trigger = optional_bool(params, &["enabled"])?
                    .unwrap_or(!project.settings.options.show_trigger);
            }
            "view.show_cursors.set" => {
                let ids = params
                    .get("ids")
                    .and_then(Value::as_array)
                    .ok_or_else(|| tool_error("INVALID_ARG", "ids must be an array of A..F"))?;
                let mut visible = [false; 6];
                for value in ids {
                    let id = value
                        .as_str()
                        .and_then(|value| value.chars().next())
                        .map(|value| value.to_ascii_uppercase())
                        .ok_or_else(|| tool_error("INVALID_ARG", "cursor id must be A..F"))?;
                    let index = "ABCDEF"
                        .find(id)
                        .ok_or_else(|| tool_error("INVALID_ARG", "cursor id must be A..F"))?;
                    visible[index] = true;
                }
                for (cursor, visible) in project.cursors.iter_mut().zip(visible) {
                    cursor.visible = visible;
                }
                project.settings.options.show_cursors = visible.iter().any(|value| *value);
                project.settings.options.cursor_qty =
                    u8::try_from(visible.iter().filter(|value| **value).count())
                        .map_err(|_| tool_error("INTERNAL", "cursor count overflow"))?;
            }
            "view.show_cursors.all" | "view.show_cursors.none" => {
                let visible = id == "view.show_cursors.all";
                for cursor in &mut project.cursors {
                    cursor.visible = visible;
                }
                project.settings.options.show_cursors = visible;
                project.settings.options.cursor_qty = if visible { 6 } else { 0 };
            }
            "view.cursor_qty.set" => {
                let qty = required_u8(params, &["qty", "cursor_qty", "value"])?;
                if qty > 6 {
                    return Err(tool_error("INVALID_ARG", "cursor quantity must be 0..6"));
                }
                for (index, cursor) in project.cursors.iter_mut().enumerate() {
                    cursor.visible = index < usize::from(qty);
                }
                project.settings.options.cursor_qty = qty;
                project.settings.options.show_cursors = qty != 0;
            }
            "view.color_scheme.set" => {
                project.settings.options.color_scheme =
                    required_nonempty_str(params, &["color_scheme", "scheme", "value"])?.to_owned();
            }
            "view.alt_background.enable" => {
                project.settings.options.alt_background.enabled =
                    required_bool(params, &["enabled"])?;
            }
            "view.alt_background.adjust" => {
                if let Some(color) = optional_nonempty_str(params, &["color"])? {
                    project.settings.options.alt_background.color = color.to_owned();
                }
                if let Some(intensity) = optional_f64(params, &["intensity"]).transpose()? {
                    if !(0.0..=1.0).contains(&intensity) {
                        return Err(tool_error(
                            "INVALID_ARG",
                            "intensity must be between 0 and 1",
                        ));
                    }
                    project.settings.options.alt_background.intensity = intensity;
                }
            }
            "view.waveforms_in_front.toggle" => {
                project.settings.options.waveforms_in_front = optional_bool(params, &["enabled"])?
                    .unwrap_or(!project.settings.options.waveforms_in_front);
            }
            "view.large_waveforms.toggle" => {
                project.settings.options.large_waveforms = optional_bool(params, &["enabled"])?
                    .unwrap_or(!project.settings.options.large_waveforms);
            }
            "view.sample_reference.set" => {
                project.settings.options.sample_reference =
                    required_nonempty_str(params, &["sample_reference", "reference", "value"])?
                        .to_owned();
            }
            "view.reference_position.set" => {
                let value = required_f64(params, &["reference_position", "position", "value"])?;
                if !(0.0..=1.0).contains(&value) {
                    return Err(tool_error(
                        "INVALID_ARG",
                        "reference position must be between 0 and 1",
                    ));
                }
                project.settings.options.reference_position = value;
            }
            "view.scale_relative.set" => {
                project.settings.options.scale_relative =
                    required_bool(params, &["scale_relative", "enabled"])?;
            }
            "view.units.set" => {
                let units = required_str(params, &["units", "value"])?;
                if !matches!(units, "time" | "samples") {
                    return Err(tool_error("INVALID_ARG", "units must be time or samples"));
                }
                project.settings.options.units = units.to_owned();
            }
            "view.scale_factor.set" => {
                let scale = required_f64(params, &["scale_s_per_px", "scale", "value"])?;
                if scale <= 0.0 {
                    return Err(tool_error("INVALID_ARG", "scale must be greater than zero"));
                }
                project.view.scale_s_per_px = scale;
            }
            "view.reference_offset.set" => {
                project.view.reference_offset_samples =
                    required_i64(params, &["reference_offset_samples", "offset", "value"])?;
            }
            "view.panel.waveforms" | "view.panel.statelist" | "view.panel.notes" => {
                project.view.panel = id
                    .strip_prefix("view.panel.")
                    .ok_or_else(|| tool_error("INTERNAL", "invalid panel operation"))?
                    .to_owned();
            }
            "view.theme.set" => {
                project.view.theme = required_nonempty_str(params, &["theme", "value"])?.to_owned();
            }
            "view.control_rows.set" => {
                let selections = params
                    .get("selections")
                    .cloned()
                    .ok_or_else(|| tool_error("INVALID_ARG", "selections are required"))?;
                let values = params
                    .get("values")
                    .cloned()
                    .ok_or_else(|| tool_error("INVALID_ARG", "values are required"))?;
                project.settings.controls.selections = serde_json::from_value(selections)
                    .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
                project.settings.controls.values = serde_json::from_value(values)
                    .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
            }
            _ => {
                return Err(tool_error(
                    "INTERNAL",
                    format!("unknown view mutation: {id}"),
                ));
            }
        }
        project
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        Ok(json!({
            "view":project.view,
            "options":project.settings.options,
            "cursors":project.cursors
        }))
    }

    fn navigate_view(&self, id: &str, params: &Value) -> Result<Value, ToolError> {
        let mut project = self.project_snapshot()?;
        let capture = self.captures.latest().map_err(store_error)?;
        let mut location = capture
            .as_ref()
            .map(|capture| view_location(&project, capture));
        match id {
            "view.zoom.in" | "view.zoom.at" => {
                project.view.scale_s_per_px = zoom_125(project.view.scale_s_per_px, true);
                if id == "view.zoom.at" {
                    let capture = capture.as_ref().ok_or_else(no_capture)?;
                    let sample = required_u64(params, &["sample", "at"])?;
                    set_view_location(&mut project, capture, sample);
                    location = Some(sample.min(capture.expanded_len().saturating_sub(1)));
                }
            }
            "view.zoom.out" | "view.zoom.out_at" => {
                project.view.scale_s_per_px = zoom_125(project.view.scale_s_per_px, false);
                if id == "view.zoom.out_at" {
                    let capture = capture.as_ref().ok_or_else(no_capture)?;
                    let sample = required_u64(params, &["sample", "at"])?;
                    set_view_location(&mut project, capture, sample);
                    location = Some(sample.min(capture.expanded_len().saturating_sub(1)));
                }
            }
            "view.zoom.all" => {
                let capture = capture.as_ref().ok_or_else(no_capture)?;
                let width = optional_u64(params, &["viewport_px"])?
                    .unwrap_or(1000)
                    .clamp(1, 100_000);
                project.view.scale_s_per_px =
                    capture.expanded_len() as f64 * capture.sample_period_s / width as f64;
                let sample = capture.expanded_len().saturating_sub(1) / 2;
                set_view_location(&mut project, capture, sample);
                location = Some(sample);
            }
            "view.scroll.by" | "view.scroll.drag" => {
                let delta = scroll_delta_samples(params, capture.as_ref(), &project)?;
                let capture = capture.as_ref().ok_or_else(no_capture)?;
                let current = view_location(&project, capture);
                let target = add_signed_sample(current, delta, capture.expanded_len());
                set_view_location(&mut project, capture, target);
                location = Some(target);
            }
            "view.scroll.large"
            | "view.scroll.small"
            | "view.scroll.key_left"
            | "view.scroll.key_right" => {
                let capture = capture.as_ref().ok_or_else(no_capture)?;
                let width = optional_u64(params, &["viewport_px"])?
                    .unwrap_or(1000)
                    .clamp(1, 100_000);
                let visible = (project.view.scale_s_per_px * width as f64 / capture.sample_period_s)
                    .round()
                    .max(1.0) as i64;
                let magnitude = if id == "view.scroll.large" {
                    (visible * 2 / 5).max(1)
                } else if id == "view.scroll.small" {
                    (visible / 20).max(1)
                } else {
                    optional_u64(params, &["acceleration"])?
                        .unwrap_or(1)
                        .clamp(1, 100) as i64
                };
                let direction = if matches!(id, "view.scroll.key_left") {
                    -1
                } else {
                    optional_i64(params, &["direction"])?.unwrap_or(1).signum()
                };
                let current = view_location(&project, capture);
                let target = add_signed_sample(
                    current,
                    magnitude.saturating_mul(direction),
                    capture.expanded_len(),
                );
                set_view_location(&mut project, capture, target);
                location = Some(target);
            }
            "view.scroll_to.begin" | "view.scroll_to.trigger" | "view.scroll_to.end" => {
                let capture = capture.as_ref().ok_or_else(no_capture)?;
                let target = match id {
                    "view.scroll_to.begin" => 0,
                    "view.scroll_to.trigger" => capture.trigger_sample,
                    _ => capture.expanded_len().saturating_sub(1),
                };
                set_view_location(&mut project, capture, target);
                location = Some(target);
            }
            id if id.starts_with("view.scroll_to.cursor.") => {
                let capture = capture.as_ref().ok_or_else(no_capture)?;
                let cursor_id = id
                    .rsplit('.')
                    .next()
                    .and_then(|value| value.chars().next())
                    .map(|value| value.to_ascii_uppercase())
                    .ok_or_else(|| tool_error("INTERNAL", "invalid cursor operation"))?;
                let cursor = project
                    .cursors
                    .iter()
                    .find(|cursor| cursor.id == cursor_id)
                    .ok_or_else(|| tool_error("INVALID_ARG", "unknown cursor"))?;
                let target = add_signed_sample(
                    capture.reference_sample,
                    cursor.offset_samples,
                    capture.expanded_len(),
                );
                set_view_location(&mut project, capture, target);
                location = Some(target);
            }
            "view.next_edge" | "view.prev_edge" | "view.next_edge.row" | "view.prev_edge.row" => {
                let capture = capture.as_ref().ok_or_else(no_capture)?;
                let channel = parse_channel(first_param(params, &["source", "channel", "wire"]))?;
                let current = view_location(&project, capture);
                let edges = capture
                    .edges(channel, 0, capture.expanded_len())
                    .map_err(capture_error)?;
                let forward = id.contains("next_edge");
                let target = if forward {
                    edges
                        .iter()
                        .find(|edge| edge.sample > current)
                        .map(|edge| edge.sample)
                } else {
                    edges
                        .iter()
                        .rev()
                        .find(|edge| edge.sample < current)
                        .map(|edge| edge.sample)
                }
                .ok_or_else(|| tool_error("EDGE_NOT_FOUND", "no edge exists in that direction"))?;
                set_view_location(&mut project, capture, target);
                location = Some(target);
            }
            _ => {
                return Err(tool_error(
                    "INTERNAL",
                    format!("unknown view navigation: {id}"),
                ));
            }
        }
        project
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        *self
            .project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))? = project.clone();
        Ok(json!({"view":project.view,"location_sample":location}))
    }

    fn mutate_sample(&self, id: &str, params: &Value) -> Result<Value, ToolError> {
        let mut settings = self.current_settings()?;
        let extended_rates = settings.options.extended_rates;
        let sample = &mut settings.sample;
        match id {
            "sample.mode.set" => {
                sample.mode = match required_str(params, &["mode"])? {
                    "timing" => lp_project::SampleMode::Timing,
                    "state" => lp_project::SampleMode::State,
                    value => {
                        return Err(tool_error(
                            "INVALID_ARG",
                            format!("unsupported sample mode: {value}"),
                        ));
                    }
                };
            }
            "sample.rate.set" => {
                let hz = required_u64(params, &["rate_hz", "hz", "rate"])?;
                set_sample_rate(sample, hz, extended_rates)?;
            }
            "sample.rate.step_up" | "sample.rate.step_down" => {
                let current = usize::from(sample.rate_index);
                let direction = if id == "sample.rate.step_up" { -1 } else { 1 };
                let mut candidate = current as isize + direction;
                while let Some(entry) = usize::try_from(candidate)
                    .ok()
                    .and_then(|index| lp_proto::encode::rate::RATES.get(index))
                {
                    if entry.vendor_ui || extended_rates {
                        set_sample_rate(sample, entry.hz, extended_rates)?;
                        break;
                    }
                    candidate += direction;
                }
            }
            "sample.rate.units.set" => {
                sample.rate_units = required_str(params, &["units"])?.to_owned();
            }
            "sample.state.clock.set" => {
                sample.state.clock = required_u8(params, &["clock"])?;
            }
            "sample.state.edge.set" => {
                let edge = required_str(params, &["edge"])?;
                if !matches!(edge, "rising" | "falling") {
                    return Err(tool_error("INVALID_ARG", "edge must be rising or falling"));
                }
                sample.state.edge = edge.to_owned();
            }
            "sample.state.window.set" => {
                sample.state.window_index = required_u8(params, &["window_index", "index"])?;
                if let Some(value) = optional_f64(params, &["window_ns"]) {
                    sample.state.window_ns = value?;
                }
            }
            "sample.state.qualifier.enable" => {
                sample.state.qualifier.enabled = required_bool(params, &["enabled"])?;
            }
            "sample.state.qualifier.polarity" => {
                let polarity = required_str(params, &["polarity"])?;
                if !matches!(polarity, "high" | "low") {
                    return Err(tool_error("INVALID_ARG", "polarity must be high or low"));
                }
                sample.state.qualifier.polarity = polarity.to_owned();
            }
            "sample.state.declared_rate.set" => {
                sample.state.declared_rate_hz =
                    required_u64(params, &["declared_rate_hz", "rate_hz", "hz"])?;
            }
            "sample.state.declared_units.set" => {
                sample.state.declared_units = required_str(params, &["units"])?.to_owned();
            }
            "sample.compression.set" => {
                sample.compression = required_bool(params, &["compression", "enabled"])?;
            }
            "sample.prefill_timeout.set" | "sample.postfill_timeout.set" => {
                let timeout = if id == "sample.prefill_timeout.set" {
                    &mut sample.prefill_timeout
                } else {
                    &mut sample.postfill_timeout
                };
                timeout.index = required_u8(params, &["index"])?;
                timeout.ms = optional_u64(params, &["ms"])?;
            }
            "sample.pretrigger_buffer.set" => {
                sample.pretrigger_pct =
                    required_f64(params, &["pretrigger_pct", "percent", "value"])?;
            }
            _ => {
                return Err(tool_error(
                    "INTERNAL",
                    format!("unknown sample mutation: {id}"),
                ));
            }
        }
        let reconfigured = self.apply_settings_from_dispatch(settings.clone())?;
        Ok(json!({
            "sample":settings.sample,
            "hardware_reconfigure":reconfigured
        }))
    }

    fn settings_from_params(&self, params: Value) -> Result<lp_project::Settings, ToolError> {
        if let Some(setup) = params.get("setup") {
            return serde_json::from_value(setup.clone())
                .map_err(|error| tool_error("INVALID_ARG", error.to_string()));
        }
        let mut settings = self.current_settings()?;
        if let Some(sample) = params.get("sample") {
            settings.sample = serde_json::from_value(sample.clone())
                .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        }
        Ok(settings)
    }

    fn apply_settings_from_dispatch(
        &self,
        settings: lp_project::Settings,
    ) -> Result<bool, ToolError> {
        crate::acquisitions::validate_setup_settings(&settings)
            .map_err(crate::acquisitions::acquisition_tool_error)?;
        let mut candidate = self
            .project
            .read()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))?
            .clone();
        candidate.settings = settings.clone();
        candidate
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        let reconfigured = self
            .acquisition
            .lock()
            .map_err(|_| tool_error("INTERNAL", "acquisition lock poisoned"))?
            .apply_setup(&settings)
            .map_err(crate::acquisitions::acquisition_tool_error)?;
        self.project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))?
            .settings = settings;
        Ok(reconfigured)
    }

    fn apply_project_settings_only(&self, settings: lp_project::Settings) -> Result<(), ToolError> {
        let mut candidate = self
            .project
            .read()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))?
            .clone();
        candidate.settings = settings.clone();
        candidate
            .validate()
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        self.project
            .write()
            .map_err(|_| tool_error("INTERNAL", "project lock poisoned"))?
            .settings = settings;
        Ok(())
    }

    fn capture_param(&self, params: &Value, key: &str) -> Result<Capture, ToolError> {
        let id = match params.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) if value == "latest" => None,
            Some(Value::String(value)) => Some(value.parse::<u32>().map_err(|_| {
                tool_error(
                    "INVALID_ARG",
                    format!("{key} must be a capture ID or latest"),
                )
            })?),
            Some(value) if value.as_u64().is_some_and(|id| id <= u64::from(u32::MAX)) => {
                Some(value.as_u64().unwrap_or_default() as u32)
            }
            _ => {
                return Err(tool_error(
                    "INVALID_ARG",
                    format!("{key} must be a capture ID or latest"),
                ));
            }
        };
        id.map_or_else(|| self.captures.latest(), |id| self.captures.get(id))
            .map_err(store_error)?
            .ok_or_else(|| {
                tool_error(
                    "UNKNOWN_CAPTURE",
                    format!("unknown capture requested by {key}"),
                )
            })
    }

    fn capture_envelope(&self, capture: Capture) -> Result<Value, ToolError> {
        let pinned = self.captures.is_pinned(capture.id).map_err(store_error)?;
        Ok(json!({"capture":capture,"expanded_len":capture.expanded_len(),"pinned":pinned}))
    }
}

fn param_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}
fn first_param<'a>(params: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| params.get(*key))
}
fn required_str<'a>(params: &'a Value, keys: &[&str]) -> Result<&'a str, ToolError> {
    first_param(params, keys)
        .and_then(Value::as_str)
        .ok_or_else(|| tool_error("INVALID_ARG", format!("{} must be a string", keys[0])))
}

fn required_path(params: &Value) -> Result<PathBuf, ToolError> {
    let value = required_nonempty_str(params, &["path"])?;
    let path = PathBuf::from(value);
    if path == FsPath::new(".") || path.file_name().is_none() {
        return Err(tool_error("INVALID_ARG", "path must name a file"));
    }
    Ok(path)
}

fn validate_native_project_path(path: &FsPath) -> Result<(), ToolError> {
    let valid = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lpj"));
    if !valid {
        return Err(tool_error(
            "INVALID_ARG",
            "native projects must use the .lpj extension; LPF is import-only",
        ));
    }
    Ok(())
}

fn cursor_id_from_suffix(suffix: &str) -> Result<char, ToolError> {
    let mut chars = suffix.chars();
    let id = chars
        .next()
        .map(|value| value.to_ascii_uppercase())
        .ok_or_else(|| tool_error("INVALID_ARG", "cursor id is empty"))?;
    if chars.next().is_some() || !('A'..='F').contains(&id) {
        return Err(tool_error("INVALID_ARG", "cursor id must be A..F"));
    }
    Ok(id)
}

fn parse_cursor_id(value: &Value) -> Result<char, ToolError> {
    value
        .as_str()
        .ok_or_else(|| tool_error("INVALID_ARG", "cursor id must be a string"))
        .and_then(cursor_id_from_suffix)
}

fn cursor_id_param(params: &Value) -> Result<char, ToolError> {
    first_param(params, &["id", "cursor"])
        .ok_or_else(|| tool_error("INVALID_ARG", "cursor id is required"))
        .and_then(parse_cursor_id)
}

fn cursor_index(project: &Project, id: char) -> Result<usize, ToolError> {
    project
        .cursors
        .iter()
        .position(|cursor| cursor.id == id)
        .ok_or_else(|| tool_error("INVALID_ARG", format!("unknown cursor {id}")))
}

fn cursor_offset_from_params(
    params: &Value,
    reference: u64,
    period: f64,
    required: bool,
) -> Result<i64, ToolError> {
    if let Some(value) = first_param(params, &["offset_samples", "offset"]) {
        return value
            .as_i64()
            .ok_or_else(|| tool_error("INVALID_ARG", "offset_samples must be an integer"));
    }
    if let Some(value) = first_param(params, &["sample", "position_sample", "position"]) {
        let sample = value
            .as_u64()
            .ok_or_else(|| tool_error("INVALID_ARG", "sample must be a non-negative integer"))?;
        return Ok(i128::from(sample)
            .saturating_sub(i128::from(reference))
            .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64);
    }
    if let Some(value) = first_param(params, &["offset_s", "time_s", "time"]) {
        let seconds = value
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| tool_error("INVALID_ARG", "cursor time must be finite"))?;
        return Ok((seconds / period).round() as i64);
    }
    if required {
        Err(tool_error(
            "INVALID_ARG",
            "cursor position requires sample, offset_samples, time_s, or offset_s",
        ))
    } else {
        Ok(0)
    }
}

fn cursor_delta_from_params(params: &Value, period: f64) -> Result<i64, ToolError> {
    if let Some(value) = first_param(params, &["delta_samples", "samples", "delta"]) {
        return value
            .as_i64()
            .ok_or_else(|| tool_error("INVALID_ARG", "delta_samples must be an integer"));
    }
    let seconds = first_param(params, &["delta_s", "seconds"])
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| {
            tool_error(
                "INVALID_ARG",
                "cursor drag requires delta_samples or finite delta_s",
            )
        })?;
    Ok((seconds / period).round() as i64)
}

fn set_cursor_offset(cursor: &mut lp_project::Cursor, offset: i64, period: f64) {
    cursor.offset_samples = offset;
    cursor.offset_s = offset as f64 * period;
}

fn move_cursor_and_followers(
    project: &mut Project,
    id: char,
    offset: i64,
    period: f64,
) -> Result<(), ToolError> {
    let index = cursor_index(project, id)?;
    let delta = offset.saturating_sub(project.cursors[index].offset_samples);
    let mut moves = vec![id];
    loop {
        let before = moves.len();
        for cursor in &project.cursors {
            if cursor.tracks.is_some_and(|target| moves.contains(&target))
                && !moves.contains(&cursor.id)
            {
                moves.push(cursor.id);
            }
        }
        if moves.len() == before {
            break;
        }
    }
    for cursor in &mut project.cursors {
        if cursor.id == id {
            set_cursor_offset(cursor, offset, period);
        } else if moves.contains(&cursor.id) {
            set_cursor_offset(cursor, cursor.offset_samples.saturating_add(delta), period);
        }
    }
    Ok(())
}

fn set_cursor_tracking(
    project: &mut Project,
    id: char,
    target: Option<char>,
) -> Result<(), ToolError> {
    let index = cursor_index(project, id)?;
    if target == Some(id) {
        return Err(tool_error("INVALID_ARG", "a cursor cannot track itself"));
    }
    if let Some(target) = target {
        cursor_index(project, target)?;
        let mut next = Some(target);
        for _ in 0..project.cursors.len() {
            let Some(current) = next else { break };
            if current == id {
                return Err(tool_error(
                    "INVALID_ARG",
                    "cursor tracking cannot form a cycle",
                ));
            }
            next = project.cursors[cursor_index(project, current)?].tracks;
        }
    }
    project.cursors[index].tracks = target;
    Ok(())
}

fn statelist_config(value: &Value) -> Value {
    let mut config = value.as_object().cloned().unwrap_or_default();
    config
        .entry("relative".to_owned())
        .or_insert(Value::Bool(false));
    config.entry("scroll_row".to_owned()).or_insert(json!(0));
    config
        .entry("columns".to_owned())
        .or_insert_with(|| json!(["sample", "time", "data", "count"]));
    config
        .entry("column_formats".to_owned())
        .or_insert_with(|| json!({}));
    Value::Object(config)
}

fn help_contents() -> Value {
    json!({
        "title":"LogicPort for Linux Help",
        "language":"en",
        "sections":[
            {"id":"connect","title":"Connect","summary":"Attach the LA1034 and wait for FPGA configuration and device-ready status."},
            {"id":"sample","title":"Sampling","summary":"Choose timing or state mode, sample rate, compression, and pre-trigger depth."},
            {"id":"trigger","title":"Trigger","summary":"Configure edge, pattern, value, duration, prequalification, and term combination."},
            {"id":"acquire","title":"Acquire","summary":"Run single or recurring acquisitions, halt, or trigger immediately."},
            {"id":"analyze","title":"Analyze","summary":"Navigate waveforms, state lists, cursors, measurements, and protocol interpreters."},
            {"id":"projects","title":"Projects and export","summary":"Import LPF projects, save native projects, and export capture data."},
            {"id":"automation","title":"Automation","summary":"Every operation is available through the REST and MCP interfaces."}
        ],
        "docs":["README.md","docs/PROTOCOL.md","docs/FEATURE-INVENTORY.md"]
    })
}

fn help_shortcuts() -> Value {
    json!({
        "shortcuts":[
            {"keys":["Ctrl","N"],"operation":"file.new"},
            {"keys":["Ctrl","O"],"operation":"file.open"},
            {"keys":["Ctrl","S"],"operation":"file.save"},
            {"keys":["Space"],"operation":"acq.single"},
            {"keys":["Escape"],"operation":"acq.halt"},
            {"keys":["T"],"operation":"acq.trigger_immediate"},
            {"keys":["+"],"operation":"view.zoom.in"},
            {"keys":["-"],"operation":"view.zoom.out"},
            {"keys":["Home"],"operation":"view.scroll_to.begin"},
            {"keys":["End"],"operation":"view.scroll_to.end"},
            {"keys":["Left"],"operation":"view.scroll.key_left"},
            {"keys":["Right"],"operation":"view.scroll.key_right"}
        ]
    })
}

fn statelist_scroll_row(config: &Value) -> usize {
    config
        .get("scroll_row")
        .and_then(Value::as_u64)
        .and_then(|row| usize::try_from(row).ok())
        .unwrap_or(0)
}

fn set_statelist_scroll_row(config: &mut Value, row: usize) {
    config["scroll_row"] = json!(row);
}

fn clamp_statelist_row(row: usize, row_count: usize) -> usize {
    row.min(row_count.saturating_sub(1))
}

fn validate_statelist_format(value: &str) -> Result<&str, ToolError> {
    match value.to_ascii_lowercase().as_str() {
        "hex" => Ok("hex"),
        "binary" | "bin" => Ok("binary"),
        "unsigned" | "decimal" | "dec" => Ok("unsigned"),
        "signed" => Ok("signed"),
        "ascii" => Ok("ascii"),
        _ => Err(tool_error(
            "INVALID_ARG",
            format!("unsupported state-list format: {value}"),
        )),
    }
}

fn validate_statelist_column(value: &str) -> Result<&str, ToolError> {
    match value {
        "sample" | "time" | "data" | "count" => Ok(value),
        _ => Err(tool_error(
            "INVALID_ARG",
            format!("unsupported state-list column: {value}"),
        )),
    }
}

fn parse_statelist_columns(value: Option<&Value>) -> Result<Vec<String>, ToolError> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| tool_error("INVALID_ARG", "columns must be an array"))?;
    if values.len() != 4 {
        return Err(tool_error(
            "INVALID_ARG",
            "columns must contain sample, time, data, and count exactly once",
        ));
    }
    let mut columns = Vec::with_capacity(values.len());
    for value in values {
        let column = value
            .as_str()
            .ok_or_else(|| tool_error("INVALID_ARG", "column names must be strings"))?;
        validate_statelist_column(column)?;
        if columns.iter().any(|existing| existing == column) {
            return Err(tool_error(
                "INVALID_ARG",
                "state-list columns must be unique",
            ));
        }
        columns.push(column.to_owned());
    }
    Ok(columns)
}

fn statelist_row_sample(capture: &Capture, row: usize) -> Result<u64, ToolError> {
    if row >= capture.runs.len() {
        return Err(tool_error(
            "INVALID_ARG",
            format!("state-list row {row} is outside the capture"),
        ));
    }
    Ok(capture
        .runs
        .iter()
        .take(row)
        .fold(0_u64, |sample, run| sample.saturating_add(run.count)))
}

fn statelist_data(value: u64, format: &str) -> String {
    match format {
        "binary" => format!("{value:034b}"),
        "unsigned" => value.to_string(),
        "signed" => {
            let sign = 1_u64 << 33;
            let signed = if value & sign == 0 {
                i128::from(value)
            } else {
                i128::from(value) - (1_i128 << 34)
            };
            signed.to_string()
        }
        "ascii" => (0..4)
            .map(|shift| {
                let byte = ((value >> (shift * 8)) & 0xff) as u8;
                if byte.is_ascii_graphic() || byte == b' ' {
                    char::from(byte)
                } else {
                    '.'
                }
            })
            .collect(),
        _ => format!("{value:09x}"),
    }
}

fn statelist_response(
    project: &Project,
    capture: Option<&Capture>,
    config: Value,
    offset: usize,
    limit: usize,
) -> Result<Value, ToolError> {
    let Some(capture) = capture else {
        return Ok(json!({
            "capture_id": Value::Null,
            "config": config,
            "format": project.settings.options.statelist_format,
            "rows": [],
            "next_cursor": Value::Null
        }));
    };
    let relative = config
        .get("relative")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let column_formats = config.get("column_formats").and_then(Value::as_object);
    let data_format = column_formats
        .and_then(|formats| formats.get("data"))
        .and_then(Value::as_str)
        .unwrap_or(&project.settings.options.statelist_format);
    let data_format = validate_statelist_format(data_format)?;
    let mut sample = 0_u64;
    let rows = capture
        .runs
        .iter()
        .enumerate()
        .filter_map(|(index, run)| {
            let start = sample;
            sample = sample.saturating_add(run.count);
            (index >= offset && index < offset.saturating_add(limit)).then(|| {
                let relative_sample = i128::from(start) - i128::from(capture.reference_sample);
                json!({
                    "index": index,
                    "sample": start,
                    "display_sample": if relative { json!(relative_sample) } else { json!(start) },
                    "time_s": start as f64 * capture.sample_period_s,
                    "display_time_s": if relative {
                        relative_sample as f64 * capture.sample_period_s
                    } else {
                        start as f64 * capture.sample_period_s
                    },
                    "data": run.data,
                    "formatted_data": statelist_data(run.data, data_format),
                    "count": run.count
                })
            })
        })
        .collect::<Vec<_>>();
    let next_cursor = (offset.saturating_add(rows.len()) < capture.runs.len()).then(|| {
        format!(
            "{}:statelist:{}",
            capture.id,
            offset.saturating_add(rows.len())
        )
    });
    Ok(json!({
        "capture_id": capture.id,
        "config": config,
        "format": project.settings.options.statelist_format,
        "rows": rows,
        "next_cursor": next_cursor
    }))
}
fn required_bool(params: &Value, keys: &[&str]) -> Result<bool, ToolError> {
    first_param(params, keys)
        .and_then(Value::as_bool)
        .ok_or_else(|| tool_error("INVALID_ARG", format!("{} must be boolean", keys[0])))
}
fn optional_bool(params: &Value, keys: &[&str]) -> Result<Option<bool>, ToolError> {
    first_param(params, keys)
        .map(|_| required_bool(params, keys))
        .transpose()
}
fn required_nonempty_str<'a>(params: &'a Value, keys: &[&str]) -> Result<&'a str, ToolError> {
    required_str(params, keys).and_then(|value| {
        (!value.is_empty())
            .then_some(value)
            .ok_or_else(|| tool_error("INVALID_ARG", format!("{} must not be empty", keys[0])))
    })
}
fn optional_nonempty_str<'a>(
    params: &'a Value,
    keys: &[&str],
) -> Result<Option<&'a str>, ToolError> {
    first_param(params, keys)
        .map(|_| required_nonempty_str(params, keys))
        .transpose()
}
fn required_u64(params: &Value, keys: &[&str]) -> Result<u64, ToolError> {
    first_param(params, keys)
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            tool_error(
                "INVALID_ARG",
                format!("{} must be a non-negative integer", keys[0]),
            )
        })
}
fn required_i64(params: &Value, keys: &[&str]) -> Result<i64, ToolError> {
    first_param(params, keys)
        .and_then(Value::as_i64)
        .ok_or_else(|| tool_error("INVALID_ARG", format!("{} must be an integer", keys[0])))
}
fn optional_i64(params: &Value, keys: &[&str]) -> Result<Option<i64>, ToolError> {
    first_param(params, keys)
        .map(|_| required_i64(params, keys))
        .transpose()
}
fn required_u8(params: &Value, keys: &[&str]) -> Result<u8, ToolError> {
    required_u64(params, keys).and_then(|value| {
        u8::try_from(value)
            .map_err(|_| tool_error("INVALID_ARG", format!("{} exceeds 255", keys[0])))
    })
}
fn required_f64(params: &Value, keys: &[&str]) -> Result<f64, ToolError> {
    first_param(params, keys)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| tool_error("INVALID_ARG", format!("{} must be finite", keys[0])))
}
fn optional_f64<'a>(params: &'a Value, keys: &'a [&'a str]) -> Option<Result<f64, ToolError>> {
    first_param(params, keys).map(|_| required_f64(params, keys))
}
fn optional_u64(params: &Value, keys: &[&str]) -> Result<Option<u64>, ToolError> {
    first_param(params, keys)
        .map(|_| required_u64(params, keys))
        .transpose()
}
fn optional_string(params: &Value, keys: &[&str]) -> Result<Option<String>, ToolError> {
    match first_param(params, keys) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(Value::String(_)) => Err(tool_error(
            "INVALID_ARG",
            format!("{} must not be empty", keys[0]),
        )),
        Some(_) => Err(tool_error(
            "INVALID_ARG",
            format!("{} must be a string or null", keys[0]),
        )),
    }
}
fn validate_export_format(value: &str) -> Result<&str, ToolError> {
    match value {
        "as_formatted" | "csv" | "vcd" | "txt" | "state" | "state_list" | "json" => Ok(value),
        _ => Err(tool_error(
            "INVALID_ARG",
            format!("unsupported export format: {value}"),
        )),
    }
}
fn validate_radix(value: &str) -> Result<&str, ToolError> {
    match value {
        "binary" | "decimal" | "hex" | "ascii" => Ok(value),
        _ => Err(tool_error(
            "INVALID_ARG",
            format!("unsupported export radix: {value}"),
        )),
    }
}
fn export_payload(
    capture: &Capture,
    kind: &str,
    target_path: Option<String>,
) -> Result<Value, ToolError> {
    let normalized = if kind == "as_formatted" { "csv" } else { kind };
    let data = match normalized {
        "csv" => lp_project::export::csv_channel_bits(capture),
        "vcd" => lp_project::export::vcd(capture),
        "txt" | "state" | "state_list" => lp_project::export::state_list(capture),
        "json" => serde_json::to_string_pretty(capture).map_err(json_error)?,
        _ => {
            return Err(tool_error(
                "INVALID_ARG",
                format!("unsupported export kind: {kind}"),
            ));
        }
    };
    Ok(json!({
        "capture_id":capture.id,
        "kind":normalized,
        "target_path":target_path,
        "data":data
    }))
}
fn no_capture() -> ToolError {
    tool_error("NO_CAPTURE", "no capture is available")
}
fn zoom_125(value: f64, zoom_in: bool) -> f64 {
    let exponent = value.log10().floor();
    let decade = 10_f64.powf(exponent);
    let normalized = value / decade;
    if zoom_in {
        if normalized > 5.0 {
            5.0 * decade
        } else if normalized > 2.0 {
            2.0 * decade
        } else if normalized > 1.0 {
            decade
        } else {
            5.0 * decade / 10.0
        }
    } else if normalized < 1.0 {
        decade
    } else if normalized < 2.0 {
        2.0 * decade
    } else if normalized < 5.0 {
        5.0 * decade
    } else {
        10.0 * decade
    }
}
fn view_location(project: &Project, capture: &Capture) -> u64 {
    add_signed_sample(
        capture.reference_sample,
        project.view.reference_offset_samples,
        capture.expanded_len(),
    )
}
fn set_view_location(project: &mut Project, capture: &Capture, sample: u64) {
    let sample = sample.min(capture.expanded_len().saturating_sub(1));
    let offset = i128::from(sample) - i128::from(capture.reference_sample);
    project.view.reference_offset_samples =
        offset.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
}
fn add_signed_sample(sample: u64, delta: i64, len: u64) -> u64 {
    let last = len.saturating_sub(1);
    (i128::from(sample) + i128::from(delta)).clamp(0, i128::from(last)) as u64
}
fn scroll_delta_samples(
    params: &Value,
    capture: Option<&Capture>,
    project: &Project,
) -> Result<i64, ToolError> {
    if first_param(params, &["samples", "delta_samples"]).is_some() {
        return required_i64(params, &["samples", "delta_samples"]);
    }
    let pixels = required_f64(params, &["pixels", "delta_px"])?;
    let capture = capture.ok_or_else(no_capture)?;
    let samples = pixels * project.view.scale_s_per_px / capture.sample_period_s;
    if samples < i64::MIN as f64 || samples > i64::MAX as f64 {
        return Err(tool_error("INVALID_ARG", "scroll delta is out of range"));
    }
    Ok(samples.round() as i64)
}
fn set_sample_rate(
    sample: &mut lp_project::SampleSettings,
    hz: u64,
    extended_rates: bool,
) -> Result<(), ToolError> {
    let entry = lp_proto::encode::rate::RATES
        .iter()
        .find(|entry| entry.hz == hz && (entry.vendor_ui || extended_rates))
        .ok_or_else(|| tool_error("INVALID_ARG", format!("unsupported sample rate: {hz}")))?;
    sample.rate_index = entry.idx;
    sample.rate_hz = entry.hz;
    Ok(())
}
fn param_usize(params: &Value, key: &str, default: usize, max: usize) -> Result<usize, ToolError> {
    let value = params.get(key).map_or(Ok(default), |value| {
        value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                tool_error(
                    "INVALID_ARG",
                    format!("{key} must be a non-negative integer"),
                )
            })
    })?;
    if value > max {
        return Err(tool_error(
            "INVALID_ARG",
            format!("{key} exceeds maximum {max}"),
        ));
    }
    Ok(value)
}
fn parse_channel(value: Option<&Value>) -> Result<u8, ToolError> {
    let channel = match value {
        None => 0,
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .ok_or_else(|| tool_error("INVALID_ARG", "source channel is invalid"))?,
        Some(Value::String(name)) if name.eq_ignore_ascii_case("clk1") => 32,
        Some(Value::String(name)) if name.eq_ignore_ascii_case("clk2") => 33,
        Some(Value::String(name)) => name
            .strip_prefix('D')
            .or_else(|| name.strip_prefix('d'))
            .and_then(|v| v.parse::<u8>().ok())
            .ok_or_else(|| {
                tool_error(
                    "INVALID_ARG",
                    "source must be D0..D31, CLK1, CLK2, or channel number",
                )
            })?,
        _ => return Err(tool_error("INVALID_ARG", "source channel is invalid")),
    };
    (channel < 34)
        .then_some(channel)
        .ok_or_else(|| tool_error("INVALID_ARG", "source channel is outside D0..D31/CLK1/CLK2"))
}
fn channel_name(channel: u8) -> String {
    match channel {
        32 => "CLK1".to_owned(),
        33 => "CLK2".to_owned(),
        _ => format!("D{channel}"),
    }
}
fn measurement_slot_index(params: &Value) -> Result<usize, ToolError> {
    let slot = required_u8(params, &["slot", "index"])?;
    (slot < 4)
        .then_some(usize::from(slot))
        .ok_or_else(|| tool_error("INVALID_ARG", "measurement slot must be in 0..3"))
}
fn measurement_kind_name(value: &str) -> Result<String, ToolError> {
    serde_json::from_value::<MeasurementKind>(Value::String(value.to_ascii_lowercase()))
        .map_err(|_| tool_error("INVALID_ARG", format!("unknown measurement type: {value}")))
        .and_then(|kind| serde_json::to_value(kind).map_err(json_error))
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| tool_error("INTERNAL", "measurement type did not serialize"))
        })
}
fn measurement_reference_name(value: &str) -> Result<String, ToolError> {
    let normalized = value.to_ascii_lowercase();
    if matches!(normalized.as_str(), "trigger" | "reference") {
        return Ok(normalized);
    }
    let id = cursor_id_from_suffix(value)?;
    Ok(id.to_string())
}
fn validate_measurement_slot(
    slot: &lp_project::MeasurementSlot,
    index: usize,
) -> Result<(), ToolError> {
    if usize::from(slot.slot) != index {
        return Err(tool_error(
            "INVALID_ARG",
            format!("measurement slot {index} must have slot={index}"),
        ));
    }
    measurement_kind_name(&slot.kind)?;
    measurement_reference_name(&slot.left)?;
    measurement_reference_name(&slot.right)?;
    parse_channel(Some(&Value::String(slot.source.clone())))?;
    Ok(())
}
fn patch_measurement_slot(
    slot: &mut lp_project::MeasurementSlot,
    params: &Value,
) -> Result<(), ToolError> {
    if let Some(value) = first_param(params, &["type", "kind"]) {
        slot.kind = measurement_kind_name(
            value
                .as_str()
                .ok_or_else(|| tool_error("INVALID_ARG", "type must be a string"))?,
        )?;
    }
    if let Some(value) = params.get("left") {
        slot.left = measurement_reference_name(
            value
                .as_str()
                .ok_or_else(|| tool_error("INVALID_ARG", "left must be a string"))?,
        )?;
    }
    if let Some(value) = params.get("right") {
        slot.right = measurement_reference_name(
            value
                .as_str()
                .ok_or_else(|| tool_error("INVALID_ARG", "right must be a string"))?,
        )?;
    }
    if let Some(value) = params.get("source") {
        slot.source = channel_name(parse_channel(Some(value))?);
    }
    Ok(())
}
fn measurement_for_slot(
    project: &Project,
    capture: &Capture,
    slot: &lp_project::MeasurementSlot,
) -> Result<lp_project::Measurement, ToolError> {
    let kind = serde_json::from_value::<MeasurementKind>(Value::String(slot.kind.clone()))
        .map_err(|_| {
            tool_error(
                "INVALID_ARG",
                format!("unknown measurement type: {}", slot.kind),
            )
        })?;
    let channel = parse_channel(Some(&Value::String(slot.source.clone())))?;
    let left = measurement_reference(project, capture, &slot.left)?;
    let right = measurement_reference(project, capture, &slot.right)?;
    lp_project::measure(capture, kind, channel, left.min(right), left.max(right))
        .map_err(capture_error)
}
fn measurement_reference(
    project: &Project,
    capture: &Capture,
    reference: &str,
) -> Result<u64, ToolError> {
    match reference.to_ascii_lowercase().as_str() {
        "trigger" => Ok(capture.trigger_sample),
        "reference" => Ok(capture.reference_sample),
        value if value.len() == 1 => {
            let id = value
                .chars()
                .next()
                .map(|id| id.to_ascii_uppercase())
                .ok_or_else(|| tool_error("INVALID_ARG", "measurement reference is empty"))?;
            let cursor = project
                .cursors
                .iter()
                .find(|cursor| cursor.id == id)
                .ok_or_else(|| {
                    tool_error(
                        "INVALID_ARG",
                        format!("unknown measurement reference: {reference}"),
                    )
                })?;
            let position = i128::from(capture.reference_sample) + i128::from(cursor.offset_samples);
            Ok(position.clamp(0, i128::from(capture.expanded_len())) as u64)
        }
        _ => Err(tool_error(
            "INVALID_ARG",
            format!("unknown measurement reference: {reference}"),
        )),
    }
}
fn parse_wires_param(params: &Value) -> Result<Vec<u8>, ToolError> {
    let values = params
        .get("wires")
        .and_then(Value::as_array)
        .ok_or_else(|| tool_error("INVALID_ARG", "wires must be an array"))?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_u64()
                .and_then(|wire| u8::try_from(wire).ok())
                .filter(|wire| *wire < 34)
                .ok_or_else(|| {
                    tool_error("INVALID_ARG", format!("wires[{index}] must be in 0..33"))
                })
        })
        .collect()
}
fn group_from_params(
    params: &Value,
    groups: &[lp_project::Group],
) -> Result<lp_project::Group, ToolError> {
    if let Some(group) = params.get("group") {
        let group = serde_json::from_value(group.clone())
            .map_err(|error| tool_error("INVALID_ARG", error.to_string()))?;
        validate_group(&group)?;
        return Ok(group);
    }
    let group = lp_project::Group {
        id: params
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| next_group_id(groups)),
        name: required_nonempty_str(params, &["name"])?.to_owned(),
        wires: params
            .get("wires")
            .map(|_| parse_wires_param(params))
            .transpose()?
            .unwrap_or_default(),
        radix: param_str(params, "radix").unwrap_or("hex").to_owned(),
        signed: params
            .get("signed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        wire_order: param_str(params, "wire_order")
            .unwrap_or("msb_first")
            .to_owned(),
        display_order: param_str(params, "display_order")
            .unwrap_or("high_top")
            .to_owned(),
        style: param_str(params, "style").unwrap_or("digital").to_owned(),
        color: param_str(params, "color").unwrap_or("default").to_owned(),
        lpf_raw: None,
    };
    validate_group(&group)?;
    Ok(group)
}
fn patch_group(group: &mut lp_project::Group, params: &Value) -> Result<(), ToolError> {
    if let Some(name) = optional_nonempty_str(params, &["name"])? {
        group.name = name.to_owned();
    }
    if params.get("wires").is_some() {
        group.wires = parse_wires_param(params)?;
    }
    if let Some(radix) = optional_nonempty_str(params, &["radix"])? {
        group.radix = radix.to_owned();
    }
    if let Some(signed) = optional_bool(params, &["signed"])? {
        group.signed = signed;
    }
    if let Some(order) = optional_nonempty_str(params, &["wire_order"])? {
        group.wire_order = order.to_owned();
    }
    if let Some(order) = optional_nonempty_str(params, &["display_order"])? {
        group.display_order = order.to_owned();
    }
    if let Some(style) = optional_nonempty_str(params, &["style"])? {
        group.style = style.to_owned();
    }
    if let Some(color) = optional_nonempty_str(params, &["color"])? {
        group.color = color.to_owned();
    }
    Ok(())
}
fn validate_group(group: &lp_project::Group) -> Result<(), ToolError> {
    if group.id.is_empty() || group.name.is_empty() {
        return Err(tool_error(
            "INVALID_ARG",
            "group id and name must not be empty",
        ));
    }
    if !matches!(group.radix.as_str(), "binary" | "decimal" | "hex" | "ascii") {
        return Err(tool_error("INVALID_ARG", "unsupported group radix"));
    }
    if !matches!(group.style.as_str(), "digital" | "analog") {
        return Err(tool_error(
            "INVALID_ARG",
            "group style must be digital or analog",
        ));
    }
    if group.wire_order.is_empty() || group.display_order.is_empty() || group.color.is_empty() {
        return Err(tool_error(
            "INVALID_ARG",
            "group order and color fields must not be empty",
        ));
    }
    let mut seen = [false; 34];
    for wire in &group.wires {
        let slot = seen
            .get_mut(usize::from(*wire))
            .ok_or_else(|| tool_error("INVALID_ARG", "group wire must be in 0..33"))?;
        if *slot {
            return Err(tool_error("INVALID_ARG", "group wires must be unique"));
        }
        *slot = true;
    }
    Ok(())
}
fn ensure_unique_group(
    groups: &[lp_project::Group],
    group: &lp_project::Group,
    except: Option<usize>,
) -> Result<(), ToolError> {
    if groups.iter().enumerate().any(|(index, existing)| {
        Some(index) != except && (existing.id == group.id || existing.name == group.name)
    }) {
        return Err(tool_error(
            "INVALID_ARG",
            "group id and name must be unique",
        ));
    }
    Ok(())
}
fn group_index(groups: &[lp_project::Group], id: &str) -> Result<usize, ToolError> {
    groups
        .iter()
        .position(|group| group.id == id)
        .ok_or_else(|| tool_error("UNKNOWN_GROUP", format!("unknown group {id}")))
}
fn next_group_id(groups: &[lp_project::Group]) -> String {
    (1_u64..)
        .map(|index| format!("group-{index}"))
        .find(|candidate| groups.iter().all(|group| group.id != *candidate))
        .unwrap_or_else(|| "group".to_owned())
}
fn row_from_params(
    params: &Value,
    kind: &str,
    project: &Project,
) -> Result<lp_project::Row, ToolError> {
    let reference = match kind {
        "signal" => {
            let wire = required_u8(params, &["wire", "channel"])?;
            project
                .signals
                .get(usize::from(wire))
                .filter(|signal| signal.wire == wire)
                .map(|signal| signal.wire_name.clone())
                .ok_or_else(|| tool_error("INVALID_ARG", "wire must be in 0..33"))?
        }
        "group" => {
            let id = required_str(params, &["group_id", "reference", "ref"])?;
            group_index(&project.groups, id)?;
            id.to_owned()
        }
        "interpreter" => {
            let id = required_str(params, &["interpreter_id", "reference", "ref"])?;
            if !project
                .interpreters
                .iter()
                .any(|interpreter| interpreter.id == id)
            {
                return Err(tool_error(
                    "UNKNOWN_INTERPRETER",
                    format!("unknown interpreter {id}"),
                ));
            }
            id.to_owned()
        }
        _ => return Err(tool_error("INTERNAL", "unknown row kind")),
    };
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| next_row_id(&project.rows));
    let mut row = default_row(id, kind, reference);
    if let Some(parent) = params.get("parent") {
        row.parent = match parent {
            Value::Null => None,
            Value::String(value) => Some(value.clone()),
            _ => return Err(tool_error("INVALID_ARG", "parent must be a row id or null")),
        };
    }
    if let Some(height) = params.get("height_px") {
        let height = height
            .as_u64()
            .filter(|height| (8..=4096).contains(height))
            .ok_or_else(|| tool_error("INVALID_ARG", "row height must be 8..4096 px"))?;
        row.height_px = height as u32;
    }
    Ok(row)
}
fn default_row(id: String, kind: &str, reference: String) -> lp_project::Row {
    lp_project::Row {
        id,
        kind: kind.to_owned(),
        reference,
        parent: None,
        height_px: 24,
        color_index: 0,
        style: "digital".to_owned(),
        color: "default".to_owned(),
        expanded: true,
        visible: true,
    }
}
fn next_row_id(rows: &[lp_project::Row]) -> String {
    (1_u64..)
        .map(|index| format!("row-{index}"))
        .find(|candidate| rows.iter().all(|row| row.id != *candidate))
        .unwrap_or_else(|| "row".to_owned())
}
fn ensure_unique_row(rows: &[lp_project::Row], id: &str) -> Result<(), ToolError> {
    if id.is_empty() {
        return Err(tool_error("INVALID_ARG", "row id must not be empty"));
    }
    if rows.iter().any(|row| row.id == id) {
        return Err(tool_error("INVALID_ARG", format!("duplicate row id {id}")));
    }
    Ok(())
}
fn row_index(rows: &[lp_project::Row], id: &str) -> Result<usize, ToolError> {
    rows.iter()
        .position(|row| row.id == id)
        .ok_or_else(|| tool_error("UNKNOWN_ROW", format!("unknown row {id}")))
}
fn sync_row_style(project: &mut Project, row_index: usize, style: &str) {
    let kind = project.rows[row_index].kind.clone();
    let reference = project.rows[row_index].reference.clone();
    if kind == "group" {
        if let Some(group) = project
            .groups
            .iter_mut()
            .find(|group| group.id == reference)
        {
            group.style = style.to_owned();
        }
    } else if kind == "interpreter"
        && let Some(interpreter) = project
            .interpreters
            .iter_mut()
            .find(|interpreter| interpreter.id == reference)
    {
        interpreter.style = style.to_owned();
    }
}
fn sync_row_color(project: &mut Project, row_index: usize, color: &str) {
    let kind = project.rows[row_index].kind.clone();
    let reference = project.rows[row_index].reference.clone();
    if kind == "group" {
        if let Some(group) = project
            .groups
            .iter_mut()
            .find(|group| group.id == reference)
        {
            group.color = color.to_owned();
        }
    } else if kind == "interpreter"
        && let Some(interpreter) = project
            .interpreters
            .iter_mut()
            .find(|interpreter| interpreter.id == reference)
    {
        interpreter.color = color.to_owned();
    }
}
// Expand one channel of an RLE capture into a per-sample level sequence.
fn channel_levels(capture: &Capture, wire: u8) -> Vec<bool> {
    let mut levels = Vec::with_capacity(capture.expanded_len() as usize);
    for run in &capture.runs {
        let high = (run.data >> wire) & 1 == 1;
        for _ in 0..run.count {
            levels.push(high);
        }
    }
    levels
}
fn decode_i2c_frames(scl: &[bool], sda: &[bool]) -> Vec<Value> {
    lp_proto::decode::decode_i2c(scl, sda)
        .iter()
        .map(|event| match event {
            lp_proto::decode::I2cEvent::Start => json!({ "text": "START" }),
            lp_proto::decode::I2cEvent::Byte { value, ack } => {
                json!({ "text": format!("0x{value:02X} {}", if *ack { "ACK" } else { "NAK" }) })
            }
            lp_proto::decode::I2cEvent::Stop => json!({ "text": "STOP" }),
        })
        .collect()
}
fn decode_onewire_frames(line: &[bool], rate: u64) -> Vec<Value> {
    lp_proto::decode::decode_onewire(line, rate)
        .iter()
        .map(|event| match event {
            lp_proto::decode::OneWireEvent::Reset => json!({ "text": "RESET" }),
            lp_proto::decode::OneWireEvent::Byte(value) => json!({ "text": format!("0x{value:02X}") }),
        })
        .collect()
}
fn decode_spi_frames(clock: &[bool], data: &[bool]) -> Vec<Value> {
    lp_proto::decode::decode_spi(clock, data, &lp_proto::decode::SpiConfig::mode0_8bit())
        .iter()
        .map(|word| json!({ "text": format!("0x{word:02X}") }))
        .collect()
}
fn decode_uart_frames(line: &[bool], rate: u64, baud: u32) -> Vec<Value> {
    lp_proto::decode::decode_async_serial(line, &lp_proto::decode::AsyncSerialConfig::uart_8n1(rate, baud))
        .iter()
        .map(|byte| json!({ "text": format!("0x{:02X}", byte.value), "start_sample": byte.start_sample }))
        .collect()
}
fn decode_can_frames(line: &[bool], rate: u64, bitrate: u32) -> Vec<Value> {
    lp_proto::decode::decode_can(line, rate, bitrate)
        .iter()
        .map(|frame| {
            let data = frame
                .data
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let payload = if data.is_empty() {
                String::new()
            } else {
                format!(" [{data}]")
            };
            let crc = if frame.crc_ok { "" } else { " CRC!" };
            json!({ "text": format!("ID=0x{:03X} DLC={}{payload}{crc}", frame.id, frame.dlc) })
        })
        .collect()
}
fn decode_parallel_frames(clock: &[bool], data: &[Vec<bool>]) -> Vec<Value> {
    let refs: Vec<&[bool]> = data.iter().map(Vec::as_slice).collect();
    lp_proto::decode::decode_parallel(clock, &refs, true)
        .iter()
        .map(|word| json!({ "text": format!("0x{word:X}") }))
        .collect()
}
fn decode_iso7816_frames(line: &[bool], rate: u64, baud: u32, inverse: bool) -> Vec<Value> {
    let convention = if inverse {
        lp_proto::decode::Iso7816Convention::Inverse
    } else {
        lp_proto::decode::Iso7816Convention::Direct
    };
    lp_proto::decode::decode_iso7816(line, rate, baud, convention)
        .iter()
        .map(|byte| json!({ "text": format!("0x{:02X}", byte.value), "start_sample": byte.start_sample }))
        .collect()
}

fn group_value_at(
    project: &Project,
    capture: &Capture,
    group_id: &str,
    sample: u64,
) -> Result<Value, ToolError> {
    let group = project
        .groups
        .iter()
        .find(|group| group.id == group_id)
        .ok_or_else(|| tool_error("UNKNOWN_GROUP", format!("unknown group {group_id}")))?;
    let data = capture
        .sample_at(sample)
        .ok_or_else(|| tool_error("INVALID_ARG", "sample is outside capture"))?;
    let wires = if group.wire_order == "lsb_first" {
        group.wires.to_vec()
    } else {
        group.wires.iter().rev().copied().collect::<Vec<_>>()
    };
    let raw = wires.iter().enumerate().fold(0_u64, |value, (bit, wire)| {
        value | (((data >> wire) & 1) << bit)
    });
    let width = wires.len().min(64);
    let formatted = format_group_value(raw, width, group);
    Ok(json!({
        "group_id":group.id,
        "sample":sample,
        "value":raw,
        "formatted":formatted,
        "radix":group.radix,
        "signed":group.signed
    }))
}
fn format_group_value(raw: u64, width: usize, group: &lp_project::Group) -> String {
    match group.radix.as_str() {
        "binary" => format!("0b{raw:0width$b}"),
        "hex" => format!("0x{raw:0digits$x}", digits = width.div_ceil(4)),
        "ascii" => {
            let bytes = width.div_ceil(8);
            (0..bytes)
                .map(|index| ((raw >> (index * 8)) & 0xff) as u8)
                .map(|byte| {
                    if byte.is_ascii_graphic() || byte == b' ' {
                        char::from(byte)
                    } else {
                        '.'
                    }
                })
                .collect()
        }
        _ if group.signed && width > 0 && width < 64 && raw & (1_u64 << (width - 1)) != 0 => {
            (i128::from(raw) - (1_i128 << width)).to_string()
        }
        _ if group.signed && width == 64 => (raw as i64).to_string(),
        _ => raw.to_string(),
    }
}
fn capture_error(error: lp_project::CaptureError) -> ToolError {
    tool_error("INVALID_ARG", error.to_string())
}
fn store_error(error: lp_project::StoreError) -> ToolError {
    match error {
        lp_project::StoreError::UnknownCapture(id) => {
            tool_error("UNKNOWN_CAPTURE", format!("unknown capture {id}"))
        }
        other => tool_error("INTERNAL", other.to_string()),
    }
}

#[derive(Debug)]
pub struct ApiError(pub(crate) ToolError);
#[derive(Serialize)]
struct ErrorEnvelope {
    error: ToolError,
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            status_for(&self.0.code),
            Json(ErrorEnvelope { error: self.0 }),
        )
            .into_response()
    }
}
fn status_for(code: &str) -> StatusCode {
    match code {
        "INVALID_ARG" => StatusCode::BAD_REQUEST,
        c if c.starts_with("UNKNOWN_") => StatusCode::NOT_FOUND,
        "LEASE_REQUIRED" | "LEASE_HELD" => StatusCode::LOCKED,
        "DEVICE_NOT_CONNECTED"
        | "DEVICE_GONE"
        | "DEVICE_NEEDS_REPLUG"
        | "ACQ_BUSY"
        | "ACQ_NOT_RUNNING"
        | "ACQ_TIMEOUT" => StatusCode::CONFLICT,
        "DEVICE_RECOVERING" => StatusCode::SERVICE_UNAVAILABLE,
        "RANGE_TOO_LARGE" => StatusCode::RANGE_NOT_SATISFIABLE,
        "USB_ERROR" | "FPGA_CONFIG_FAILED" => StatusCode::BAD_GATEWAY,
        "SCREENSHOT_UNAVAILABLE" | "STIMULUS_UNAVAILABLE" => StatusCode::SERVICE_UNAVAILABLE,
        "NOT_SUPPORTED" => StatusCode::NOT_IMPLEMENTED,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
fn tool_error(code: &str, message: impl Into<String>) -> ToolError {
    ToolError::new(code, message)
}
pub(crate) fn api_error(code: &str, message: impl Into<String>) -> ApiError {
    ApiError(tool_error(code, message))
}
fn json_error(error: serde_json::Error) -> ToolError {
    tool_error("INTERNAL", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, header},
    };
    use tower::ServiceExt;

    async fn request(method: &str, uri: &str, body: &str) -> (StatusCode, Value) {
        let response = router(AppState::new())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_owned()))
                    .unwrap_or_else(|e| panic!("{e}")),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{e}")),
        )
    }

    // Like `request`, but drives a caller-supplied state so device-state
    // transitions (e.g. needs_replug) can be exercised end to end over HTTP.
    async fn send(state: AppState, method: &str, uri: &str, body: &str) -> (StatusCode, Value) {
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_owned()))
                    .unwrap_or_else(|e| panic!("{e}")),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{e}")),
        )
    }

    #[test]
    fn is_command_wedge_matches_dead_channel_not_benign_framing() {
        // Dead FT245 device->host FIFO / stale usbfs endpoint -> wedge.
        assert!(is_command_wedge(
            "timed out waiting for 3 protocol bytes; got 0"
        ));
        assert!(is_command_wedge(
            "capture failed: bulk I/O failed: transfer was cancelled"
        ));
        // A partial read still returned bytes -> the channel is alive.
        assert!(!is_command_wedge(
            "timed out waiting for 3 protocol bytes; got 2"
        ));
        // Framing errors the link already self-heals, or a plain absence.
        assert!(!is_command_wedge(
            "unexpected response opcode: expected c2, got c1"
        ));
        assert!(!is_command_wedge("packet number mismatch"));
        assert!(!is_command_wedge("LogicPort 0403:dc48 is not attached"));
    }

    #[tokio::test]
    async fn needs_replug_is_surfaced_on_health_and_blocks_acquire() {
        let state = AppState::real_pending("test hardware unavailable");
        assert!(!state.needs_replug());

        // Software recovery exhausted while enumerated -> ask for a replug.
        state.require_replug(REPLUG_HINT);
        assert!(state.needs_replug());

        // /api/health tells the user/agent exactly what to do.
        let (status, health) = send(state.clone(), "GET", "/api/health", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(health["device"], "needs_replug");
        assert_eq!(health["needs_replug"], true);
        assert_eq!(health["hint"], REPLUG_HINT);

        // Acquire refuses with an actionable message, never an opaque 502.
        let (status, body) = send(
            state.clone(),
            "POST",
            "/api/acquire",
            "{\"mode\":\"single\"}",
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], "DEVICE_NEEDS_REPLUG");
        assert_eq!(body["error"]["message"], REPLUG_HINT);
    }

    #[tokio::test]
    async fn healthy_device_is_not_flagged_for_replug() {
        let (status, health) = request("GET", "/api/health", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(health["needs_replug"], false);
        assert!(health.get("hint").is_none());
    }

    #[tokio::test]
    async fn health_and_registry_contract() {
        let (status, health) = request("GET", "/api/health", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(health["ok"], true);
        let (status, operations) = request("GET", "/api/ops", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(operations.as_array().map(Vec::len), Some(459));
    }

    #[tokio::test]
    async fn pending_real_backend_is_live_and_truthfully_disconnected() {
        let state = AppState::real_pending("test hardware unavailable");
        let device = ops::dispatch(state.inner.as_ref(), "device.status", json!({}))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(device["state"], "disconnected");
        assert_eq!(device["fpga"], "unavailable");
        assert_eq!(device["error"], "test hardware unavailable");
        assert!(state.check_real_connection().is_err());
        state
            .disconnect_real("test connection lost")
            .unwrap_or_else(|error| panic!("{error}"));
        let device = ops::dispatch(state.inner.as_ref(), "device.status", json!({}))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(device["state"], "disconnected");
        assert_eq!(device["error"], "test connection lost");

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("{error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let health: Value =
            serde_json::from_slice(&bytes).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(health["ok"], true);
        assert_eq!(health["device"], "disconnected");
    }

    #[test]
    fn file_lifecycle_roundtrips_native_and_treats_lpf_as_import_only() {
        let unique = format!(
            "stimulus-file-ops-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&directory).unwrap_or_else(|error| panic!("{error}"));
        let native = directory.join("roundtrip.lpj");
        let state = AppState::new();
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };

        call("notes.set", json!({"notes":"saved notes"}));
        assert_eq!(call("file.save_as", json!({"path":native}))["saved"], true);
        call("notes.set", json!({"notes":"changed notes"}));
        call("file.save", json!({}));
        call("file.new", json!({}));
        assert_eq!(call("notes.get", json!({}))["notes"], "");
        call("file.open", json!({"path":native}));
        assert_eq!(call("notes.get", json!({}))["notes"], "changed notes");
        assert_eq!(
            call("file.recent.list", json!({}))["recent"][0],
            native.to_string_lossy().as_ref()
        );
        call("file.close", json!({}));
        assert_eq!(call("file.readonly.get", json!({}))["path"], Value::Null);

        let lpf = FsPath::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/vendor/examples/Quickstart.LPF");
        let imported = call("file.open", json!({"path":lpf}));
        assert_eq!(imported["imported"], true);
        assert_eq!(imported["save_requires_path"], true);
        assert_eq!(
            state
                .captures()
                .list(2)
                .unwrap_or_else(|error| panic!("{error}"))
                .len(),
            1
        );
        let error = ops::dispatch(state.inner.as_ref(), "file.save", json!({}))
            .err()
            .unwrap_or_else(|| panic!("LPF import must require save_as"));
        assert_eq!(error.code, "PATH_REQUIRED");

        let invalid = ops::dispatch(
            state.inner.as_ref(),
            "file.save_as",
            json!({"path":directory.join("invalid.lpf")}),
        )
        .err()
        .unwrap_or_else(|| panic!("native save must reject non-LPJ extension"));
        assert_eq!(invalid.code, "INVALID_ARG");
        state
            .inner
            .project
            .write()
            .unwrap_or_else(|_| panic!("project lock poisoned"))
            .read_only = true;
        let read_only = ops::dispatch(
            state.inner.as_ref(),
            "file.save_as",
            json!({"path":directory.join("readonly.lpj")}),
        )
        .err()
        .unwrap_or_else(|| panic!("read-only project must reject save"));
        assert_eq!(read_only.code, "READ_ONLY");
        assert_eq!(call("file.exit", json!({}))["exit_requested"], true);
        assert!(state.inner.exit_requested.load(Ordering::Acquire));

        std::fs::remove_dir_all(&directory).unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn sample_operation_family_mutates_shared_setup() {
        let state = AppState::new();
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };
        assert_eq!(
            call("sample.mode.set", json!({"mode":"state"}))["sample"]["mode"],
            "state"
        );
        assert_eq!(
            call("sample.rate.step_up", json!({}))["sample"]["rate_hz"],
            20_000_000
        );
        assert_eq!(
            call("sample.rate.step_down", json!({}))["sample"]["rate_hz"],
            10_000_000
        );
        for (id, params) in [
            ("sample.rate.set", json!({"rate_hz":50_000_000})),
            ("sample.rate.units.set", json!({"units":"hz"})),
            ("sample.state.clock.set", json!({"clock":1})),
            ("sample.state.edge.set", json!({"edge":"falling"})),
            (
                "sample.state.window.set",
                json!({"window_index":2,"window_ns":4.0}),
            ),
            ("sample.state.qualifier.enable", json!({"enabled":true})),
            ("sample.state.qualifier.polarity", json!({"polarity":"low"})),
            (
                "sample.state.declared_rate.set",
                json!({"rate_hz":10_000_000}),
            ),
            ("sample.state.declared_units.set", json!({"units":"hz"})),
            ("sample.compression.set", json!({"enabled":true})),
            ("sample.prefill_timeout.set", json!({"index":2,"ms":1000})),
            ("sample.postfill_timeout.set", json!({"index":3,"ms":2000})),
            ("sample.pretrigger_buffer.set", json!({"percent":25.0})),
        ] {
            call(id, params);
        }
        let sample = call("sample.get", json!({}))["sample"].clone();
        assert_eq!(sample["rate_hz"], 50_000_000);
        assert_eq!(sample["state"]["clock"], 1);
        assert_eq!(sample["state"]["edge"], "falling");
        assert_eq!(sample["state"]["qualifier"]["enabled"], true);
        assert_eq!(sample["compression"], true);
        assert_eq!(sample["pretrigger_pct"], 25.0);
        assert_eq!(call("sample.dialog.open", json!({}))["open"], true);

        let result = ops::dispatch(
            state.inner.as_ref(),
            "sample.pretrigger_buffer.set",
            json!({"percent":101.0}),
        );
        let Err(error) = result else {
            panic!("out-of-range pretrigger must fail");
        };
        assert_eq!(error.code, "INVALID_ARG");
    }

    #[test]
    fn threshold_and_logic_sense_operations_validate_and_persist() {
        let state = AppState::new();
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {}", error.message))
        };
        assert_eq!(
            call("threshold.set", json!({"volts":1.31}))["threshold_v"],
            1.3
        );
        assert_eq!(call("threshold.step_up", json!({}))["threshold_v"], 1.35);
        assert_eq!(call("threshold.step_down", json!({}))["threshold_v"], 1.3);
        assert_eq!(
            call("logicsense.set", json!({"channel":33,"inverted":true}))["inverted"],
            true
        );
        let all = call("logicsense.set_all", json!({"inverted":true}));
        assert_eq!(all["inverted"].as_array().map(Vec::len), Some(34));
        assert!(
            all["inverted"]
                .as_array()
                .is_some_and(|values| { values.iter().all(|value| value == &Value::Bool(true)) })
        );
        let open = call("logicsense.dialog.open", json!({}));
        assert_eq!(open["open"], true);
        let values = (0..34).map(|channel| channel % 2 == 0).collect::<Vec<_>>();
        let applied = call("logicsense.dialog.ok", json!({"inverted":values}));
        assert_eq!(applied["open"], false);
        assert_eq!(call("logicsense.get", json!({}))["inverted"][1], false);

        for (id, params) in [
            ("threshold.set", json!({"volts":6.05})),
            ("logicsense.set", json!({"channel":34,"inverted":true})),
            ("logicsense.dialog.ok", json!({"inverted":[true]})),
        ] {
            let result = ops::dispatch(state.inner.as_ref(), id, params);
            let Err(error) = result else {
                panic!("{id} must reject invalid input");
            };
            assert_eq!(error.code, "INVALID_ARG");
        }
    }

    #[test]
    fn status_operation_family_reads_shared_acquisition_state() {
        let state = AppState::new();
        ops::dispatch(state.inner.as_ref(), "acq.single", json!({}))
            .unwrap_or_else(|error| panic!("{error}"));
        let call = |id: &str| {
            ops::dispatch(state.inner.as_ref(), id, json!({}))
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };
        assert_eq!(call("status.phase.get")["phase"], "ready");
        assert_eq!(call("status.stats.get")["acq_count"], 1);
        assert!(call("status.buffer_indicator.get")["buffer_fill_pct"].as_f64() == Some(100.0));
        assert_eq!(
            call("status.warnings.get")["warnings"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
        let measurements = call("status.measurements.get");
        assert_eq!(measurements["capture_id"], 1);
        assert_eq!(
            measurements["measurements"].as_array().map(Vec::len),
            Some(4)
        );
        let status = call("status.get");
        assert!(
            status["samples"]
                .as_u64()
                .is_some_and(|samples| samples > 0)
        );
        assert_eq!(status["measurements"].as_array().map(Vec::len), Some(4));
    }

    #[test]
    fn canonical_recurring_start_and_wait_use_the_shared_worker() {
        let state = AppState::new();
        let started = ops::dispatch(
            state.inner.as_ref(),
            "acq.recurring.start",
            json!({"max_runs":3}),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(started["recurring"], true);
        let finished = ops::dispatch(
            state.inner.as_ref(),
            "acq.wait",
            json!({"for":"idle","timeout_ms":1000}),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(finished["recurring"], false);
        assert_eq!(finished["acq_count"], 3);
        assert_eq!(
            state.captures().list(10).map(|captures| captures.len()),
            Ok(3)
        );

        for (id, params) in [
            ("acq.recurring.start", json!({"max_runs":0})),
            ("acq.wait", json!({"for":"unknown","timeout_ms":0})),
        ] {
            let result = ops::dispatch(state.inner.as_ref(), id, params);
            let Err(error) = result else {
                panic!("{id} must reject invalid input");
            };
            assert_eq!(error.code, "INVALID_ARG");
        }
    }

    #[test]
    fn application_options_persist_without_touching_offline_hardware() {
        let state = AppState::real_pending("deliberately offline");
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };
        let optimized = call(
            "options.optimization.set",
            json!({"optimization":"reduced_cpu"}),
        );
        assert_eq!(optimized["options"]["optimization"], "reduced_cpu");
        assert_eq!(optimized["hardware_reconfigure"], false);
        assert_eq!(
            call("options.save_on_exit.set", json!({"save_on_exit":false}))["options"]["save_on_exit"],
            false
        );
        assert_eq!(
            call("options.extended_rates.set", json!({"extended_rates":true}))["options"]["extended_rates"],
            true
        );
        let top = call("options.keep_on_top.set", json!({"enabled":true}));
        assert_eq!(top["accepted"], true);
        assert_eq!(top["effective"], false);
        let project = call("project.get", json!({}));
        assert_eq!(project["settings"]["options"]["extended_rates"], true);
    }

    #[test]
    fn export_operations_share_settings_and_canonical_encoders() {
        let state = AppState::new();
        ops::dispatch(state.inner.as_ref(), "acq.single", json!({}))
            .unwrap_or_else(|error| panic!("{error}"));
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };
        assert_eq!(
            call("export.format.set", json!({"format":"vcd"}))["export"]["format"],
            "vcd"
        );
        assert_eq!(
            call("export.radix.set", json!({"radix":"hex"}))["export"]["radix"],
            "hex"
        );
        assert_eq!(
            call("export.target.set", json!({"path":"capture.vcd"}))["export"]["target_path"],
            "capture.vcd"
        );
        assert_eq!(call("export.dialog.open", json!({}))["open"], true);
        let run = call("export.run", json!({}));
        assert_eq!(run["kind"], "vcd");
        assert_eq!(run["target_path"], "capture.vcd");
        assert!(
            run["data"]
                .as_str()
                .is_some_and(|data| data.contains("$timescale"))
        );
        assert_eq!(call("export.txt", json!({}))["kind"], "txt");
        assert_eq!(call("export.json", json!({}))["kind"], "json");
        let closed = call(
            "export.dialog.ok",
            json!({"format":"csv","radix":"binary","target_path":null}),
        );
        assert_eq!(closed["open"], false);
        assert_eq!(closed["export"]["target_path"], Value::Null);
        assert!(
            call("export.run", json!({}))["data"]
                .as_str()
                .is_some_and(|data| data.starts_with("SamplePeriod,"))
        );

        for (id, params) in [
            ("export.format.set", json!({"format":"pdf"})),
            ("export.radix.set", json!({"radix":"octal"})),
            ("export.target.set", json!({"path":""})),
        ] {
            let result = ops::dispatch(state.inner.as_ref(), id, params);
            let Err(error) = result else {
                panic!("{id} must reject invalid input");
            };
            assert_eq!(error.code, "INVALID_ARG");
        }
    }

    #[test]
    fn core_view_operations_persist_without_hardware_io() {
        let state = AppState::real_pending("offline");
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };
        assert_eq!(
            call("view.graticule.toggle", json!({}))["options"]["show_graticule"],
            false
        );
        assert_eq!(
            call("view.show_trigger.toggle", json!({"enabled":false}))["options"]["show_trigger"],
            false
        );
        let cursors = call("view.show_cursors.set", json!({"ids":["A","C","F"]}));
        assert_eq!(cursors["options"]["cursor_qty"], 3);
        assert_eq!(cursors["cursors"][2]["visible"], true);
        assert_eq!(
            call("view.show_cursors.all", json!({}))["options"]["cursor_qty"],
            6
        );
        assert_eq!(
            call("view.cursor_qty.set", json!({"qty":2}))["cursors"][2]["visible"],
            false
        );
        assert_eq!(
            call("view.color_scheme.set", json!({"scheme":"dark"}))["options"]["color_scheme"],
            "dark"
        );
        call("view.alt_background.enable", json!({"enabled":true}));
        let background = call(
            "view.alt_background.adjust",
            json!({"color":"#102030","intensity":0.25}),
        );
        assert_eq!(background["options"]["alt_background"]["color"], "#102030");
        assert_eq!(
            call("view.waveforms_in_front.toggle", json!({}))["options"]["waveforms_in_front"],
            false
        );
        assert_eq!(
            call("view.large_waveforms.toggle", json!({"enabled":true}))["options"]["large_waveforms"],
            true
        );
        call(
            "view.reference_position.set",
            json!({"reference_position":0.25}),
        );
        call("view.scale_relative.set", json!({"enabled":false}));
        call("view.units.set", json!({"units":"samples"}));
        call("view.scale_factor.set", json!({"scale":0.000002}));
        call("view.reference_offset.set", json!({"offset":-12}));
        assert_eq!(
            call("view.panel.statelist", json!({}))["view"]["panel"],
            "statelist"
        );
        assert_eq!(
            call("view.theme.set", json!({"theme":"night"}))["view"]["theme"],
            "night"
        );
        let current = call("view.get", json!({}));
        assert_eq!(current["view"]["reference_offset_samples"], -12);
        assert_eq!(current["options"]["units"], "samples");

        for (id, params) in [
            ("view.cursor_qty.set", json!({"qty":7})),
            ("view.reference_position.set", json!({"position":1.1})),
            ("view.scale_factor.set", json!({"scale":0})),
            ("view.units.set", json!({"units":"yards"})),
        ] {
            let result = ops::dispatch(state.inner.as_ref(), id, params);
            let Err(error) = result else {
                panic!("{id} must reject invalid input");
            };
            assert_eq!(error.code, "INVALID_ARG");
        }
    }

    #[test]
    fn view_navigation_uses_capture_and_cursor_geometry() {
        let state = AppState::new();
        ops::dispatch(state.inner.as_ref(), "acq.single", json!({}))
            .unwrap_or_else(|error| panic!("{error}"));
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };
        let all = call("view.zoom.all", json!({"viewport_px":800}));
        assert!(
            all["view"]["scale_s_per_px"]
                .as_f64()
                .is_some_and(|scale| scale > 0.0)
        );
        assert_eq!(
            call("view.scroll_to.begin", json!({}))["location_sample"],
            0
        );
        assert_eq!(
            call("view.scroll.by", json!({"samples":5}))["location_sample"],
            5
        );
        assert!(
            call("view.scroll.drag", json!({"pixels":2.0}))["location_sample"]
                .as_u64()
                .is_some()
        );
        let before = call("view.get", json!({}))["view"]["scale_s_per_px"]
            .as_f64()
            .unwrap_or_else(|| panic!("missing scale"));
        let after = call("view.zoom.in", json!({}))["view"]["scale_s_per_px"]
            .as_f64()
            .unwrap_or_else(|| panic!("missing scale"));
        assert!(after < before);
        assert_eq!(
            call("view.zoom.at", json!({"sample":10}))["location_sample"],
            10
        );
        let end = call("view.scroll_to.end", json!({}))["location_sample"]
            .as_u64()
            .unwrap_or_else(|| panic!("missing end location"));
        assert!(end > 10);
        assert!(
            call("view.scroll_to.cursor.a", json!({}))["location_sample"]
                .as_u64()
                .is_some()
        );
        call("view.scroll_to.begin", json!({}));
        let next = call("view.next_edge", json!({"source":"D0"}))["location_sample"]
            .as_u64()
            .unwrap_or_else(|| panic!("missing edge location"));
        assert!(next > 0);
        call("view.scroll_to.end", json!({}));
        let previous = call("view.prev_edge.row", json!({"source":"D0"}))["location_sample"]
            .as_u64()
            .unwrap_or_else(|| panic!("missing previous edge"));
        assert!(previous < end);
    }

    #[test]
    fn cursor_operations_persist_geometry_visibility_and_tracking() {
        let state = AppState::real_pending("offline");
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };

        assert_eq!(
            call("cursor.place.a", json!({"offset_samples":100}))["cursors"][0]["offset_samples"],
            100
        );
        call("cursor.place.b", json!({"offset_samples":140}));
        assert_eq!(
            call("cursor.tracking.set.b", json!({"target":"A"}))["cursors"][1]["tracks"],
            "A"
        );
        let moved = call("cursor.drag", json!({"id":"A","delta_samples":25}));
        assert_eq!(moved["cursors"][0]["offset_samples"], 125);
        assert_eq!(moved["cursors"][1]["offset_samples"], 165);

        let interlocked = call("cursor.tracking.interlock_all", json!({"target":"C"}));
        assert_eq!(interlocked["cursors"][0]["tracks"], "C");
        assert_eq!(interlocked["cursors"][2]["tracks"], Value::Null);
        let released = call("cursor.tracking.release_all", json!({}));
        assert!(
            released["cursors"]
                .as_array()
                .is_some_and(|cursors| cursors.iter().all(|cursor| cursor["tracks"].is_null()))
        );

        let placed = call(
            "cursor.place_all",
            json!({"positions":[
                {"offset_samples":0},{"offset_samples":1},{"offset_samples":2},
                {"offset_samples":3},{"offset_samples":4},{"offset_samples":5}
            ]}),
        );
        assert_eq!(placed["cursors"][5]["offset_samples"], 5);
        assert_eq!(call("cursor.snap.toggle", json!({}))["snap"], false);
        assert_eq!(call("cursor.set", json!({"enabled":true}))["cursor_qty"], 6);
        assert_eq!(
            call("cursor.set", json!({"id":"F","enabled":false}))["cursor_qty"],
            5
        );
        assert_eq!(call("cursor.get", json!({}))["show_cursors"], true);

        call("cursor.tracking.set.a", json!({"target":"B"}));
        let result = ops::dispatch(
            state.inner.as_ref(),
            "cursor.tracking.set.b",
            json!({"target":"A"}),
        );
        let Err(error) = result else {
            panic!("cursor tracking cycle must be rejected");
        };
        assert_eq!(error.code, "INVALID_ARG");
    }

    #[test]
    fn measurement_operations_configure_and_compute_all_slots() {
        let state = AppState::new();
        ops::dispatch(state.inner.as_ref(), "acq.single", json!({}))
            .unwrap_or_else(|error| panic!("{error}"));
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };

        assert_eq!(
            call("measure.get", json!({}))["measurements"][0]["slot"]["slot"],
            0
        );
        call(
            "measure.slot.type.set",
            json!({"slot":0,"type":"transitions"}),
        );
        call("measure.slot.left.set", json!({"slot":0,"reference":"A"}));
        call("measure.slot.right.set", json!({"slot":0,"reference":"F"}));
        let configured = call("measure.slot.source.set", json!({"slot":0,"source":"CLK1"}));
        assert_eq!(configured["measurements"][0]["type"], "transitions");
        assert_eq!(configured["measurements"][0]["source"], "CLK1");

        let patched = call(
            "measure.dialog.ok",
            json!({"slot":1,"type":"frequency","left":"trigger","right":"reference","source":"D0"}),
        );
        assert_eq!(patched["open"], false);
        assert_eq!(patched["measurements"][1]["type"], "frequency");
        assert_eq!(call("measure.dialog.open", json!({}))["open"], true);
        assert_eq!(
            call("measure.compute", json!({"slot":1}))["slot"]["slot"],
            1
        );
        assert_eq!(
            call("measure.panel.click", json!({"slot":1}))["selected_slot"],
            1
        );

        for (id, params) in [
            ("measure.slot.type.set", json!({"slot":4,"type":"period"})),
            ("measure.slot.type.set", json!({"slot":0,"type":"bogus"})),
            ("measure.slot.left.set", json!({"slot":0,"left":"G"})),
            ("measure.slot.source.set", json!({"slot":0,"source":"D34"})),
        ] {
            let result = ops::dispatch(state.inner.as_ref(), id, params);
            let Err(error) = result else {
                panic!("{id} must reject invalid input");
            };
            assert_eq!(error.code, "INVALID_ARG");
        }
    }

    #[test]
    fn statelist_operations_render_scroll_format_and_place_cursors() {
        let state = AppState::new();
        ops::dispatch(state.inner.as_ref(), "acq.single", json!({}))
            .unwrap_or_else(|error| panic!("{error}"));
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };

        let initial = call("statelist.get", json!({"limit":3}));
        assert_eq!(initial["rows"].as_array().map(Vec::len), Some(3));
        assert!(initial["rows"][0]["formatted_data"].is_string());
        assert_eq!(
            call("statelist.format.set", json!({"format":"binary"}))["format"],
            "binary"
        );
        let relative = call(
            "statelist.relative.set",
            json!({"relative":true,"page_size":3}),
        );
        assert_eq!(relative["config"]["relative"], true);
        let reordered = call(
            "statelist.column.reorder",
            json!({"columns":["data","sample","count","time"]}),
        );
        assert_eq!(reordered["config"]["columns"][0], "data");
        let formatted = call(
            "statelist.column.format.set",
            json!({"column":"data","format":"signed"}),
        );
        assert!(formatted["rows"][0]["formatted_data"].is_string());

        let down = call("statelist.scroll.page_down", json!({"page_size":2}));
        assert_eq!(down["config"]["scroll_row"], 2);
        assert_eq!(
            call("statelist.scroll.key", json!({"key":"ArrowUp"}))["config"]["scroll_row"],
            1
        );
        assert_eq!(
            call("statelist.scroll.drag", json!({"row":3}))["config"]["scroll_row"],
            3
        );
        call("statelist.place_cursor", json!({"row":2,"cursor":"C"}));
        assert_ne!(
            call("cursor.get", json!({}))["cursors"][2]["offset_samples"],
            0
        );

        for (id, params) in [
            ("statelist.format.set", json!({"format":"octal"})),
            (
                "statelist.column.reorder",
                json!({"columns":["data","data","count","time"]}),
            ),
            ("statelist.scroll.key", json!({"key":"Delete"})),
            ("statelist.place_cursor", json!({"row":999999})),
        ] {
            let result = ops::dispatch(state.inner.as_ref(), id, params);
            let Err(error) = result else {
                panic!("{id} must reject invalid input");
            };
            assert_eq!(error.code, "INVALID_ARG");
        }
    }

    #[test]
    fn notes_and_help_operations_are_persistent_and_self_describing() {
        let state = AppState::real_pending("offline");
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };
        assert_eq!(call("notes.get", json!({}))["notes"], "");
        let set = call("notes.set", json!({"text":"Bench notes ✓"}));
        assert_eq!(set["length"], 13);
        assert_eq!(call("notes.open", json!({}))["open"], true);
        assert_eq!(call("notes.get", json!({}))["notes"], "Bench notes ✓");

        assert!(
            call("help.contents", json!({}))["sections"]
                .as_array()
                .is_some_and(|sections| sections.len() >= 7)
        );
        assert_eq!(call("help.language", json!({}))["current"], "en");
        assert_eq!(call("help.website", json!({}))["opened"], false);
        assert_eq!(
            call("help.about", json!({}))["device"],
            "Intronix LogicPort LA1034"
        );
        assert!(
            call("help.shortcuts", json!({}))["shortcuts"]
                .as_array()
                .is_some_and(|shortcuts| shortcuts.len() >= 10)
        );

        let oversized = "x".repeat(1_048_577);
        let result = ops::dispatch(
            state.inner.as_ref(),
            "notes.set",
            json!({"notes":oversized}),
        );
        let Err(error) = result else {
            panic!("oversized notes must be rejected");
        };
        assert_eq!(error.code, "INVALID_ARG");
    }

    #[test]
    fn signal_and_group_operations_preserve_project_invariants() {
        let state = AppState::new();
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };
        assert_eq!(
            call("signals.list", json!({}))["signals"][32]["name"],
            "CLK1"
        );
        assert_eq!(
            call("signals.rename", json!({"wire":0,"name":"DATA"}))["signals"][0]["name"],
            "DATA"
        );
        let names = (0..34).map(|wire| format!("S{wire}")).collect::<Vec<_>>();
        assert_eq!(
            call("signals.dialog.ok", json!({"names":names}))["signals"][33]["name"],
            "S33"
        );

        let created = call(
            "groups.create",
            json!({"name":"Bus","wires":[0,1,2,3],"radix":"hex"}),
        );
        let id = created["group"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("missing group id"))
            .to_owned();
        assert_eq!(created["group"]["wires"].as_array().map(Vec::len), Some(4));
        assert_eq!(
            call("groups.members.add", json!({"id":id,"wires":[4,5]}))["group"]["wires"]
                .as_array()
                .map(Vec::len),
            Some(6)
        );
        assert_eq!(
            call("groups.members.remove", json!({"id":id,"wires":[1,5]}))["group"]["wires"]
                .as_array()
                .map(Vec::len),
            Some(4)
        );
        assert_eq!(
            call("groups.edit", json!({"id":id,"style":"analog"}))["group"]["style"],
            "analog"
        );
        let reversed = call("groups.reverse_display_order", json!({"id":id}));
        assert_eq!(reversed["group"]["display_order"], "high_bottom");
        let copied = call(
            "groups.copy",
            json!({"id":id,"new_id":"copy","name":"Bus Copy"}),
        );
        assert_eq!(copied["groups"].as_array().map(Vec::len), Some(2));
        assert_eq!(
            call("groups.rename", json!({"id":"copy","name":"Other"}))["group"]["name"],
            "Other"
        );
        assert_eq!(call("groups.dialog.open", json!({"id":id}))["open"], true);
        assert_eq!(
            call("groups.delete", json!({"id":"copy"}))["groups"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(call("groups.list", json!({}))["groups"][0]["id"], id);

        for (op, params) in [
            ("signals.rename", json!({"wire":34,"name":"bad"})),
            ("groups.create", json!({"name":"Bad","wires":[34]})),
            ("groups.create", json!({"name":"Dup","wires":[0,0]})),
            (
                "groups.validate",
                json!({"group":{"id":"x","name":"x","wires":[],"radix":"octal","signed":false,"wire_order":"msb_first","display_order":"high_top","style":"digital","color":"default","lpf_raw":null}}),
            ),
        ] {
            let result = ops::dispatch(state.inner.as_ref(), op, params);
            let Err(error) = result else {
                panic!("{op} must reject invalid input");
            };
            assert_eq!(error.code, "INVALID_ARG");
        }
    }

    #[test]
    fn row_operations_manage_layout_style_and_capture_values() {
        let state = AppState::new();
        let call = |id: &str, params: Value| {
            ops::dispatch(state.inner.as_ref(), id, params)
                .unwrap_or_else(|error| panic!("{id}: {error}"))
        };
        call("acq.single", json!({}));
        let group = call(
            "groups.create",
            json!({"id":"bus","name":"Bus","wires":[0,1],"radix":"binary","wire_order":"lsb_first"}),
        );
        assert_eq!(group["group"]["id"], "bus");
        call("rows.add.signal", json!({"id":"signal-row","wire":0}));
        call(
            "rows.insert.group",
            json!({"id":"group-row","group_id":"bus","index":0}),
        );
        assert_eq!(call("rows.list", json!({}))["rows"][0]["id"], "group-row");
        assert!(
            call("row.hover_value", json!({"id":"signal-row","sample":0}))["value"]
                .as_u64()
                .is_some()
        );
        let value = call("group.value_at", json!({"id":"bus","sample":0}));
        assert_eq!(value["radix"], "binary");
        assert!(
            value["formatted"]
                .as_str()
                .is_some_and(|formatted| formatted.starts_with("0b"))
        );
        assert_eq!(
            call("row.style.set", json!({"id":"group-row","style":"analog"}))["row"]["style"],
            "analog"
        );
        assert_eq!(
            call("row.color.set", json!({"id":"group-row","color":"#ff0000"}))["row"]["color"],
            "#ff0000"
        );
        assert_eq!(
            call("rows.height.set", json!({"id":"group-row","height_px":80}))["row"]["height_px"],
            80
        );
        assert_eq!(
            call("rows.collapse", json!({"id":"group-row"}))["row"]["expanded"],
            false
        );
        call("rows.reorder", json!({"ids":["signal-row","group-row"]}));
        assert_eq!(
            call("group.radix.set", json!({"id":"bus","radix":"hex"}))["groups"][0]["radix"],
            "hex"
        );
        call("group.signed.set", json!({"id":"bus","signed":true}));
        call(
            "group.display_order.set",
            json!({"id":"bus","display_order":"high_bottom"}),
        );
        let all = call("rows.add_all", json!({}));
        assert_eq!(all["rows"].as_array().map(Vec::len), Some(35));
        call("rows.remove.group", json!({"id":"group-row"}));
        assert_eq!(
            call("row.color.default", json!({"id":"signal-row"}))["row"]["color"],
            "default"
        );
        assert_eq!(
            call("rows.remove_all", json!({}))["rows"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );

        for (id, params) in [
            ("rows.add.signal", json!({"wire":34})),
            ("rows.height.set", json!({"id":"missing","height_px":20})),
            ("rows.reorder", json!({"ids":["missing"]})),
        ] {
            let result = ops::dispatch(state.inner.as_ref(), id, params);
            let Err(error) = result else {
                panic!("{id} must reject invalid input");
            };
            assert!(matches!(error.code.as_str(), "INVALID_ARG" | "UNKNOWN_ROW"));
        }
    }

    #[tokio::test]
    async fn schema_errors_use_envelope_and_dispatch_mutates_shared_state() {
        let state = AppState::new();
        let app = router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/ops/not.real/schema")
                    .body(Body::empty())
                    .unwrap_or_else(|e| panic!("{e}")),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap_or_else(|e| panic!("{e}")),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(body["error"]["code"], "UNKNOWN_OP");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ops/notes.set")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"notes":"bench"}"#))
                    .unwrap_or_else(|e| panic!("{e}")),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(response.status(), StatusCode::OK);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ops/notes.get")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap_or_else(|e| panic!("{e}")),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap_or_else(|e| panic!("{e}")),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(body["notes"], "bench");
    }

    #[tokio::test]
    async fn capture_operations_share_the_rest_dispatcher() {
        let state = AppState::new();
        let first = state
            .insert_capture(
                Capture::new(
                    0,
                    1e-6,
                    5,
                    vec![
                        lp_project::Run { data: 0, count: 5 },
                        lp_project::Run { data: 1, count: 5 },
                    ],
                )
                .unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{}", error.0.message));
        let app = router(state);
        let call = |id: &'static str, body: &'static str| {
            app.clone().oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/ops/{id}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap_or_else(|error| panic!("{error}")),
            )
        };
        let response = call("capture.summary", r#"{"capture_id":"latest"}"#)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(body["expanded_len"], 10);

        let response = call(
            "capture.measure",
            r#"{"capture_id":"latest","type":"transitions","source":"D0","left":0,"right":10}"#,
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(body["value"], 1.0);

        let response = call("capture.pin", r#"{"capture_id":1,"pinned":true}"#)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(response.status(), StatusCode::OK);
        let response = call("capture.get", r#"{"capture_id":1}"#)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(body["capture"]["id"], first.id);
        assert_eq!(body["pinned"], true);
    }

    #[tokio::test]
    async fn lpf_import_mutates_the_shared_project_through_rest() {
        let app = router(AppState::new());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/vendor/examples/Quickstart.LPF");
        let body =
            serde_json::to_vec(&json!({"path":path})).unwrap_or_else(|error| panic!("{error}"));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ops/project.import_lpf")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap_or_else(|error| panic!("{error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(response.status(), StatusCode::OK);
        let imported: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(imported["source"]["kind"], "lpf_import");
        assert_eq!(imported["source"]["unknown_keys"], json!([]));
        assert_eq!(imported["capture"]["trigger_sample"], 1023);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/project")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("{error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let stored: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(stored["source"]["kind"], "lpf_import");

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ops/capture.list")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"limit":20}"#))
                    .unwrap_or_else(|error| panic!("{error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let listed: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(listed["captures"].as_array().map(Vec::len), Some(1));
        assert_eq!(listed["captures"][0]["capture"]["trigger_sample"], 1023);
    }

    #[tokio::test]
    async fn mcp_http_initializes_and_acquires_through_shared_dispatcher() {
        let app = router(AppState::new());
        let stream = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/mcp")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("{error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(stream.status(), StatusCode::OK);
        assert_eq!(
            stream
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );
        let call = |body: &'static str| {
            app.clone().oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap_or_else(|error| panic!("{error}")),
            )
        };
        let response = call(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(body["result"]["protocolVersion"], "2025-06-18");

        let response = call(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"device_status","arguments":{}}}"#,
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(body["result"]["structuredContent"]["state"], "connected");

        let response = call(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"acquire_single","arguments":{"wait":true}}}"#,
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(body["result"]["isError"], false);
        assert_eq!(body["result"]["structuredContent"]["expanded_len"], 2048);

        let response = call(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"lease_acquire","arguments":{"steal":true}}}"#,
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let lease = body["result"]["structuredContent"]["lease"]
            .as_str()
            .unwrap_or_else(|| panic!("missing MCP lease"));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/vendor/examples/Quickstart.LPF");
        let request = json!({
            "jsonrpc":"2.0",
            "id":5,
            "method":"tools/call",
            "params":{
                "name":"op_call",
                "arguments":{
                    "op":"project.import_lpf",
                    "params":{"path":path},
                    "lease":lease
                }
            }
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).unwrap_or_else(|error| panic!("{error}")),
                    ))
                    .unwrap_or_else(|error| panic!("{error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let body: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(body["result"]["isError"], false);
        assert_eq!(
            body["result"]["structuredContent"]["source"]["kind"],
            "lpf_import"
        );
        assert_eq!(body["result"]["structuredContent"]["lease"], lease);
    }
}
