use crate::ProtoError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Data { bits: u32, clk1: bool, clk2: bool },
    Run { count: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub bits: u64,
    pub repeat: u64,
}

pub const fn decode_slot(b0: u8, b1: u8, b2: u8, b3: u8, flags: u8) -> Slot {
    if flags & 8 == 0 {
        Slot::Data {
            bits: u32::from_le_bytes([b0, b1, b2, b3]),
            clk1: flags & 2 != 0,
            clk2: flags & 4 != 0,
        }
    } else {
        Slot::Run {
            count: ((flags as u64 & 7) << 32)
                | ((b3 as u64) << 24)
                | ((b2 as u64) << 16)
                | ((b1 as u64) << 8)
                | b0 as u64,
        }
    }
}

pub fn slots_to_samples(slots: &[Slot]) -> Result<Vec<Sample>, ProtoError> {
    let mut samples: Vec<Sample> = Vec::new();
    for slot in slots {
        match *slot {
            Slot::Data { bits, clk1, clk2 } => samples.push(Sample {
                bits: u64::from(bits) | ((clk1 as u64) << 32) | ((clk2 as u64) << 33),
                repeat: 0,
            }),
            Slot::Run { count } => {
                if count >= 1 << 35 {
                    return Err(ProtoError::RunTooLong(count));
                }
                let prior = samples.last_mut().ok_or(ProtoError::RunWithoutData)?;
                prior.repeat = prior
                    .repeat
                    .checked_add(count)
                    .ok_or(ProtoError::RunTooLong(count))?;
            }
        }
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn data_and_run_35bit() {
        assert_eq!(
            decode_slot(0x78, 0x56, 0x34, 0x12, 6),
            Slot::Data {
                bits: 0x12345678,
                clk1: true,
                clk2: true
            }
        );
        assert_eq!(
            decode_slot(0x78, 0x56, 0x34, 0x12, 0x0f),
            Slot::Run {
                count: 0x0712345678
            }
        );
        let samples = slots_to_samples(&[decode_slot(1, 0, 0, 0, 2), Slot::Run { count: 7 }])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            samples,
            vec![Sample {
                bits: 1 | 1 << 32,
                repeat: 7
            }]
        );
    }
}
