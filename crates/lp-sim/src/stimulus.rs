use crate::seed::{SimSeed, StimulusId};

#[derive(Debug, Clone)]
pub struct Stimulus {
    seed: SimSeed,
    sample_hz: u64,
}

impl Stimulus {
    pub fn new(seed: SimSeed, sample_hz: u64) -> Self {
        Self {
            seed,
            sample_hz: sample_hz.max(1),
        }
    }
    pub fn bits_at(&self, sample: u64) -> u32 {
        match self.seed.stimulus {
            StimulusId::Clock => {
                let hz = self.param(1_000_000).max(1);
                let half = (self.sample_hz / (hz.saturating_mul(2))).max(1);
                u32::from((sample / half) & 1 != 0)
            }
            StimulusId::Count16 => (sample as u32) & 0xffff,
            StimulusId::Pattern => self.param(0xaaaa_5555) as u32,
            StimulusId::Burst => {
                if sample % 100 < 20 {
                    1
                } else {
                    0
                }
            }
            StimulusId::Edges => u32::from(!sample.is_multiple_of(2)),
            _ => 0,
        }
    }
    fn param(&self, default: u64) -> u64 {
        self.seed
            .params
            .get("value")
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_frequency_is_exact_over_one_second() -> Result<(), Box<dyn std::error::Error>> {
        let seed: SimSeed = "stim=clk:1000".parse()?;
        let s = Stimulus::new(seed, 100_000);
        let rises = (1..100_000)
            .filter(|i| s.bits_at(*i - 1) & 1 == 0 && s.bits_at(*i) & 1 != 0)
            .count();
        assert_eq!(rises, 1000);
        Ok(())
    }
    #[test]
    fn count16_increments() -> Result<(), Box<dyn std::error::Error>> {
        let seed: SimSeed = "stim=count16".parse()?;
        let s = Stimulus::new(seed, 1_000_000);
        assert_eq!(s.bits_at(0xffff), 0xffff);
        assert_eq!(s.bits_at(0x1_0000), 0);
        Ok(())
    }
}
