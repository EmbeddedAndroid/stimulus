use crate::api::{ApiError, AppState, api_error};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use lp_device::{
    acquisition::{AcquisitionConfig, AcquisitionError, acquire_single, halt, trigger_immediate},
    clock::{VirtualClock, WallClock},
    device::LogicPortDevice,
    link::{Link, LinkConfig},
    real::RealTransport,
};
use lp_project::Settings;
use lp_project::{Capture, Run};
use lp_proto::status::Phase;
use lp_proto::{
    encode::{
        Provenance,
        mode::encode_mode,
        rate::RATES,
        threshold::encode_threshold,
        trigger::{TriggerLayout, TriggerSpec},
    },
    setup_seq::{Dirty, Setup, setup_sequence},
};
use lp_sim::{SimSeed, SimTransport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/acquire", post(acquire))
        .route("/api/acquire/status", get(status))
        .route("/api/acquire/wait", get(wait))
        .route("/api/acquire/halt", post(halt_acquisition))
        .route("/api/acquire/trigger-immediate", post(immediate))
}

pub(crate) trait AcquisitionBackend: Send {
    fn single(&mut self) -> Result<Capture, AcquisitionError>;
    fn halt(&mut self) -> Result<Phase, AcquisitionError>;
    fn immediate(&mut self) -> Result<Phase, AcquisitionError>;
    fn apply_setup(&mut self, settings: &Settings) -> Result<bool, AcquisitionError>;
    fn health_check(&mut self) -> Result<(), AcquisitionError>;
}

pub(crate) struct OfflineBackend {
    reason: String,
}

impl OfflineBackend {
    pub(crate) fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    fn unavailable(&self) -> AcquisitionError {
        AcquisitionError::Capture(format!("device not connected: {}", self.reason))
    }
}

impl AcquisitionBackend for OfflineBackend {
    fn single(&mut self) -> Result<Capture, AcquisitionError> {
        Err(self.unavailable())
    }

    fn halt(&mut self) -> Result<Phase, AcquisitionError> {
        Err(self.unavailable())
    }

    fn immediate(&mut self) -> Result<Phase, AcquisitionError> {
        Err(self.unavailable())
    }

    fn apply_setup(&mut self, _settings: &Settings) -> Result<bool, AcquisitionError> {
        Err(self.unavailable())
    }

    fn health_check(&mut self) -> Result<(), AcquisitionError> {
        Err(self.unavailable())
    }
}

pub(crate) struct SimBackend {
    device: Link<SimTransport, VirtualClock>,
    clock: VirtualClock,
    sample_period_s: f64,
}
impl SimBackend {
    pub(crate) fn new(seed: SimSeed) -> Self {
        Self {
            device: Link::new(
                SimTransport::new(seed),
                VirtualClock::default(),
                LinkConfig::default(),
            ),
            clock: VirtualClock::default(),
            sample_period_s: 1e-7,
        }
    }
}
impl AcquisitionBackend for SimBackend {
    fn single(&mut self) -> Result<Capture, AcquisitionError> {
        let result = acquire_single(
            &mut self.device,
            &mut self.clock,
            AcquisitionConfig::default(),
        )?;
        capture_from_readback(
            result.readback.samples,
            result.readback.trigger_sample,
            self.sample_period_s,
        )
    }
    fn halt(&mut self) -> Result<Phase, AcquisitionError> {
        halt(
            &mut self.device,
            &mut self.clock,
            AcquisitionConfig::default(),
        )
    }
    fn immediate(&mut self) -> Result<Phase, AcquisitionError> {
        trigger_immediate(&mut self.device)
    }
    fn apply_setup(&mut self, settings: &Settings) -> Result<bool, AcquisitionError> {
        let mut divider = None;
        apply_register_setup(&mut self.device, settings, &mut divider)?;
        self.sample_period_s = 1.0 / settings.sample.rate_hz as f64;
        Ok(false)
    }

