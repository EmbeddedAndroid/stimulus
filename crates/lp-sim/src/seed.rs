use crate::faults::{Fault, FaultParseError};
use std::{collections::BTreeMap, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimSeed {
    pub rng_seed: u64,
    pub stimulus: StimulusId,
    pub params: BTreeMap<String, String>,
    pub start_state: StartState,
    pub faults: Vec<Fault>,
}

impl Default for SimSeed {
    fn default() -> Self {
        Self {
            rng_seed: 42,
            stimulus: StimulusId::Clock,
            params: BTreeMap::from([("hz".into(), "1000000".into())]),
            start_state: StartState::Cold,
            faults: vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartState {
    Cold,
    Warm { image: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StimulusId {
    Clock,
    Count16,
    Pattern,
    Burst,
    Edges,
    Serial,
    I2c,
    Spi,
    Can,
    OneWire,
    I2s,
}

impl FromStr for StimulusId {
    type Err = SeedError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "clk" => Ok(Self::Clock),
            "count16" => Ok(Self::Count16),
            "pat" => Ok(Self::Pattern),
            "burst" => Ok(Self::Burst),
            "edges" => Ok(Self::Edges),
            "serial" => Ok(Self::Serial),
            "i2c" => Ok(Self::I2c),
            "spi" => Ok(Self::Spi),
            "can" => Ok(Self::Can),
            "onewire" => Ok(Self::OneWire),
            "i2s" => Ok(Self::I2s),
            other => Err(SeedError::Stimulus(other.into())),
        }
    }
}

impl FromStr for SimSeed {
    type Err = SeedError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut seed = Self::default();
        for field in input.split(';').filter(|v| !v.is_empty()) {
            let (key, value) = field
                .split_once('=')
                .ok_or_else(|| SeedError::Field(field.into()))?;
            match key {
                "seed" => {
                    seed.rng_seed = value.parse().map_err(|_| SeedError::Number(value.into()))?
                }
                "stim" => {
                    let mut pieces = value.split(':');
                    let id = pieces
                        .next()
                        .ok_or_else(|| SeedError::Stimulus(value.into()))?;
                    seed.stimulus = id.parse()?;
                    seed.params.clear();
                    for (i, param) in pieces.enumerate() {
                        seed.params.insert(
                            if i == 0 {
                                "value".into()
                            } else {
                                format!("arg{i}")
                            },
                            param.into(),
                        );
                    }
                }
                "start" if value == "cold" => seed.start_state = StartState::Cold,
                "start" if value.starts_with("warm:") => {
                    seed.start_state = StartState::Warm {
                        image: value[5..]
                            .parse()
                            .map_err(|_| SeedError::Number(value.into()))?,
                    }
                }
                "fault" => {
                    seed.faults = value
                        .split(',')
                        .filter(|v| !v.is_empty())
                        .map(str::parse)
                        .collect::<Result<_, FaultParseError>>()?
                }
                other => return Err(SeedError::Key(other.into())),
            }
        }
        Ok(seed)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SeedError {
    #[error("malformed simulator seed field: {0}")]
    Field(String),
    #[error("unknown simulator seed key: {0}")]
    Key(String),
    #[error("invalid simulator number: {0}")]
    Number(String),
    #[error("unknown stimulus: {0}")]
    Stimulus(String),
    #[error(transparent)]
    Fault(#[from] FaultParseError),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_documented_seed() -> Result<(), SeedError> {
        let s: SimSeed = "seed=42;stim=clk:1000000;start=cold".parse()?;
        assert_eq!(s.rng_seed, 42);
        assert_eq!(s.stimulus, StimulusId::Clock);
        assert_eq!(s.params["value"], "1000000");
        assert_eq!(s.start_state, StartState::Cold);
        Ok(())
    }
    #[test]
    fn parses_faults_and_warm_image() -> Result<(), SeedError> {
        let s: SimSeed = "start=warm:7;fault=drop-pktno,stall-done".parse()?;
        assert_eq!(s.start_state, StartState::Warm { image: 7 });
        assert_eq!(s.faults, [Fault::DropPktno, Fault::StallDone]);
        Ok(())
    }
}
