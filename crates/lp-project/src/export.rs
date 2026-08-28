use crate::Capture;
use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcdTransition {
    pub time_ns: u64,
    pub channel: u8,
    pub high: bool,
}

pub fn parse_vcd_transitions(input: &str) -> Result<Vec<VcdTransition>, ExportError> {
    let mut time_ns = 0_u64;
    let mut transitions = Vec::new();
    for line in input.lines() {
        if let Some(time) = line.strip_prefix('#') {
            time_ns = time
                .parse()
                .map_err(|_| ExportError::InvalidVcd(line.into()))?;
        } else if let Some((level, id)) = line.split_at_checked(1)
            && matches!(level, "0" | "1")
            && id.starts_with('c')
        {
            let channel: u8 = id[1..]
                .parse()
                .map_err(|_| ExportError::InvalidVcd(line.into()))?;
            if channel >= 34 {
                return Err(ExportError::InvalidVcd(line.into()));
            }
            transitions.push(VcdTransition {
                time_ns,
                channel,
                high: level == "1",
            });
        }
    }
    if transitions.len() < 34 {
        return Err(ExportError::InvalidVcd(
            "missing initial channel values".into(),
        ));
    }
    Ok(transitions)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExportError {
    #[error("invalid VCD: {0}")]
    InvalidVcd(String),
}

pub fn csv_channel_bits(capture: &Capture) -> String {
    let mut out = format!(
        "SamplePeriod,{:.17}\nTriggerSample,{}\nCompressedData,{}\nSampleNumber",
        capture.sample_period_s,
        capture.trigger_sample,
        capture.runs.len() < capture.expanded_len() as usize
    );
    for channel in 0..32 {
        let _ = write!(out, ",D{channel}");
    }
    out.push_str(",CLK1,CLK2,Count\n");
    let mut sample = 0_u64;
    for run in &capture.runs {
        let _ = write!(out, "{sample}");
        for channel in 0..34 {
            let _ = write!(out, ",{}", (run.data >> channel) & 1);
        }
        let _ = writeln!(out, ",{}", run.count);
        sample += run.count;
    }
    out
}

pub fn vcd(capture: &Capture) -> String {
    let mut out = String::from("$timescale 1ns $end\n$scope module logicport $end\n");
    for channel in 0..34 {
        let name = if channel < 32 {
            format!("D{channel}")
        } else {
            format!("CLK{}", channel - 31)
        };
        let _ = writeln!(out, "$var wire 1 c{channel} {name} $end");
    }
    out.push_str("$upscope $end\n$enddefinitions $end\n");
    let mut sample = 0_u64;
    let mut previous = u64::MAX;
    for run in &capture.runs {
        let nanos = sample as f64 * capture.sample_period_s * 1e9;
        let _ = writeln!(out, "#{}", nanos.round() as u64);
        for channel in 0..34 {
            let mask = 1_u64 << channel;
            if (previous ^ run.data) & mask != 0 {
                let _ = writeln!(out, "{}c{channel}", (run.data >> channel) & 1);
            }
        }
        if sample == capture.trigger_sample {
            out.push_str("$comment trigger $end\n");
        }
        previous = run.data;
        if capture.trigger_sample > sample && capture.trigger_sample < sample + run.count {
            let trigger_ns = capture.trigger_sample as f64 * capture.sample_period_s * 1e9;
            let _ = writeln!(out, "#{}", trigger_ns.round() as u64);
            out.push_str("$comment trigger $end\n");
        }
        sample += run.count;
    }
    out
}

pub fn state_list(capture: &Capture) -> String {
    let mut out = String::from("Sample\tTime(s)\tValue\tCount\n");
    let mut sample = 0_u64;
    for run in &capture.runs {
        let _ = writeln!(
            out,
            "{sample}\t{:.12}\t0x{:09x}\t{}",
            sample as f64 * capture.sample_period_s,
            run.data,
            run.count
        );
        sample += run.count;
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpretedRow {
    pub interp: String,
    pub start_sample: u64,
    pub end_sample: u64,
    pub mnemonic: String,
    pub value: String,
    pub radix: String,
    pub error: String,
    pub tooltip: String,
}
pub fn interpreted(rows: &[InterpretedRow], sample_period_s: f64) -> String {
    let mut out = String::from(
        "interp,start_sample,end_sample,t_start,t_end,mnemonic,value,radix,error,tooltip\n",
    );
    for row in rows {
        let fields = [
            &row.interp,
            &row.mnemonic,
            &row.value,
            &row.radix,
            &row.error,
            &row.tooltip,
        ];
        let escaped: Vec<_> = fields.iter().map(|field| csv_field(field)).collect();
        let _ = writeln!(
            out,
            "{},{},{},{:.12},{:.12},{},{},{},{},{}",
            escaped[0],
            row.start_sample,
            row.end_sample,
            row.start_sample as f64 * sample_period_s,
            row.end_sample as f64 * sample_period_s,
            escaped[1],
            escaped[2],
            escaped[3],
            escaped[4],
            escaped[5]
        );
    }
    out
}
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Run;
    fn capture() -> Capture {
        Capture::new(
            1,
            1e-9,
            1,
            vec![Run { data: 0, count: 1 }, Run { data: 1, count: 2 }],
        )
        .unwrap_or_else(|e| panic!("{e}"))
    }
    #[test]
    fn exports_contain_rle_boundaries() {
        let c = capture();
        assert!(csv_channel_bits(&c).contains("CompressedData,true"));
        assert!(vcd(&c).contains("#1\n1c0"));
        assert!(state_list(&c).contains("0x000000001\t2"));
    }
    #[test]
    fn vcd_parse_back_matches_every_channel_transition() {
        let c = Capture::new(
            1,
            1e-9,
            2,
            vec![
                Run { data: 0, count: 2 },
                Run { data: 3, count: 4 },
                Run {
                    data: (1_u64 << 33) | 2,
                    count: 3,
                },
            ],
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let transitions = parse_vcd_transitions(&vcd(&c)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(transitions.len(), 38);
        assert!(transitions.contains(&VcdTransition {
            time_ns: 2,
            channel: 0,
            high: true
        }));
        assert!(transitions.contains(&VcdTransition {
            time_ns: 2,
            channel: 1,
            high: true
        }));
        assert!(transitions.contains(&VcdTransition {
            time_ns: 6,
            channel: 0,
            high: false
        }));
        assert!(transitions.contains(&VcdTransition {
            time_ns: 6,
            channel: 33,
            high: true
        }));
    }
    #[test]
    fn interpreted_export_has_fixed_columns_and_csv_escaping() {
        let output = interpreted(
            &[InterpretedRow {
                interp: "uart".into(),
                start_sample: 10,
                end_sample: 20,
                mnemonic: "DATA".into(),
                value: "A,\"B\"".into(),
                radix: "ascii".into(),
                error: String::new(),
                tooltip: "line\nbreak".into(),
            }],
            1e-6,
        );
        assert!(output.starts_with(
            "interp,start_sample,end_sample,t_start,t_end,mnemonic,value,radix,error,tooltip\n"
        ));
        assert!(output.contains("\"A,\"\"B\"\"\""));
        assert!(output.contains("\"line\nbreak\""));
    }
}
