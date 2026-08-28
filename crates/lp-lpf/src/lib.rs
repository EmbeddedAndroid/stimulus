pub mod keys;

use encoding_rs::WINDOWS_1252;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub key: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Low,
    High,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleRun {
    pub channels: [Level; 34],
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleData {
    pub metadata: [u64; 4],
    pub runs: Vec<SampleRun>,
}

impl SampleData {
    pub fn expanded_len(&self) -> Result<u64, Error> {
        self.runs.iter().try_fold(0_u64, |total, run| {
            total
                .checked_add(run.count)
                .ok_or(Error::SampleCountOverflow)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub records: Vec<Record>,
    pub unknown_keys: Vec<String>,
    pub sample_data: SampleData,
}

impl Document {
    pub fn records(&self, key: &str) -> impl Iterator<Item = &Record> {
        self.records.iter().filter(move |record| record.key == key)
    }

    pub fn value(&self, key: &str) -> Result<&str, Error> {
        let record = self
            .records
            .iter()
            .find(|record| record.key == key)
            .ok_or_else(|| Error::MissingRecord(key.to_owned()))?;
        if record.values.len() != 1 {
            return Err(Error::Arity {
                key: key.to_owned(),
                expected: 1,
                got: record.values.len(),
            });
        }
        Ok(&record.values[0])
    }

    pub fn to_capture(&self, id: u32) -> Result<lp_project::Capture, Error> {
        let period = self.value("AcquiredSamplePeriod")?;
        let sample_period_s = period
            .parse::<f64>()
            .map_err(|_| Error::Float(period.to_owned()))?;
        let runs = self
            .sample_data
            .runs
            .iter()
            .map(|run| {
                let data = run
                    .channels
                    .iter()
                    .enumerate()
                    .fold(0_u64, |bits, (channel, level)| {
                        bits | (u64::from(*level == Level::High) << channel)
                    });
                lp_project::Run {
                    data,
                    count: run.count,
                }
            })
            .collect::<Vec<_>>();
        let mut capture =
            lp_project::Capture::new(id, sample_period_s, self.sample_data.metadata[2], runs)
                .map_err(|error| Error::Capture(error.to_string()))?;
        capture.reference_sample = self.sample_data.metadata[3];
        capture.channels_acquired = (0..34).fold(0_u64, |mask, channel| {
            let acquired = self
                .sample_data
                .runs
                .iter()
                .all(|run| run.channels[channel] != Level::Unknown);
            mask | (u64::from(acquired) << channel)
        });
        Ok(capture)
    }

    pub fn to_project(
        &self,
        timestamp: impl Into<String>,
        source_path: Option<String>,
    ) -> Result<lp_project::Project, Error> {
        let mut project = lp_project::Project::new(timestamp);
        project.source = lp_project::Source {
            kind: lp_project::SourceKind::LpfImport,
            path: source_path,
            unknown_keys: self.unknown_keys.clone(),
        };
        project.capture = Some(self.to_capture(1)?);
        project.read_only = self.optional_bool_value("ReadOnly", false)?;
        project.notes = self.value("NotesString/")?.to_owned();

        let signals = self.values("Signals")?;
        if signals.len() != 34 {
            return Err(Error::Arity {
                key: "Signals".into(),
                expected: 34,
                got: signals.len(),
            });
        }
        for (signal, name) in project.signals.iter_mut().zip(signals) {
            signal.name.clone_from(name);
        }

        project.view.scale_s_per_px = self.float_value("ScaleFactor")?;
        project.view.reference_offset_samples = self.integer_value("ReferenceOffset")?;
        project.view.columns_visible = self.bool_value("ColumnsVisible")?;
        project.settings.options.reference_position = self.float_value("ReferencePosition")?;
        project.settings.options.scale_relative = self.bool_value("ScaleRelativeToReference")?;
        project.settings.options.cursor_snap = self.bool_value("CursorSnap")?;
        project.settings.options.show_graticule = self.bool_value("ShowGraticule")?;
        project.settings.options.show_trigger = self.bool_value("ShowTrigger")?;
        project.settings.options.show_cursors = self.bool_value("ShowCursors")?;
        project.settings.options.cursor_qty = self
            .integer_value("LastVisibleCursor")?
            .checked_add(1)
            .and_then(|value| u8::try_from(value).ok())
            .ok_or_else(|| {
                Error::Integer(self.value("LastVisibleCursor").unwrap_or_default().into())
            })?;
        project.settings.options.units = if self.bool_value("UnitsTime")? {
            "time".into()
        } else {
            "samples".into()
        };

        let inverted = self.values("InvertedChannelList")?;
        if inverted.len() != 34 {
            return Err(Error::Arity {
                key: "InvertedChannelList".into(),
                expected: 34,
                got: inverted.len(),
            });
        }
        project.settings.logic_sense.inverted = inverted
            .iter()
            .map(|value| parse_bool(value))
            .collect::<Result<Vec<_>, _>>()?;
        project.settings.sample.compression = self.bool_value("UseCompression")?;
        project.settings.sample.prefill_timeout.index = self.u8_value("PrefillTimeout")?;
        project.settings.sample.postfill_timeout.index = self.u8_value("PostfillTimeout")?;
        project.settings.is_demo_data = self.bool_value("IsDemoData")?;
        project.settings.acquisition.save_on_acq.enabled = self.bool_value("SaveOnAcq")?;
        project.settings.acquisition.save_on_acq.action = self.u8_value("SaveOnAcqAction")?;
        project.settings.acquisition.save_on_acq.max_files = self.u32_value("SaveOnAcqMaxFiles")?;
        project.settings.acquisition.save_on_acq.holdoff_s =
            self.float_value("SaveOnAcqHoldoff")?;
        project.settings.sample.mode = if self.bool_value("TimingMode")? {
            lp_project::SampleMode::Timing
        } else {
            lp_project::SampleMode::State
        };
        let rate_hz = (1.0 / self.float_value("AcquiredSamplePeriod")?).round() as u64;
        project.settings.sample.rate_hz = rate_hz;
        project.settings.sample.rate_index = rate_index(rate_hz)?;
        project.settings.sample.rate_units = "hz".into();
        project.settings.sample.state.clock = self.u8_value("StateClockSelect")?;
        project.settings.sample.state.edge = match self.u8_value("StateClockPolarity")? {
            0 => "falling",
            _ => "rising",
        }
        .into();
        project.settings.sample.state.window_index = self.u8_value("StateClockDelay")?;
        project.settings.sample.state.qualifier.enabled =
            self.bool_value("QualifyStateSampling")?;
        project.settings.sample.state.qualifier.polarity =
            if self.u8_value("QualifierPolarity")? == 0 {
                "low"
            } else {
                "high"
            }
            .into();
        project.settings.sample.state.declared_rate_hz =
            self.float_value("StateClockRate")?.round() as u64;
        project.settings.sample.state.declared_units =
            state_units(self.u8_value("StateClockUnits")?).into();
        project.settings.sample.pretrigger_pct = self
            .values("ControlValues")?
            .get(2)
            .ok_or_else(|| Error::Arity {
                key: "ControlValues".into(),
                expected: 5,
                got: self.values("ControlValues").map_or(0, <[String]>::len),
            })
            .and_then(|value| parse_f64(value))?;

        let selections = self.values("ControlSelections")?;
        let control_values = self.values("ControlValues")?;
        if selections.len() < 5 || control_values.len() < 5 {
            return Err(Error::ControlCardinality {
                selections: selections.len(),
                values: control_values.len(),
            });
        }
        project.settings.controls.selections = selections[..5]
            .iter()
            .map(|value| value.parse().map_err(|_| Error::Integer(value.clone())))
            .collect::<Result<Vec<i8>, _>>()?;
        project.settings.controls.values = control_values[..5]
            .iter()
            .map(|value| parse_f64(value))
            .collect::<Result<Vec<_>, _>>()?;
        project.settings.threshold_v = project.settings.controls.values[3];
        project.settings.usb_error_count = self.optional_u64_value("USB_ErrorCount", 0)?;
        project.settings.export.target_path = nonempty(self.value("ExportFile")?);
        project.settings.export.format = export_format(self.u8_value("ExportFormat")?).into();
        project.settings.export.radix = radix(self.u8_value("ExportRadix")?).into();
        project.settings.print.caption = self.bool_value("PrintCaption")?;
        project.settings.print.caption_string = self.value("PrintCaptionString")?.into();
        project.settings.print.caption_type =
            caption_type(self.u8_value("PrintCaptionType")?).into();
        project.settings.print.date = self.bool_value("PrintDate")?;
        project.settings.print.measurements = self.bool_value("PrintMeasurements")?;
        project.settings.trigger.combine = format!("lpf:{}", self.value("CombineMode")?);

        project.groups.clear();
        project.interpreters.clear();
        for (index, record) in self.records("Group").enumerate() {
            if record.values.iter().all(String::is_empty) || record.values[0].is_empty() {
                continue;
            }
            let wires = parse_wires(&record.values[6])?;
            let interpreter_type = parse_i64(&record.values[9])?;
            let raw = serde_json::json!(record.values);
            if interpreter_type < 0 {
                project.groups.push(lp_project::Group {
                    id: format!("lpf-group-{index}"),
                    name: record.values[0].clone(),
                    wires,
                    radix: radix(
                        record.values[2]
                            .parse()
                            .map_err(|_| Error::Integer(record.values[2].clone()))?,
                    )
                    .into(),
                    signed: record.values[3].starts_with('-'),
                    wire_order: if record.values[3] == "-1" {
                        "msb_first"
                    } else {
                        "lsb_first"
                    }
                    .into(),
                    display_order: record.values[8].clone(),
                    style: if parse_bool(&record.values[7])? {
                        "analog"
                    } else {
                        "digital"
                    }
                    .into(),
                    color: "default".into(),
                    lpf_raw: Some(raw),
                });
            } else {
                project.interpreters.push(lp_project::Interpreter {
                    id: format!("lpf-interpreter-{index}"),
                    name: record.values[0].clone(),
                    kind: interpreter_kind(interpreter_type)?.into(),
                    wires,
                    radix: radix(
                        record.values[2]
                            .parse()
                            .map_err(|_| Error::Integer(record.values[2].clone()))?,
                    )
                    .into(),
                    style: if parse_bool(&record.values[7])? {
                        "analog"
                    } else {
                        "digital"
                    }
                    .into(),
                    color: "default".into(),
                    config: interpreter_config(interpreter_type, &record.values[10..]),
                    extra: None,
                    lpf_raw: Some(raw),
                });
            }
        }

        project.rows = self
            .records("Row")
            .enumerate()
            .filter(|(_, record)| record.values.iter().any(|value| !value.is_empty()))
            .map(|(index, record)| row_from_record(index, record))
            .collect::<Result<Vec<_>, _>>()?;
        project.columns = self
            .records("Column")
            .filter(|record| record.values.iter().any(|value| !value.is_empty()))
            .map(column_from_record)
            .collect::<Result<Vec<_>, _>>()?;
        let measurement_types = self.values("MeasurementType")?;
        let measurement_left = self.values("MeasurementLeftTerm")?;
        let measurement_right = self.values("MeasurementRightTerm")?;
        if measurement_types.len() != 4
            || measurement_left.len() != 4
            || measurement_right.len() != 4
        {
            return Err(Error::MeasurementCardinality);
        }
        for slot in 0..4 {
            project.measurements[slot].kind = measurement_kind(&measurement_types[slot])?.into();
            project.measurements[slot].left = cursor_term(&measurement_left[slot])?;
            project.measurements[slot].right = cursor_term(&measurement_right[slot])?;
            project.measurements[slot].source = control_values
                .get(4 + slot)
                .map(|value| format!("wire:{value}"))
                .unwrap_or_else(|| "wire:0".into());
        }

        let offsets = self.values("CursorOffsetSamples")?;
        let times = self.values("CursorOffsetTime")?;
        let tracks = self.values("CursorInterlock")?;
        if offsets.len() != 6 || times.len() != 6 || tracks.len() != 6 {
            return Err(Error::CursorCardinality);
        }
        for (index, cursor) in project.cursors.iter_mut().enumerate() {
            cursor.offset_samples = parse_i64(&offsets[index])?;
            cursor.offset_s = parse_f64(&times[index])?;
            let track = parse_i64(&tracks[index])?;
            cursor.tracks = (0..6)
                .contains(&track)
                .then(|| char::from(b'A' + u8::try_from(track).unwrap_or_default()));
            cursor.visible = index < usize::from(project.settings.options.cursor_qty);
        }

        project.x_unknown.insert(
            "lpf_records".into(),
            serde_json::Value::Array(
                self.records
                    .iter()
                    .map(|record| serde_json::json!({"key":record.key,"values":record.values}))
                    .collect(),
            ),
        );
        project
            .validate()
            .map_err(|error| Error::Project(error.to_string()))?;
        Ok(project)
    }

    fn values(&self, key: &str) -> Result<&[String], Error> {
        self.records
            .iter()
            .find(|record| record.key == key)
            .map(|record| record.values.as_slice())
            .ok_or_else(|| Error::MissingRecord(key.to_owned()))
    }

    fn bool_value(&self, key: &str) -> Result<bool, Error> {
        parse_bool(self.value(key)?)
    }

    fn optional_bool_value(&self, key: &str, default: bool) -> Result<bool, Error> {
        match self.records.iter().find(|record| record.key == key) {
            None => Ok(default),
            Some(record) if record.values.len() == 1 => parse_bool(&record.values[0]),
            Some(record) => Err(Error::Arity {
                key: key.into(),
                expected: 1,
                got: record.values.len(),
            }),
        }
    }

    fn integer_value(&self, key: &str) -> Result<i64, Error> {
        parse_i64(self.value(key)?)
    }

    fn float_value(&self, key: &str) -> Result<f64, Error> {
        parse_f64(self.value(key)?)
    }

    fn u8_value(&self, key: &str) -> Result<u8, Error> {
        let value = self.value(key)?;
        value.parse().map_err(|_| Error::Integer(value.into()))
    }

    fn u32_value(&self, key: &str) -> Result<u32, Error> {
        let value = self.value(key)?;
        value.parse().map_err(|_| Error::Integer(value.into()))
    }

    fn optional_u64_value(&self, key: &str, default: u64) -> Result<u64, Error> {
        match self.records.iter().find(|record| record.key == key) {
            None => Ok(default),
            Some(record) if record.values.len() == 1 => record.values[0]
                .parse()
                .map_err(|_| Error::Integer(record.values[0].clone())),
            Some(record) => Err(Error::Arity {
                key: key.into(),
                expected: 1,
                got: record.values.len(),
            }),
        }
    }
}

fn rate_index(hz: u64) -> Result<u8, Error> {
    const RATES: [u64; 20] = [
        1_000_000_000,
        500_000_000,
        250_000_000,
        200_000_000,
        100_000_000,
        50_000_000,
        20_000_000,
        10_000_000,
        5_000_000,
        2_000_000,
        1_000_000,
        500_000,
        200_000,
        100_000,
        50_000,
        20_000,
        10_000,
        5_000,
        2_000,
        1_000,
    ];
    RATES
        .iter()
        .position(|candidate| *candidate == hz)
        .and_then(|index| u8::try_from(index).ok())
        .ok_or(Error::UnsupportedRate(hz))
}

fn state_units(value: u8) -> &'static str {
    match value {
        0 => "khz",
        1 => "hz",
        _ => "index",
    }
}
fn export_format(value: u8) -> &'static str {
    match value {
        0 => "as_formatted",
        1 => "single_group",
        2 => "channel_bits",
        _ => "unknown",
    }
}
fn radix(value: u8) -> &'static str {
    match value {
        0 => "binary",
        1 => "decimal",
        2 => "hex",
        3 => "ascii",
        _ => "unknown",
    }
}
fn caption_type(value: u8) -> &'static str {
    match value {
        0 => "none",
        1 => "project",
        2 => "custom",
        _ => "unknown",
    }
}
fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_wires(value: &str) -> Result<Vec<u8>, Error> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|wire| wire.parse().map_err(|_| Error::Integer(wire.into())))
        .collect()
}