    fn health_check(&mut self) -> Result<(), AcquisitionError> {
        Ok(())
    }
}

pub(crate) struct RealBackend {
    device: Link<RealTransport, WallClock>,
    clock: WallClock,
    sample_period_s: f64,
    ccf: lp_ccf::Ccf,
    image: u8,
    divider: Option<[u8; 2]>,
    // Whether the device is configured to capture RLE-compressed. The readback
    // MUST use the same mode the device captured in: a compressed capture read
    // as non-compressed does an extra POST_COUNT_RD register read that desyncs
    // the FIFO on alternate acquisitions (full/empty/full/empty). Kept in sync
    // with the applied settings.
    compressed: bool,
}
impl RealBackend {
    pub(crate) fn open_transport() -> Result<Self, String> {
        eprintln!("real startup: opening FTDI transport");
        let transport = RealTransport::open().map_err(|error| error.to_string())?;
        let device = Link::new(transport, WallClock::default(), LinkConfig::default());
        let ccf_path =
            std::env::var("LP_CCF").unwrap_or_else(|_| "fixtures/vendor/LogicPort.ccf".to_owned());
        let ccf = lp_ccf::Ccf::load(&ccf_path, true).map_err(|error| error.to_string())?;
        Ok(Self {
            device,
            clock: WallClock::default(),
            sample_period_s: 1e-7,
            ccf,
            image: 7,
            // The vendor's first settings pass treats the rate group as dirty
            // and emits both one-byte divider fields before MODE, even when
            // image 7 already has the 10 MHz values at power-on.
            divider: None,
            // Set by apply_setup (always run before serving) to the real setting.
            compressed: false,
        })
    }

    pub(crate) fn establish_session(&mut self) -> Result<(), String> {
        let image = self
            .ccf
            .image_for_upload(7)
            .map_err(|error| error.to_string())?;
        eprintln!("real startup: establishing FPGA image-7 session");
        let _configured = self
            .device
            .configure_fpga(&image, 7, false)
            .map_err(|error| error.to_string())?;
        eprintln!("real startup: FPGA image-7 session ready");
        Ok(())
    }
}

pub fn reset_real_usb() -> Result<(), String> {
    RealTransport::reset_attached().map_err(|error| error.to_string())
}
impl AcquisitionBackend for RealBackend {
    fn single(&mut self) -> Result<Capture, AcquisitionError> {
        let result = acquire_single(
            &mut self.device,
            &mut self.clock,
            AcquisitionConfig {
                compressed: self.compressed,
                ..AcquisitionConfig::default()
            },
        )?;
        capture_from_readback(
            result.readback.samples,
            result.readback.trigger_sample,
            self.sample_period_s,
        )
    }
    fn halt(&mut self) -> Result<Phase, AcquisitionError> {
        halt(
            &mut self.device,
            &mut self.clock,
            AcquisitionConfig {
                compressed: self.compressed,
                ..AcquisitionConfig::default()
            },
        )
    }
    fn immediate(&mut self) -> Result<Phase, AcquisitionError> {
        trigger_immediate(&mut self.device)
    }
    fn apply_setup(&mut self, settings: &Settings) -> Result<bool, AcquisitionError> {
        let entry = rate_entry(settings)?;
        let reconfigure = entry.image_timing != self.image;
        if reconfigure {
            let upload = self
                .ccf
                .image_for_upload(entry.image_timing)
                .map_err(|error| AcquisitionError::Setup(error.to_string()))?;
            let _configured = self
                .device
                .configure_fpga(&upload, entry.image_timing, true)
                .map_err(|error| AcquisitionError::Setup(error.to_string()))?;
            self.image = entry.image_timing;
            // Each timing image boots with its corresponding default divider:
            // image 6 is the 500 MHz path (00 00), while image 7 boots at the
            // application default 10 MHz (21 00). Both values are established
            // by the register-smoke captures.
            self.divider = Some([entry.r0, entry.r1]);
        }
        // A setup framing error does not prove the FPGA image is bad. Forcing
        // nCONFIG here can turn a recoverable host-session fault into a device
        // that requires power removal. Return the error so the connection
        // worker can reopen/resynchronise while preserving the live image.
        apply_register_setup(&mut self.device, settings, &mut self.divider)?;
        self.sample_period_s = entry.period_s;
        // The readback must match the mode the device just captured in.
        self.compressed = settings.sample.compression;
        Ok(reconfigure)
    }

