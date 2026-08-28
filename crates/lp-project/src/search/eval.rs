use super::{Comparison, EdgeKind, Level, Query, SearchError, Step, Unit};
use crate::Capture;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Bindings {
    signals: BTreeMap<String, u8>,
    groups: BTreeMap<String, Vec<u8>>,
}

impl Bindings {
    pub fn signal(mut self, name: impl Into<String>, channel: u8) -> Result<Self, SearchError> {
        if channel >= 34 {
            return Err(SearchError::UnknownSignal(name.into()));
        }
        self.signals.insert(fold(&name.into()), channel);
        Ok(self)
    }
    pub fn group(
        mut self,
        name: impl Into<String>,
        wires_lsb_first: Vec<u8>,
    ) -> Result<Self, SearchError> {
        if wires_lsb_first.is_empty() || wires_lsb_first.iter().any(|channel| *channel >= 34) {
            return Err(SearchError::UnknownGroup(name.into()));
        }
        self.groups.insert(fold(&name.into()), wires_lsb_first);
        Ok(self)
    }
    fn channel(&self, name: &str) -> Result<u8, SearchError> {
        let key = fold(name);
        if let Some(channel) = physical(&key) {
            return Ok(channel);
        }
        self.signals
            .get(&key)
            .copied()
            .ok_or_else(|| SearchError::UnknownSignal(name.into()))
    }
    fn group_wires(&self, name: &str) -> Result<Vec<u8>, SearchError> {
        if let Some(channel) = physical(&fold(name)) {
            return Ok(vec![channel]);
        }
        self.groups
            .get(&fold(name))
            .cloned()
            .ok_or_else(|| SearchError::UnknownGroup(name.into()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Copy)]
struct State {
    matched: SearchMatch,
    pending_gap: Option<(Comparison, f64, Unit)>,
}

pub fn execute(
    capture: &Capture,
    query: &Query,
    bindings: &Bindings,
    limit: usize,
) -> Result<Vec<SearchMatch>, SearchError> {
    if limit == 0 {
        return Err(SearchError::ZeroLimit);
    }
    let mut states = vec![State {
        matched: SearchMatch { start: 0, end: 0 },
        pending_gap: None,
    }];
    let mut has_primary = false;
    for step in &query.steps {
        match step {
            Step::Duration {
                comparison,
                left,
                right,
                unit,
            } => {
                states.retain(|state| {
                    compare(
                        span_value(state.matched, *unit, capture.sample_period_s),
                        *comparison,
                        *left,
                        *right,
                    )
                });
            }
            Step::Gap {
                comparison,
                value,
                unit,
            } => {
                for state in &mut states {
                    state.pending_gap = Some((*comparison, *value, *unit));
                }
            }
            _ => {
                let mut next = Vec::new();
                for state in states {
                    for candidate in candidates(capture, step, bindings, state.matched.end)? {
                        if let Some((comparison, value, unit)) = state.pending_gap {
                            let gap = candidate.start.saturating_sub(state.matched.end);
                            if !compare(
                                unit_value(gap, unit, capture.sample_period_s),
                                comparison,
                                value,
                                None,
                            ) {
                                continue;
                            }
                        }
                        let start = if has_primary {
                            state.matched.start
                        } else {
                            candidate.start
                        };
                        next.push(State {
                            matched: SearchMatch {
                                start,
                                end: candidate.end,
                            },
                            pending_gap: None,
                        });
                        if next.len() >= limit {
                            break;
                        }
                    }
                    if next.len() >= limit {
                        break;
                    }
                }
                states = next;
                has_primary = true;
            }
        }
    }
    Ok(states
        .into_iter()
        .take(limit)
        .map(|state| state.matched)
        .collect())
}

fn candidates(
    capture: &Capture,
    step: &Step,
    bindings: &Bindings,
    from: u64,
) -> Result<Vec<SearchMatch>, SearchError> {
    match step {
        Step::Pattern(terms) => {
            let terms = terms
                .iter()
                .map(|term| Ok((bindings.channel(&term.channel)?, term.level)))
                .collect::<Result<Vec<_>, SearchError>>()?;
            Ok(run_spans(capture)
                .filter(|(start, _, data)| {
                    *start >= from
                        && terms.iter().all(|(channel, level)| match level {
                            Level::Either => true,
                            Level::Low => data & (1_u64 << channel) == 0,
                            Level::High => data & (1_u64 << channel) != 0,
                        })
                })
                .map(span)
                .collect())
        }
        Step::Value {
            group,
            comparison,
            left,
            right,
        } => {
            let wires = bindings.group_wires(group)?;
            Ok(run_spans(capture)
                .filter(|(start, _, data)| {
                    if *start < from {
                        return false;
                    }
                    let value = wires
                        .iter()
                        .enumerate()
                        .fold(0_u64, |value, (bit, channel)| {
                            value | (((data >> channel) & 1) << bit)
                        }) as f64;
                    compare(value, *comparison, *left, *right)
                })
                .map(span)
                .collect())
        }
        Step::Edge {
            channel,
            kind,
            count,
        } => {
            let channel = bindings.channel(channel)?;
            let edges = capture
                .edges(channel, from, capture.expanded_len())
                .map_err(|_| SearchError::UnknownSignal(channel.to_string()))?;
            let filtered: Vec<_> = edges
                .into_iter()
                .filter(|edge| match kind {
                    EdgeKind::Rising => edge.rising,
                    EdgeKind::Falling => !edge.rising,
                    EdgeKind::Either => true,
                })
                .collect();
            let count = usize::try_from(*count).unwrap_or(usize::MAX);
            if count == 0 {
                return Ok(Vec::new());
            }
            Ok(filtered
                .chunks(count)
                .filter(|chunk| chunk.len() == count)
                .map(|chunk| SearchMatch {
                    start: chunk[0].sample,
                    end: chunk[count - 1].sample.saturating_add(1),
                })
                .collect())
        }
        Step::Duration { .. } | Step::Gap { .. } => Ok(Vec::new()),
    }
}

fn run_spans(capture: &Capture) -> impl Iterator<Item = (u64, u64, u64)> + '_ {
    let mut start = 0_u64;
    capture.runs.iter().map(move |run| {
        let span = (start, start + run.count, run.data);
        start += run.count;
        span
    })
}
fn span(value: (u64, u64, u64)) -> SearchMatch {
    SearchMatch {
        start: value.0,
        end: value.1,
    }
}
fn span_value(value: SearchMatch, unit: Unit, period: f64) -> f64 {
    unit_value(value.end.saturating_sub(value.start), unit, period)
}
fn unit_value(samples: u64, unit: Unit, period: f64) -> f64 {
    match unit {
        Unit::Samples => samples as f64,
        Unit::Seconds => samples as f64 * period,
        Unit::Milliseconds => samples as f64 * period * 1e3,
        Unit::Microseconds => samples as f64 * period * 1e6,
        Unit::Nanoseconds => samples as f64 * period * 1e9,
    }
}
fn compare(value: f64, comparison: Comparison, left: f64, right: Option<f64>) -> bool {
    match comparison {
        Comparison::Eq => value == left,
        Comparison::Ne => value != left,
        Comparison::Lt => value < left,
        Comparison::Gt => value > left,
        Comparison::In => right.is_some_and(|right| value >= left && value <= right),
        Comparison::NotIn => right.is_some_and(|right| value < left || value > right),
    }
}
fn fold(value: &str) -> String {
    value.to_ascii_lowercase()
}
fn physical(value: &str) -> Option<u8> {
    if value == "clk1" {
        return Some(32);
    }
    if value == "clk2" {
        return Some(33);
    }
    value
        .strip_prefix('d')?
        .parse::<u8>()
        .ok()
        .filter(|channel| *channel < 32)
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;
    use crate::Run;
    fn capture() -> Capture {
        Capture::new(
            1,
            1e-6,
            0,
            vec![
                Run { data: 0, count: 3 },
                Run { data: 3, count: 4 },
                Run { data: 2, count: 5 },
                Run { data: 0, count: 2 },
            ],
        )
        .unwrap_or_else(|e| panic!("{e}"))
    }
    #[test]
    fn evaluates_patterns_groups_edges_duration_and_gap_on_runs() {
        let bindings = Bindings::default()
            .signal("ALE", 0)
            .unwrap_or_else(|e| panic!("{e}"))
            .signal("WR", 1)
            .unwrap_or_else(|e| panic!("{e}"))
            .group("AD[7..0]", vec![0, 1])
            .unwrap_or_else(|e| panic!("{e}"));
        let query = parse("pat{ALE=1};dur > 3us;gap == 0 samples;val AD[7..0] in 0x2,0x3")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            execute(&capture(), &query, &bindings, 10),
            Ok(vec![SearchMatch { start: 3, end: 12 }])
        );
        let query = parse("edge ALE rising*1").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            execute(&capture(), &query, &bindings, 10).map(|m| m[0].start),
            Ok(3)
        );
    }
}