fn interpreter_kind(value: i64) -> Result<&'static str, Error> {
    [
        "i2c", "spi", "uart", "iso7816", "can", "parallel", "onewire",
    ]
    .get(usize::try_from(value).unwrap_or(usize::MAX))
    .copied()
    .ok_or(Error::InterpreterType(value))
}

fn interpreter_config(kind: i64, values: &[String]) -> serde_json::Value {
    serde_json::json!({"kind":interpreter_kind(kind).unwrap_or("unknown"), "slots":values})
}

fn row_from_record(index: usize, record: &Record) -> Result<lp_project::Row, Error> {
    let row_type = parse_i64(&record.values[6])?;
    Ok(lp_project::Row {
        id: format!("lpf-row-{index}"),
        kind: match row_type {
            1 => "signal",
            2 => "group",
            3 => "interpreter",
            _ => "unknown",
        }
        .into(),
        reference: record.values[0].clone(),
        parent: (record.values[2] != "-1" && !record.values[2].is_empty())
            .then(|| format!("lpf-row-{}", record.values[2])),
        height_px: record.values[3]
            .trim()
            .parse()
            .map_err(|_| Error::Integer(record.values[3].clone()))?,
        color_index: record.values[4]
            .parse::<i16>()
            .map_err(|_| Error::Integer(record.values[4].clone()))?
            .max(0) as u8,
        style: "digital".into(),
        color: "default".into(),
        expanded: parse_bool(&record.values[1])?,
        visible: parse_bool(&record.values[7])?,
    })
}

