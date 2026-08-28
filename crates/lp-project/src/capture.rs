use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CHANNEL_MASK: u64 = (1_u64 << 34) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Run {
    pub data: u64,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Capture {
    pub id: u32,
    pub seq: u64,
    pub sample_period_s: f64,
    pub trigger_sample: u64,
    pub reference_sample: u64,
    pub channels_acquired: u64,
    pub runs: Vec<Run>,
}

impl Capture {
    pub fn new(
        id: u32,
        sample_period_s: f64,
        trigger_sample: u64,
        runs: Vec<Run>,
    ) -> Result<Self, CaptureError> {
        if !sample_period_s.is_finite() || sample_period_s <= 0.0 {
            return Err(CaptureError::InvalidSamplePeriod);
        }
        if runs.is_empty() {
            return Err(CaptureError::Empty);
        }
        for (index, run) in runs.iter().enumerate() {
            if run.count == 0 {
                return Err(CaptureError::ZeroCount { index });
            }
            if run.data & !CHANNEL_MASK != 0 {
                return Err(CaptureError::ChannelBits { index });
            }
        }
        let expanded_len = runs.iter().try_fold(0_u64, |sum, run| {
            sum.checked_add(run.count)
                .ok_or(CaptureError::LengthOverflow)
        })?;
        if trigger_sample >= expanded_len {
            return Err(CaptureError::TriggerOutOfRange);
        }
        Ok(Self {
            id,
            seq: u64::from(id),
            sample_period_s,
            trigger_sample,
            reference_sample: trigger_sample,
            channels_acquired: CHANNEL_MASK,
            runs: normalize(runs),
        })
    }

    pub fn expanded_len(&self) -> u64 {
        self.runs.iter().map(|run| run.count).sum()
    }

    pub fn sample_at(&self, sample: u64) -> Option<u64> {
        let mut start = 0_u64;
        for run in &self.runs {
            let end = start.checked_add(run.count)?;
            if sample < end {
                return Some(run.data);
            }
            start = end;
        }
        None
    }

    pub fn edges(&self, channel: u8, from: u64, to: u64) -> Result<Vec<Edge>, CaptureError> {
        if channel >= 34 {
            return Err(CaptureError::InvalidChannel(channel));
        }
        let to = to.min(self.expanded_len());
        if from >= to {
            return Ok(Vec::new());
        }
        let mask = 1_u64 << channel;
        let mut edges = Vec::new();
        let mut position = 0_u64;
        let mut previous = None;
        for run in &self.runs {
            let level = run.data & mask != 0;
            if let Some(old) = previous
                && old != level
                && position >= from
                && position < to
            {
                edges.push(Edge {
                    sample: position,
                    rising: level,
                });
            }
            previous = Some(level);
            position += run.count;
            if position >= to {
                break;
            }
        }
        Ok(edges)
    }
}

fn normalize(runs: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for run in runs {
        if let Some(last) = out.last_mut()
            && last.data == run.data
        {
            last.count += run.count;
            continue;
        }
        out.push(run);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub sample: u64,
    pub rising: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CaptureError {
    #[error("capture has no runs")]
    Empty,
    #[error("sample period must be finite and positive")]
    InvalidSamplePeriod,
    #[error("run {index} has a zero count")]
    ZeroCount { index: usize },
    #[error("run {index} has bits outside the 34 channels")]
    ChannelBits { index: usize },
    #[error("expanded capture length overflow")]
    LengthOverflow,
    #[error("trigger sample is outside the capture")]
    TriggerOutOfRange,
    #[error("channel {0} is outside D0..D31/CLK1/CLK2")]
    InvalidChannel(u8),
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn normalizes_and_indexes_without_expansion() {
        let capture = Capture::new(
            1,
            1e-9,
            2,
            vec![
                Run { data: 0, count: 2 },
                Run { data: 0, count: 3 },
                Run { data: 1, count: 4 },
            ],
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            capture.runs,
            vec![Run { data: 0, count: 5 }, Run { data: 1, count: 4 }]
        );
        assert_eq!(capture.expanded_len(), 9);
        assert_eq!(capture.sample_at(4), Some(0));
        assert_eq!(capture.sample_at(5), Some(1));
        assert_eq!(capture.sample_at(9), None);
        assert_eq!(capture.edges(0, 0, 9).map(|v| v[0].sample), Ok(5));
    }

    #[test]
    fn json_round_trip_is_stable() {
        let capture = Capture::new(7, 1e-6, 1, vec![Run { data: 3, count: 2 }])
            .unwrap_or_else(|error| panic!("{error}"));
        let json = serde_json::to_string(&capture).unwrap_or_else(|error| panic!("{error}"));
        let decoded: Capture =
            serde_json::from_str(&json).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(decoded, capture);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn rle_normalization_and_sample_access_match_expanded_oracle(raw in prop::collection::vec((0_u64..=CHANNEL_MASK, 1_u16..64), 1..128)) {
            let runs: Vec<_> = raw.iter().map(|(data, count)| Run { data: *data, count: u64::from(*count) }).collect();
            let expanded: Vec<_> = raw.iter().flat_map(|(data, count)| std::iter::repeat_n(*data, usize::from(*count))).collect();
            let capture = Capture::new(1, 1e-9, 0, runs).unwrap_or_else(|error| panic!("{error}"));
            prop_assert_eq!(capture.expanded_len() as usize, expanded.len());
            for (sample, expected) in expanded.iter().enumerate() { prop_assert_eq!(capture.sample_at(sample as u64), Some(*expected)); }
            prop_assert!(capture.runs.windows(2).all(|pair| pair[0].data != pair[1].data));
        }
    }
}
