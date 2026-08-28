use crate::{Capture, CaptureError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelSummary {
    pub channel: u8,
    pub transitions: u64,
    pub rising: u64,
    pub falling: u64,
    pub min_pulse_samples: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureSummary {
    pub capture_id: u32,
    pub rle_slots: usize,
    pub expanded_len: u64,
    pub duration_s: f64,
    pub compression_ratio: f64,
    pub channels: Vec<ChannelSummary>,
}

pub fn summarize(capture: &Capture) -> Result<CaptureSummary, CaptureError> {
    let expanded_len = capture.expanded_len();
    let mut channels = Vec::with_capacity(34);
    for channel in 0..34_u8 {
        let edges = capture.edges(channel, 0, expanded_len)?;
        let min_pulse_samples = edges
            .windows(2)
            .map(|pair| pair[1].sample - pair[0].sample)
            .min();
        channels.push(ChannelSummary {
            channel,
            transitions: edges.len() as u64,
            rising: edges.iter().filter(|edge| edge.rising).count() as u64,
            falling: edges.iter().filter(|edge| !edge.rising).count() as u64,
            min_pulse_samples,
        });
    }
    Ok(CaptureSummary {
        capture_id: capture.id,
        rle_slots: capture.runs.len(),
        expanded_len,
        duration_s: expanded_len as f64 * capture.sample_period_s,
        compression_ratio: expanded_len as f64 / capture.runs.len() as f64,
        channels,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureDiff {
    pub equal: bool,
    pub compared_samples: u64,
    pub first_divergence: Option<u64>,
    pub differing_channels: Vec<u8>,
}

pub fn diff(a: &Capture, b: &Capture, channels: u64) -> CaptureDiff {
    let compared_samples = a.expanded_len().min(b.expanded_len());
    let mut ai = 0_usize;
    let mut bi = 0_usize;
    let mut a_left = a.runs.first().map_or(0, |run| run.count);
    let mut b_left = b.runs.first().map_or(0, |run| run.count);
    let mut sample = 0_u64;
    while sample < compared_samples {
        let av = a.runs[ai].data;
        let bv = b.runs[bi].data;
        let mismatch = (av ^ bv) & channels;
        if mismatch != 0 {
            return CaptureDiff {
                equal: false,
                compared_samples: sample,
                first_divergence: Some(sample),
                differing_channels: (0..34)
                    .filter(|channel| mismatch & (1_u64 << channel) != 0)
                    .collect(),
            };
        }
        let advance = a_left.min(b_left).min(compared_samples - sample);
        sample += advance;
        a_left -= advance;
        b_left -= advance;
        if a_left == 0 && sample < compared_samples {
            ai += 1;
            a_left = a.runs[ai].count;
        }
        if b_left == 0 && sample < compared_samples {
            bi += 1;
            b_left = b.runs[bi].count;
        }
    }
    let same_length = a.expanded_len() == b.expanded_len();
    CaptureDiff {
        equal: same_length,
        compared_samples,
        first_divergence: (!same_length).then_some(compared_samples),
        differing_channels: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Run;
    fn cap(runs: Vec<Run>) -> Capture {
        Capture::new(1, 1e-6, 0, runs).unwrap_or_else(|e| panic!("{e}"))
    }
    #[test]
    fn summary_and_diff_work_without_expansion() {
        let a = cap(vec![Run { data: 0, count: 5 }, Run { data: 1, count: 5 }]);
        let b = cap(vec![
            Run { data: 0, count: 3 },
            Run { data: 0, count: 2 },
            Run { data: 3, count: 5 },
        ]);
        let summary = summarize(&a).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(summary.channels[0].transitions, 1);
        assert_eq!(summary.compression_ratio, 5.0);
        let result = diff(&a, &b, u64::MAX);
        assert_eq!(result.first_divergence, Some(5));
        assert_eq!(result.differing_channels, vec![1]);
        assert!(diff(&a, &b, 1).equal);
    }
}