fn column_from_record(record: &Record) -> Result<lp_project::Column, Error> {
    Ok(lp_project::Column {
        kind: format!("lpf:{}:{}", record.values[0], record.values[1]),
        width_px: record.values[2]
            .trim()
            .parse()
            .map_err(|_| Error::Integer(record.values[2].clone()))?,
        visible: record.values.get(3).is_none_or(|value| value != "-1"),
    })
}

fn measurement_kind(value: &str) -> Result<&'static str, Error> {
    let index: usize = value.parse().map_err(|_| Error::Integer(value.into()))?;
    [
        "interval",
        "period",
        "frequency",
        "average_frequency",
        "transitions",
        "cycles",
        "duty_cycle",
    ]
    .get(index)
    .copied()
    .ok_or_else(|| Error::Integer(value.into()))
}

fn cursor_term(value: &str) -> Result<String, Error> {
    let index: u8 = value.parse().map_err(|_| Error::Integer(value.into()))?;
    Ok(if index < 6 {
        char::from(b'A' + index).to_string()
    } else {
        format!("lpf:{index}")
    })
}

fn parse_bool(value: &str) -> Result<bool, Error> {
    match value {
        "True" | "1" => Ok(true),
        "False" | "0" => Ok(false),
        _ => Err(Error::Boolean(value.into())),
    }
}

