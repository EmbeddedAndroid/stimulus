use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Settings {
    pub sample: SampleSettings,
    pub trigger: TriggerSettings,
    pub threshold_v: f64,
    pub logic_sense: LogicSense,
    pub acquisition: AcquisitionSettings,
    pub export: ExportSettings,
    pub print: PrintSettings,
    pub options: Options,
    pub controls: Controls,
    pub usb_error_count: u64,
    pub is_demo_data: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SampleSettings {
    pub mode: SampleMode,
    pub rate_index: u8,
    pub rate_hz: u64,
    pub rate_units: String,
    pub state: StateSettings,
    pub compression: bool,
    pub prefill_timeout: TimeoutSetting,
    pub postfill_timeout: TimeoutSetting,
    pub pretrigger_pct: f64,
    /// Bitmask of channels to capture (bit N = channel DN). Zero means all
    /// channels, the default. Masking out a fast channel keeps its transitions
    /// out of the capture, which extends the effective window and stops a fast
    /// input from crowding out a slower signal of interest.
    #[serde(default)]
    pub channel_mask: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SampleMode {
    Timing,
    State,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateSettings {
    pub clock: u8,
    pub edge: String,
    pub window_index: u8,
    pub window_ns: f64,
    pub qualifier: Qualifier,
    pub declared_rate_hz: u64,
    pub declared_units: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Qualifier {
    pub enabled: bool,
    pub polarity: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TimeoutSetting {
    pub index: u8,
    pub ms: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TriggerSettings {
    pub combine: String,
    pub levels: Value,
    pub edge_cells: Vec<Value>,
    pub pattern_cells: Vec<Value>,
    pub edge_group_flag: bool,
    /// A simple single-channel edge trigger for term A. When present, the
    /// acquisition arms on this edge instead of triggering immediately. Kept
    /// separate from the opaque LPF `edge_cells`; `None` preserves the
    /// immediate-trigger default and the existing serialization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<EdgeTrigger>,
}

/// A single-channel edge-trigger term. `plane` and `pattern` are the raw
/// encoder codes (edge plane 1/2, pattern 0..3); their mapping to
/// rising/falling/either is resolved empirically on hardware (see the trigger
/// entry in docs/KNOWN-GAPS.md), so the raw codes are stored to keep that
/// mapping in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EdgeTrigger {
    pub channel: u8,
    pub plane: u8,
    pub pattern: u8,
    /// Raw term-mode bytes and combine code. Exposed so the still-unmapped
    /// edge-term encoding can be resolved empirically on hardware without a
    /// rebuild; they default to 0 so a plain edge term is unaffected.
    #[serde(default)]
    pub combine: u8,
    #[serde(default)]
    pub m20: u8,
    #[serde(default)]
    pub m22: u8,
    #[serde(default)]
    pub m23: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LogicSense {
    pub inverted: Vec<bool>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AcquisitionSettings {
    pub clear_before: bool,
    pub save_on_acq: SaveOnAcq,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SaveOnAcq {
    pub enabled: bool,
    pub action: u8,
    pub max_files: u32,
    pub holdoff_s: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExportSettings {
    pub format: String,
    pub radix: String,
    pub target_path: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PrintSettings {
    pub caption: bool,
    pub caption_type: String,
    pub caption_string: String,
    pub date: bool,
    pub measurements: bool,
    pub orientation: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Options {
    pub units: String,
    pub scale_relative: bool,
    pub sample_reference: String,
    pub reference_position: f64,
    pub statelist_format: String,
    pub cursor_snap: bool,
    pub show_graticule: bool,
    pub show_trigger: bool,
    pub show_cursors: bool,
    pub cursor_qty: u8,
    pub color_scheme: String,
    pub alt_background: AltBackground,
    pub waveforms_in_front: bool,
    pub large_waveforms: bool,
    pub optimization: String,
    pub save_on_exit: bool,
    pub extended_rates: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AltBackground {
    pub enabled: bool,
    pub color: String,
    pub intensity: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Controls {
    pub selections: Vec<i8>,
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Signal {
    pub wire: u8,
    pub wire_name: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub wires: Vec<u8>,
    pub radix: String,
    pub signed: bool,
    pub wire_order: String,
    pub display_order: String,
    pub style: String,
    pub color: String,
    pub lpf_raw: Option<Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Interpreter {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub wires: Vec<u8>,
    pub radix: String,
    pub style: String,
    pub color: String,
    pub config: Value,
    pub extra: Option<Value>,
    pub lpf_raw: Option<Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Row {
    pub id: String,
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub parent: Option<String>,
    pub height_px: u32,
    pub color_index: u8,
    #[serde(default = "default_row_style")]
    pub style: String,
    #[serde(default = "default_row_color")]
    pub color: String,
    pub expanded: bool,
    pub visible: bool,
}
fn default_row_style() -> String {
    "digital".to_owned()
}
fn default_row_color() -> String {
    "default".to_owned()
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Column {
    #[serde(rename = "type")]
    pub kind: String,
    pub width_px: u32,
    pub visible: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewState {
    pub scale_s_per_px: f64,
    pub reference_offset_samples: i64,
    pub columns_visible: bool,
    pub panel: String,
    pub theme: String,
    pub statelist: Value,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Cursor {
    pub id: char,
    pub offset_samples: i64,
    pub offset_s: f64,
    pub tracks: Option<char>,
    pub visible: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MeasurementSlot {
    pub slot: u8,
    #[serde(rename = "type")]
    pub kind: String,
    pub left: String,
    pub right: String,
    pub source: String,
}
impl Default for ViewState {
    fn default() -> Self {
        Self {
            scale_s_per_px: 1e-6,
            reference_offset_samples: 0,
            columns_visible: true,
            panel: "waveforms".into(),
            theme: "default".into(),
            statelist: Value::Object(Default::default()),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sample: SampleSettings {
                mode: SampleMode::Timing,
                // Vendor cold-start setup writes divider 21 00 (10 MHz).
                // Keep the native-project default aligned with that hardware
                // default; faster rates are explicit user selections.
                rate_index: 7,
                rate_hz: 10_000_000,
                rate_units: "hz".into(),
                state: StateSettings {
                    clock: 0,
                    edge: "rising".into(),
                    window_index: 0,
                    window_ns: 0.0,
                    qualifier: Qualifier {
                        enabled: false,
                        polarity: "high".into(),
                    },
                    declared_rate_hz: 10_000_000,
                    declared_units: "hz".into(),
                },
                compression: false,
                prefill_timeout: TimeoutSetting { index: 0, ms: None },
                postfill_timeout: TimeoutSetting { index: 0, ms: None },
                pretrigger_pct: 50.0,
                channel_mask: 0,
            },
            trigger: TriggerSettings {
                combine: "immediate".into(),
                levels: Value::Object(Default::default()),
                edge_cells: Vec::new(),
                pattern_cells: Vec::new(),
                edge_group_flag: false,
                edge: None,
            },
            threshold_v: 1.65,
            logic_sense: LogicSense {
                inverted: vec![false; 34],
            },
            acquisition: AcquisitionSettings {
                clear_before: true,
                save_on_acq: SaveOnAcq {
                    enabled: false,
                    action: 0,
                    max_files: 16,
                    holdoff_s: 0.0,
                },
            },
            export: ExportSettings {
                format: "as_formatted".into(),
                radix: "binary".into(),
                target_path: None,
            },
            print: PrintSettings {
                caption: true,
                caption_type: "project".into(),
                caption_string: String::new(),
                date: true,
                measurements: true,
                orientation: "auto".into(),
            },
            options: Options {
                units: "time".into(),
                scale_relative: true,
                sample_reference: "trigger".into(),
                reference_position: 0.5,
                statelist_format: "hex".into(),
                cursor_snap: true,
                show_graticule: true,
                show_trigger: true,
                show_cursors: true,
                cursor_qty: 2,
                color_scheme: "default".into(),
                alt_background: AltBackground {
                    enabled: false,
                    color: "#000000".into(),
                    intensity: 0.0,
                },
                waveforms_in_front: true,
                large_waveforms: false,
                optimization: "maximum_performance".into(),
                save_on_exit: true,
                extended_rates: false,
            },
            controls: Controls {
                selections: vec![-1; 5],
                values: vec![0.0; 5],
            },
            usb_error_count: 0,
            is_demo_data: false,
        }
    }
}