    fn health_check(&mut self) -> Result<(), AcquisitionError> {
        let identity = RealTransport::attached_identity()
            .map_err(|error| AcquisitionError::Capture(error.to_string()))?;
        if identity.serial != self.device.identity().serial {
            return Err(AcquisitionError::Capture(format!(
                "LogicPort identity changed: expected serial {}, got {}",
                self.device.identity().serial,
                identity.serial
            )));
        }
        Ok(())
    }
}

fn rate_entry(
    settings: &Settings,
) -> Result<&'static lp_proto::encode::rate::RateEntry, AcquisitionError> {
    let entry = RATES
        .get(usize::from(settings.sample.rate_index))
        .filter(|entry| entry.hz == settings.sample.rate_hz)
        .ok_or_else(|| AcquisitionError::Setup("rate_index and rate_hz do not match".into()))?;
    if settings.sample.compression && !entry.compression_ok {
        return Err(AcquisitionError::Setup(
            "compression is unavailable above 200 MHz".into(),
        ));
    }
    Ok(entry)
}

pub(crate) fn validate_setup_settings(settings: &Settings) -> Result<(), AcquisitionError> {
    let _ = rate_entry(settings)?;
    match settings.sample.mode {
        lp_project::SampleMode::Timing => encode_mode(0, true, false),
        lp_project::SampleMode::State => encode_mode(settings.sample.state.clock, true, false),
    }
    .map_err(|error| AcquisitionError::Setup(error.to_string()))?;
    // Combine modes other than "immediate" (for example those imported from an
    // LPF project) are not encoded yet; they fall back to immediate at setup
    // rather than blocking settings changes and acquisition. See
    // apply_register_setup.
    Ok(())
}

