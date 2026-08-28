use pest::{Parser, iterators::Pair};
use pest_derive::Parser;

mod eval;
pub use eval::{Bindings, SearchMatch, execute};

#[derive(Parser)]
#[grammar = "search/query.pest"]
struct QueryParser;

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Pattern(Vec<PatternTerm>),
    Edge {
        channel: String,
        kind: EdgeKind,
        count: u64,
    },
    Value {
        group: String,
        comparison: Comparison,
        left: f64,
        right: Option<f64>,
    },
    Duration {
        comparison: Comparison,
        left: f64,
        right: Option<f64>,
        unit: Unit,
    },
    Gap {
        comparison: Comparison,
        value: f64,
        unit: Unit,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternTerm {
    pub channel: String,
    pub level: Level,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Low,
    High,
    Either,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Rising,
    Falling,
    Either,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    Eq,
    Ne,
    Lt,
    Gt,
    In,
    NotIn,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Samples,
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

pub fn parse(input: &str) -> Result<Query, SearchError> {
    let mut pairs = QueryParser::parse(Rule::query, input)
        .map_err(|error| SearchError::Syntax(error.to_string()))?;
    let query = pairs.next().ok_or(SearchError::Empty)?;
    let steps = query
        .into_inner()
        .filter(|pair| pair.as_rule() != Rule::EOI)
        .map(parse_step)
        .collect::<Result<_, _>>()?;
    Ok(Query { steps })
}

fn parse_step(pair: Pair<'_, Rule>) -> Result<Step, SearchError> {
    match pair.as_rule() {
        Rule::pattern => {
            let terms = pair
                .into_inner()
                .map(|assignment| {
                    let mut fields = assignment.into_inner();
                    let channel = text(&mut fields)?.to_owned();
                    let level = match text(&mut fields)?.to_ascii_lowercase().as_str() {
                        "0" => Level::Low,
                        "1" => Level::High,
                        _ => Level::Either,
                    };
                    Ok(PatternTerm { channel, level })
                })
                .collect::<Result<_, SearchError>>()?;
            Ok(Step::Pattern(terms))
        }
        Rule::edge => {
            let mut fields = pair.into_inner();
            let channel = text(&mut fields)?.to_owned();
            let kind = match text(&mut fields)? {
                "rising" => EdgeKind::Rising,
                "falling" => EdgeKind::Falling,
                _ => EdgeKind::Either,
            };
            let count = fields
                .next()
                .map(|value| integer(value.as_str()))
                .transpose()?
                .unwrap_or(1);
            Ok(Step::Edge {
                channel,
                kind,
                count,
            })
        }
        Rule::value => {
            let mut fields = pair.into_inner();
            let group = text(&mut fields)?.to_owned();
            let comparison = comparison(text(&mut fields)?)?;
            let left = numeric(text(&mut fields)?)?;
            let right = fields
                .next()
                .map(|value| numeric(value.as_str()))
                .transpose()?;
            validate_range(comparison, right)?;
            Ok(Step::Value {
                group,
                comparison,
                left,
                right,
            })
        }
        Rule::duration => {
            let fields: Vec<_> = pair.into_inner().collect();
            let comparison = comparison(fields[0].as_str())?;
            let left = numeric(fields[1].as_str())?;
            let (right, unit_index) = if fields.len() == 4 {
                (Some(numeric(fields[2].as_str())?), 3)
            } else {
                (None, 2)
            };
            validate_range(comparison, right)?;
            Ok(Step::Duration {
                comparison,
                left,
                right,
                unit: unit(fields[unit_index].as_str())?,
            })
        }
        Rule::gap => {
            let mut fields = pair.into_inner();
            let comparison = comparison(text(&mut fields)?)?;
            let value = numeric(text(&mut fields)?)?;
            let unit = unit(text(&mut fields)?)?;
            Ok(Step::Gap {
                comparison,
                value,
                unit,
            })
        }
        _ => Err(SearchError::Syntax("unexpected search step".into())),
    }
}

fn text<'a>(pairs: &mut impl Iterator<Item = Pair<'a, Rule>>) -> Result<&'a str, SearchError> {
    pairs
        .next()
        .map(|pair| pair.as_str())
        .ok_or_else(|| SearchError::Syntax("missing field".into()))
}
fn integer(value: &str) -> Result<u64, SearchError> {
    value.parse().map_err(|_| SearchError::Number(value.into()))
}
fn numeric(value: &str) -> Result<f64, SearchError> {
    if let Some(hex) = value.strip_prefix("0x") {
        return u64::from_str_radix(hex, 16)
            .map(|v| v as f64)
            .map_err(|_| SearchError::Number(value.into()));
    }
    if let Some(binary) = value.strip_prefix("0b") {
        return u64::from_str_radix(binary, 2)
            .map(|v| v as f64)
            .map_err(|_| SearchError::Number(value.into()));
    }
    value.parse().map_err(|_| SearchError::Number(value.into()))
}
fn comparison(value: &str) -> Result<Comparison, SearchError> {
    Ok(match value {
        "==" => Comparison::Eq,
        "!=" => Comparison::Ne,
        "<" => Comparison::Lt,
        ">" => Comparison::Gt,
        "in" => Comparison::In,
        "notin" => Comparison::NotIn,
        _ => return Err(SearchError::Syntax("invalid comparison".into())),
    })
}
fn unit(value: &str) -> Result<Unit, SearchError> {
    Ok(match value {
        "samples" | "sample" => Unit::Samples,
        "s" => Unit::Seconds,
        "ms" => Unit::Milliseconds,
        "us" => Unit::Microseconds,
        "ns" => Unit::Nanoseconds,
        _ => return Err(SearchError::Syntax("invalid unit".into())),
    })
}
fn validate_range(comparison: Comparison, right: Option<f64>) -> Result<(), SearchError> {
    if matches!(comparison, Comparison::In | Comparison::NotIn) != right.is_some() {
        return Err(SearchError::RangeArity);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SearchError {
    #[error("empty search query")]
    Empty,
    #[error("invalid search query: {0}")]
    Syntax(String),
    #[error("invalid number: {0}")]
    Number(String),
    #[error("in/notin requires exactly two values; other comparisons require one")]
    RangeArity,
    #[error("unknown signal or channel: {0}")]
    UnknownSignal(String),
    #[error("unknown group: {0}")]
    UnknownGroup(String),
    #[error("search result limit must be positive")]
    ZeroLimit,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_documented_search_corpus() {
        let query = parse("pat{ALE=1}; edge WR falling; val AD[7..0] in 0x50,0x60; dur > 2us")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(query.steps.len(), 4);
        assert!(
            matches!(&query.steps[1], Step::Edge { channel, kind: EdgeKind::Falling, count: 1 } if channel == "WR")
        );
        assert!(
            matches!(&query.steps[2], Step::Value { left, right: Some(right), .. } if *left == 80.0 && *right == 96.0)
        );
    }
    #[test]
    fn parses_counts_wildcards_binary_and_gap() {
        let query = parse("pat{D0=x,CLK1=0};edge D31 either*12;gap >= 2 samples");
        assert!(query.is_err());
        let query = parse("pat{D0=x,CLK1=0};edge D31 either*12;gap > 0b10 samples")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(query.steps.len(), 3);
    }
}