fn parse_i64(value: &str) -> Result<i64, Error> {
    value.parse().map_err(|_| Error::Integer(value.into()))
}

fn parse_f64(value: &str) -> Result<f64, Error> {
    value.parse().map_err(|_| Error::Float(value.into()))
}

pub fn load(path: impl AsRef<Path>) -> Result<Document, Error> {
    parse(&std::fs::read(path)?)
}

pub fn parse(bytes: &[u8]) -> Result<Document, Error> {
    let (decoded, _, had_errors) = WINDOWS_1252.decode(bytes);
    if had_errors {
        return Err(Error::InvalidWindows1252);
    }
    let records = parse_records(&decoded)?;
    let unknown_keys = records
        .iter()
        .filter(|record| !keys::is_known(&record.key))
        .map(|record| record.key.clone())
        .collect::<Vec<_>>();
    let sample_record = records
        .iter()
        .find(|record| record.key == "SampleData")
        .ok_or(Error::MissingSampleData)?;
    let sample_data = parse_sample_data(sample_record)?;
    Ok(Document {
        records,
        unknown_keys,
        sample_data,
    })
}

fn parse_records(input: &str) -> Result<Vec<Record>, Error> {
    let mut fields = input.split('\x11');
    let mut key = fields.next().ok_or(Error::Empty)?.to_owned();
    let mut values = Vec::new();
    let mut records = Vec::new();
    for field in fields {
        if key == "NotesString/" {
            if field.trim_end_matches(['\r', '\n']) == "/" {
                records.push(normalize_record(key, values)?);
                key = String::new();
                values = Vec::new();
            } else {
                values.push(field.to_owned());
            }
            continue;
        }
        if let Some((value, next)) = field.rsplit_once("\r\n")
            && is_key_token(next)
        {
            values.push(value.to_owned());
            records.push(normalize_record(key, values)?);
            key = next.to_owned();
            values = Vec::new();
        } else {
            values.push(field.trim_end_matches(['\r', '\n']).to_owned());
        }
    }
    if !key.is_empty() {
        records.push(normalize_record(key, values)?);
    }
    if records.is_empty() {
        return Err(Error::Empty);
    }
    Ok(records)
}