fn apply_register_setup(
    device: &mut dyn lp_device::device::LogicPortDevice,
    settings: &Settings,
    divider: &mut Option<[u8; 2]>,
) -> Result<(), AcquisitionError> {
    validate_setup_settings(settings)?;
    let entry = rate_entry(settings)?;
    let mode = match settings.sample.mode {
        lp_project::SampleMode::Timing => encode_mode(0, true, false),
        lp_project::SampleMode::State => encode_mode(settings.sample.state.clock, true, false),
    }
    .map_err(|error| AcquisitionError::Setup(error.to_string()))?;
    let mask2 = settings
        .logic_sense
        .inverted
        .iter()
        .enumerate()
        .fold(0_u64, |mask, (channel, inverted)| {
            mask | (u64::from(*inverted) << channel)
        });
    let requested_pre = ((settings.sample.pretrigger_pct / 100.0) * 2048.0)
        .round()
        .clamp(0.0, 2047.0) as u16;
    // The device's timing pipeline has eight samples of latency, so the
    // programmed split must shift while preserving the 2048-sample total:
    // a 50% split programs 1032/1016, not the unadjusted 1024/1024 pair.
    let pre_count = match settings.sample.mode {
        lp_project::SampleMode::Timing => requested_pre.saturating_add(8).min(2047),
        lp_project::SampleMode::State => requested_pre,
    };
    let post_count = 2048_u16.saturating_sub(pre_count);
    // Only the immediate combine mode is encoded; any other mode (such as one
    // imported from an LPF project) falls back to immediate (the default) so
    // that acquisition and settings changes keep working.
    let trigger = TriggerSpec::default();
    let enable_mask = (1_u64 << 34) - 1;
    let setup = Setup {
        rate: [entry.r0, entry.r1],
        mode,
        enable_mask,
        // The initial settings pass must commit the all-enabled mask
        // between MASK_GATE=0 and MASK_GATE=1.
        channel_mask_active: true,
        mask2,
        mode_flag: settings.sample.compression,
        trigger,
        trigger_layout: TriggerLayout::default(),
        threshold_code: encode_threshold(settings.threshold_v, 0),
        pre_count,
        post_count,
        arm: false,
        provenance: Provenance::Provisional,
    };
    // Write the divider when the rate group is dirty. On the first settings
    // pass the host cache is unknown, so it is treated as dirty.
    let rate_dirty = divider.is_none_or(|current| current != setup.rate);
    if rate_dirty {
        eprintln!(
            "real setup: initialize divider -> {:02x} {:02x}",
            setup.rate[0], setup.rate[1]
        );
    }
    let operations = setup_sequence(
        &setup,
        Dirty {
            rate: rate_dirty,
            mode: true,
            trigger: true,
            threshold: true,
            position: true,
        },
    );
    let writes = operations
        .iter()
        .map(|operation| (operation.addr, operation.data.clone()))
        .collect::<Vec<_>>();
    device
        .write_checked_sequence(&writes)
        .map_err(|error| AcquisitionError::Setup(format!("write setup sequence: {error}")))?;
    let image_id = device
        .read8(lp_proto::regs::ctrl::IMAGE_ID)
        .map_err(|error| {
            AcquisitionError::Setup(format!("verify IMAGE_ID after setup: {error}"))
        })?;
    if image_id & 0x10 == 0 {
        return Err(AcquisitionError::Setup(format!(
            "verify IMAGE_ID after setup: invalid value 0x{image_id:02x}"
        )));
    }
    *divider = Some(setup.rate);
    Ok(())
}

