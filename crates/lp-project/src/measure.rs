use crate::{Capture, CaptureError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementKind {
    Frequency,
    Period,
    Interval,
    Rate,
    Transitions,
    Cycles,
    Duty,
    InverseDuty,
    PositiveWidth,
    NegativeWidth,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub kind: MeasurementKind,
    pub value: Option<f64>,
    pub unit: String,
    pub samples: u64,
}

pub fn measure(
    capture: &Capture,
    kind: MeasurementKind,
    channel: u8,
    left: u64,
    right: u64,
) -> Result<Measurement, CaptureError> {
    let right = right.min(capture.expanded_len());
    let span = right.saturating_sub(left);
    let edges = capture.edges(channel, left, right)?;
    let transitions = edges.len() as u64;
    let rising: Vec<u64> = edges
        .iter()
        .filter(|edge| edge.rising)
        .map(|edge| edge.sample)
        .collect();
    let falling: Vec<u64> = edges
        .iter()
        .filter(|edge| !edge.rising)
        .map(|edge| edge.sample)
        .collect();
    let periods: Vec<u64> = rising.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let widths = |starts: &[u64], ends: &[u64]| -> Vec<u64> {
        starts
            .iter()
            .filter_map(|start| {
                ends.iter()
                    .copied()
                    .find(|end| end > start)
                    .map(|end| end - start)
            })
            .collect()
    };
    let positive = widths(&rising, &falling);
    let negative = widths(&falling, &rising);
    let average = |values: &[u64]| {
        (!values.is_empty()).then(|| values.iter().sum::<u64>() as f64 / values.len() as f64)
    };
    let seconds = span as f64 * capture.sample_period_s;
    let (value, unit) = match kind {
        MeasurementKind::Frequency => (
            average(&periods).map(|period| 1.0 / (period * capture.sample_period_s)),
            "Hz",
        ),
        MeasurementKind::Period => (
            average(&periods).map(|period| period * capture.sample_period_s),
            "s",
        ),
        MeasurementKind::Interval => ((span > 0).then_some(seconds), "s"),
        MeasurementKind::Rate => (
            (seconds > 0.0).then_some(transitions as f64 / seconds),
            "Hz",
        ),
        MeasurementKind::Transitions => (Some(transitions as f64), "count"),
        MeasurementKind::Cycles => (Some(periods.len() as f64), "count"),
        MeasurementKind::Duty => (
            match (average(&positive), average(&periods)) {
                (Some(high), Some(period)) => Some(high / period),
                _ => None,
            },
            "ratio",
        ),
        MeasurementKind::InverseDuty => (
            match (average(&positive), average(&periods)) {
                (Some(high), Some(period)) => Some(1.0 - high / period),
                _ => None,
            },
            "ratio",
        ),
        MeasurementKind::PositiveWidth => (
            average(&positive).map(|width| width * capture.sample_period_s),
            "s",
        ),
        MeasurementKind::NegativeWidth => (
            average(&negative).map(|width| width * capture.sample_period_s),
            "s",
        ),
    };
    Ok(Measurement {
        kind,
        value,
        unit: unit.to_owned(),
        samples: span,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Run;

    fn clock() -> Capture {
        Capture::new(
            1,
            1e-6,
            0,
            (0..8)
                .map(|i| Run {
                    data: i % 2,
                    count: 5,
                })
                .collect(),
        )
        .unwrap_or_else(|error| panic!("{error}"))
    }

    #[test]
    fn all_ten_measurement_types_have_defined_contracts() {
        let capture = clock();
        for kind in [
            MeasurementKind::Frequency,
            MeasurementKind::Period,
            MeasurementKind::Interval,
            MeasurementKind::Rate,
            MeasurementKind::Transitions,
            MeasurementKind::Cycles,
            MeasurementKind::Duty,
            MeasurementKind::InverseDuty,
            MeasurementKind::PositiveWidth,
            MeasurementKind::NegativeWidth,
        ] {
            let result =
                measure(&capture, kind, 0, 0, 40).unwrap_or_else(|error| panic!("{error}"));
            assert!(result.value.is_some(), "{kind:?}");
        }
        let frequency = measure(&capture, MeasurementKind::Frequency, 0, 0, 40)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(
            frequency
                .value
                .is_some_and(|value| (value - 100_000.0).abs() < 1e-9)
        );
        let duty = measure(&capture, MeasurementKind::Duty, 0, 0, 40)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(duty.value, Some(0.5));
    }
}