fn normalize_record(key: String, mut values: Vec<String>) -> Result<Record, Error> {
    let width = match key.as_str() {
        "Group" => Some(36),
        "Row" => Some(9),
        "Column" => Some(4),
        _ => None,
    };
    if let Some(width) = width {
        if values.len() > width {
            return Err(Error::Arity {
                key,
                expected: width,
                got: values.len(),
            });
        }
        values.resize(width, String::new());
    }
    Ok(Record { key, values })
}

fn is_key_token(value: &str) -> bool {
    value.starts_with(char::is_alphabetic)
        && value
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, ' ' | '/' | '_'))
}

fn parse_sample_data(record: &Record) -> Result<SampleData, Error> {
    if record.values.len() != 4 {
        return Err(Error::Arity {
            key: record.key.clone(),
            expected: 4,
            got: record.values.len(),
        });
    }
    let mut metadata = [0_u64; 4];
    for (target, value) in metadata.iter_mut().zip(&record.values[..3]) {
        *target = value.parse().map_err(|_| Error::Integer(value.clone()))?;
    }
    let (fourth, block) = record.values[3]
        .split_once("\r\n{\r\n")
        .ok_or(Error::MalformedSampleBlock)?;
    metadata[3] = fourth
        .parse()
        .map_err(|_| Error::Integer(fourth.to_owned()))?;
    let block = block
        .strip_suffix("\r\n}")
        .ok_or(Error::MalformedSampleBlock)?;
    let mut lines = block.lines().map(str::trim).filter(|line| !line.is_empty());
    let header = lines.next().ok_or(Error::MalformedSampleBlock)?;
    let wanted_header = (0..32)
        .map(|index| format!("D{index}"))
        .chain(["CLK1".to_owned(), "CLK2".to_owned(), "Count".to_owned()])
        .collect::<Vec<_>>()
        .join(",");
    if header != wanted_header {
        return Err(Error::SampleHeader);
    }
    let runs = lines
        .enumerate()
        .map(|(index, line)| parse_sample_run(index + 1, line))
        .collect::<Result<Vec<_>, _>>()?;
    if metadata[0] != 34 || metadata[1] != runs.len() as u64 {
        return Err(Error::SampleMetadata);
    }
    Ok(SampleData { metadata, runs })
}