fn capture_from_readback(
    samples: Vec<lp_proto::slot::Sample>,
    trigger_sample: u64,
    sample_period_s: f64,
) -> Result<Capture, AcquisitionError> {
    let runs = samples
        .into_iter()
        .map(|sample| Run {
            data: sample.bits,
            count: sample.repeat.saturating_add(1),
        })
        .collect::<Vec<_>>();
    let expanded_len = runs.iter().map(|run| run.count).sum::<u64>();
    let trigger = trigger_sample.min(expanded_len.saturating_sub(1));
    Capture::new(0, sample_period_s, trigger, runs)
        .map_err(|error| AcquisitionError::Capture(error.to_string()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Single,
    Recurring,
}
fn default_mode() -> Mode {
    Mode::Single
}
#[derive(Debug, Deserialize)]
struct AcquireRequest {
    #[serde(default = "default_mode")]
    mode: Mode,
    #[serde(default)]
    clear_before: bool,
    max_runs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AcqStatus {
    pub(crate) state: &'static str,
    pub(crate) hw_status_byte: u8,
    pub(crate) acq_count: u64,
    pub(crate) samples: u64,
    pub(crate) buffer_fill_pct: f64,
    pub(crate) recurring: bool,
    pub(crate) warnings: Vec<String>,
}
impl Default for AcqStatus {
    fn default() -> Self {
        Self {
            state: "idle",
            hw_status_byte: 0,
            acq_count: 0,
            samples: 0,
            buffer_fill_pct: 0.0,
            recurring: false,
            warnings: Vec::new(),
        }
    }
}

async fn acquire(
    State(state): State<AppState>,
    Json(request): Json<AcquireRequest>,
) -> Result<Json<Value>, ApiError> {
    if request.clear_before {
        state.clear_captures()?;
    }
    match request.mode {
        Mode::Single => {
            if state.is_recurring() {
                return Err(api_error("ACQ_BUSY", "recurring acquisition is running"));
            }
            if state.needs_replug() {
                return Err(api_error("DEVICE_NEEDS_REPLUG", crate::api::REPLUG_HINT));
            }
            state.set_acq_state("readback")?;
            // A single acquisition does blocking USB I/O (arm, poll, readback).
            // Run it on the blocking pool, NOT the async worker thread: a wedged
            // readback must never stall the tokio runtime, or /api/health and the
            // recovery worker would hang and the whole daemon goes unresponsive.
            let worker = state.clone();
            let outcome = tokio::task::spawn_blocking(move || worker.acquire_single())
                .await
                .map_err(|error| api_error("INTERNAL", format!("acquire task join: {error}")))?;
            let capture = match outcome {
                Ok(capture) => capture,
                Err(error) => return Err(recover_or_map(&state, error)),
            };
            let capture = state.insert_capture(capture)?;
            state.set_capture_ready(capture.expanded_len())?;
            serde_json::to_value(capture)
                .map(Json)
                .map_err(|error| api_error("INTERNAL", error.to_string()))
        }
        Mode::Recurring => {
            if request.max_runs == Some(0) {
                return Err(api_error("INVALID_ARG", "max_runs must be positive"));
            }
            state.start_recurring(request.max_runs)?;
            Ok(Json(json!({"recurring":true,"max_runs":request.max_runs})))
        }
    }
}

async fn status(State(state): State<AppState>) -> Result<Json<AcqStatus>, ApiError> {
    state.acquisition_status().map(Json)
}

async fn halt_acquisition(State(state): State<AppState>) -> Result<Json<AcqStatus>, ApiError> {
    let recurring = state.stop_recurring();
    if !recurring {
        state.halt_acquisition().map_err(acquisition_error)?;
    }
    state.set_acq_state("halted")?;
    status(State(state)).await
}

#[derive(Deserialize)]
struct WaitQuery {
    timeout_ms: Option<u64>,
    #[serde(rename = "for")]
    target: Option<String>,
}
async fn wait(
    State(state): State<AppState>,
    Query(query): Query<WaitQuery>,
) -> Result<Json<AcqStatus>, ApiError> {
    let timeout = Duration::from_millis(query.timeout_ms.unwrap_or(30_000).min(300_000));
    let target = query.target.as_deref().unwrap_or("ready");
    if !matches!(target, "ready" | "idle" | "phase_change") {
        return Err(api_error(
            "INVALID_ARG",
            format!("unknown wait target: {target}"),
        ));
    }
    let initial = state.acquisition_status()?;
    let deadline = Instant::now() + timeout;
    loop {
        let current = state.acquisition_status()?;
        let reached = match target {
            "ready" => current.state == "ready",
            "idle" => !current.recurring && matches!(current.state, "idle" | "ready" | "halted"),
            "phase_change" => {
                current.state != initial.state || current.acq_count != initial.acq_count
            }
            _ => false,
        };
        if reached {
            return Ok(Json(current));
        }
        if Instant::now() >= deadline {
            return Err(api_error(
                "ACQ_TIMEOUT",
                format!("wait for {target} timed out"),
            ));
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[derive(Serialize)]
struct ImmediateResult {
    phase_before: &'static str,
    phase_after: &'static str,
    strobe: bool,
}
async fn immediate(State(state): State<AppState>) -> Result<Json<ImmediateResult>, ApiError> {
    let phase = state.trigger_immediate().map_err(acquisition_error)?;
    Ok(Json(ImmediateResult {
        phase_before: phase_name(phase),
        phase_after: phase_name(phase),
        strobe: true,
    }))
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Complete => "idle",
        Phase::Prefill => "prefill",
        Phase::Armed => "armed",
        Phase::Postfill => "postfill",
    }
}
pub(crate) fn acquisition_error(error: AcquisitionError) -> ApiError {
    match error {
        AcquisitionError::CannotTrigger(_) => api_error("ACQ_NOT_RUNNING", error.to_string()),
        AcquisitionError::Setup(_) => api_error("INVALID_ARG", error.to_string()),
        AcquisitionError::Capture(message) if message.starts_with("device not connected:") => {
            api_error("DEVICE_NOT_CONNECTED", message)
        }
        _ => api_error("USB_ERROR", error.to_string()),
    }
}
/// Map an acquisition failure, escalating a command-path wedge into automatic
/// recovery. A wedge (see `api::is_command_wedge`) parks the backend Offline so
/// the recovery worker reopens and, if needed, software-USB-resets the device,
/// and the caller is told to retry -- instead of an opaque USB_ERROR/502 that
/// leaves the channel dead for every future request. Benign framing errors fall
/// through to the ordinary mapping.
pub(crate) fn recover_or_map(state: &AppState, error: AcquisitionError) -> ApiError {
    let message = error.to_string();
    if crate::api::is_command_wedge(&message) {
        state.begin_recovery(format!(
            "command channel wedged during acquisition: {message}"
        ));
        return api_error(
            "DEVICE_RECOVERING",
            "the logic analyzer stopped responding; automatic recovery is running, retry shortly",
        );
    }
    acquisition_error(error)
}
pub(crate) fn acquisition_tool_error(error: AcquisitionError) -> lp_core::ToolError {
    acquisition_error(error).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use tower::ServiceExt;

    async fn request(
        state: AppState,
        method: &str,
        uri: &str,
        body: &str,
    ) -> (StatusCode, Vec<u8>) {
        let response = crate::api::router(state)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_owned()))
                    .unwrap_or_else(|error| panic!("{error}")),
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap_or_else(|error| panic!("{error}"))
            .to_vec();
        (status, bytes)
    }

    #[tokio::test]
    async fn single_acquisition_populates_capture_store_and_status() {
        let state = AppState::new();
        let (status, body) = request(state.clone(), "POST", "/api/acquire", "{}").await;
        assert_eq!(status, StatusCode::OK);
        let capture: Capture =
            serde_json::from_slice(&body).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(capture.id, 1);
        assert!(!capture.runs.is_empty());

        let (status, body) = request(state.clone(), "GET", "/api/acquire/status", "").await;
        assert_eq!(status, StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&body).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(value["state"], "ready");
        assert_eq!(value["acq_count"], 1);
        assert_eq!(value["samples"], capture.expanded_len());

        let (status, body) = request(state, "GET", "/api/captures/1/data", "").await;
        assert_eq!(status, StatusCode::OK);
        let (_, slots) =
            lp_core::api::binary::decode_rle(&body).unwrap_or_else(|error| panic!("{error}"));
        assert!(!slots.is_empty());
    }

    #[tokio::test]
    async fn recurring_runs_to_limit_and_wait_observes_completion() {
        let state = AppState::new();
        let (status, _) = request(
            state.clone(),
            "POST",
            "/api/acquire",
            r#"{"mode":"recurring","max_runs":3}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = request(
            state.clone(),
            "GET",
            "/api/acquire/wait?for=idle&timeout_ms=1000",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&body).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(value["acq_count"], 3);
        assert_eq!(value["recurring"], false);
        assert_eq!(
            state.captures().list(10).map(|captures| captures.len()),
            Ok(3)
        );
    }

    #[tokio::test]
    async fn busy_invalid_and_idle_immediate_have_structured_errors() {
        let state = AppState::new();
        let cases = [
            ("/api/acquire", r#"{"mode":"recurring","max_runs":0}"#),
            ("/api/acquire/trigger-immediate", "{}"),
            ("/api/acquire/wait?for=unknown&timeout_ms=1", ""),
        ];
        for (uri, body) in cases {
            let method = if uri.contains("wait") { "GET" } else { "POST" };
            let (status, body) = request(state.clone(), method, uri, body).await;
            assert!(status.is_client_error());
            let value: serde_json::Value =
                serde_json::from_slice(&body).unwrap_or_else(|error| panic!("{error}"));
            assert!(value.get("error").is_some());
        }
    }
}