fn parse_sample_run(line: usize, input: &str) -> Result<SampleRun, Error> {
    let fields = input.split(',').collect::<Vec<_>>();
    if fields.len() != 35 {
        return Err(Error::SampleRowWidth {
            line,
            got: fields.len(),
        });
    }
    let mut channels = [Level::Unknown; 34];
    for (target, value) in channels.iter_mut().zip(&fields[..34]) {
        *target = match *value {
            "0" => Level::Low,
            "1" => Level::High,
            "U" => Level::Unknown,
            other => return Err(Error::SampleLevel(other.to_owned())),
        };
    }
    let count = fields[34]
        .parse::<u64>()
        .map_err(|_| Error::Integer(fields[34].to_owned()))?;
    if count == 0 {
        return Err(Error::ZeroCount { line });
    }
    Ok(SampleRun { channels, count })
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("LPF is empty")]
    Empty,
    #[error("LPF is not valid Windows-1252")]
    InvalidWindows1252,
    #[error("record {key} expected {expected} fields, got {got}")]
    Arity {
        key: String,
        expected: usize,
        got: usize,
    },
    #[error("LPF has no SampleData record")]
    MissingSampleData,
    #[error("LPF has no {0} record")]
    MissingRecord(String),
    #[error("malformed SampleData block")]
    MalformedSampleBlock,
    #[error("unexpected SampleData header")]
    SampleHeader,
    #[error("SampleData metadata disagrees with its rows")]
    SampleMetadata,
    #[error("SampleData row {line} has {got} fields, expected 35")]
    SampleRowWidth { line: usize, got: usize },
    #[error("invalid sample level {0:?}")]
    SampleLevel(String),
    #[error("invalid integer {0:?}")]
    Integer(String),
    #[error("invalid floating-point value {0:?}")]
    Float(String),
    #[error("invalid boolean {0:?}")]
    Boolean(String),
    #[error("capture conversion failed: {0}")]
    Capture(String),
    #[error("project conversion failed: {0}")]
    Project(String),
    #[error("cursor records do not each contain six values")]
    CursorCardinality,
    #[error("control records are too short: {selections} selections, {values} values")]
    ControlCardinality { selections: usize, values: usize },
    #[error("measurement records do not each contain four values")]
    MeasurementCardinality,
    #[error("unsupported acquired sample rate {0} Hz")]
    UnsupportedRate(u64),
    #[error("unsupported interpreter type {0}")]
    InterpreterType(i64),
    #[error("SampleData row {line} has zero count")]
    ZeroCount { line: usize },
    #[error("expanded sample count overflow")]
    SampleCountOverflow,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn examples() -> Vec<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/vendor/examples");
        let mut paths = std::fs::read_dir(root)
            .unwrap_or_else(|error| panic!("read examples: {error}"))
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "LPF"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }

    #[test]
    fn all_17_examples_parse_with_zero_unknown_keys() {
        let paths = examples();
        assert_eq!(paths.len(), 17);
        for path in paths {
            let document =
                load(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert_eq!(
                document.unknown_keys,
                Vec::<String>::new(),
                "{}",
                path.display()
            );
            assert_eq!(document.records("Signals").count(), 1);
        }
    }

    #[test]
    fn sampledata_rows_and_counts_are_valid_for_every_example() {
        for path in examples() {
            let document =
                load(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert_eq!(
                document.sample_data.metadata[1],
                document.sample_data.runs.len() as u64
            );
            assert!(
                document.sample_data.expanded_len().unwrap_or_default()
                    >= document.sample_data.runs.len() as u64
            );
        }
    }

    #[test]
    fn record_shapes_match_corpus_contract() {
        for path in examples() {
            let document =
                load(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(
                document
                    .records("Group")
                    .all(|record| record.values.len() == 36)
            );
            assert!(
                document
                    .records("Row")
                    .all(|record| record.values.len() == 9)
            );
            assert!(
                document
                    .records("Column")
                    .all(|record| record.values.len() == 4)
            );
        }
    }

    #[test]
    fn converted_captures_match_every_source_run_bit_for_bit() {
        for path in examples() {
            let document =
                load(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let capture = document
                .to_capture(1)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert_eq!(
                capture.expanded_len(),
                document.sample_data.expanded_len().unwrap_or_default()
            );
            let mut source_position = 0_u64;
            for run in &document.sample_data.runs {
                let expected = run
                    .channels
                    .iter()
                    .enumerate()
                    .fold(0_u64, |bits, (channel, level)| {
                        bits | (u64::from(*level == Level::High) << channel)
                    });
                assert_eq!(
                    capture.sample_at(source_position),
                    Some(expected),
                    "{}",
                    path.display()
                );
                source_position += run.count;
            }
            assert_eq!(capture.trigger_sample, document.sample_data.metadata[2]);
            assert_eq!(capture.reference_sample, document.sample_data.metadata[3]);
        }
    }

    #[test]
    fn confirmed_project_fields_convert_for_all_examples() {
        for path in examples() {
            let document =
                load(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let project = document
                .to_project(
                    "2026-08-25T00:00:00-04:00",
                    Some(path.display().to_string()),
                )
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert_eq!(project.source.kind, lp_project::SourceKind::LpfImport);
            assert_eq!(project.source.unknown_keys, Vec::<String>::new());
            assert_eq!(project.signals.len(), 34);
            assert_eq!(project.cursors.len(), 6);
            assert_eq!(project.measurements.len(), 4);
            assert_eq!(project.settings.controls.selections.len(), 5);
            assert_eq!(project.settings.controls.values.len(), 5);
            let populated_groups = document
                .records("Group")
                .filter(|record| !record.values[0].is_empty())
                .count();
            assert_eq!(
                project.groups.len() + project.interpreters.len(),
                populated_groups,
                "{}",
                path.display()
            );
            assert_eq!(
                project.rows.len(),
                document
                    .records("Row")
                    .filter(|record| record.values.iter().any(|value| !value.is_empty()))
                    .count()
            );
            assert_eq!(
                project.columns.len(),
                document
                    .records("Column")
                    .filter(|record| record.values.iter().any(|value| !value.is_empty()))
                    .count()
            );
            assert_eq!(
                project.settings.sample.rate_hz,
                (1.0 / project
                    .capture
                    .as_ref()
                    .map_or(1.0, |capture| capture.sample_period_s))
                .round() as u64
            );
            assert_eq!(
                project
                    .capture
                    .as_ref()
                    .map(lp_project::Capture::expanded_len),
                document.sample_data.expanded_len().ok()
            );
            assert!(project.x_unknown.contains_key("lpf_records"));
            let encoded = serde_json::to_vec(&project)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(lp_project::Project::from_slice(&encoded).is_ok());
        }
    }
}
