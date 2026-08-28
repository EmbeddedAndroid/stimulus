use crate::ProtoError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleMode {
    Timing,
    Vendor2,
    Vendor3,
    State,
}

pub fn encode_mode(state_code: u8, flag_a: bool, flag_b: bool) -> Result<u8, ProtoError> {
    let base = [4, 5, 2, 3]
        .get(state_code as usize)
        .copied()
        .ok_or(ProtoError::InvalidStateCode(state_code))?;
    Ok(base | ((!flag_a as u8) << 3) | ((!flag_b as u8) << 4))
}

pub fn image_index(mode: SampleMode, rate_idx: u8, state_code: u8) -> Result<u8, ProtoError> {
    match mode {
        SampleMode::Timing => Ok(if rate_idx < 2 { 6 } else { 7 }),
        SampleMode::Vendor2 => Ok(0),
        SampleMode::Vendor3 => Ok(5),
        SampleMode::State => [1, 3, 2, 4]
            .get(state_code as usize)
            .copied()
            .ok_or(ProtoError::InvalidStateCode(state_code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_byte_vectors() {
        assert_eq!(encode_mode(0, true, false), Ok(0x14));
        assert_eq!(encode_mode(3, false, false), Ok(0x1b));
    }
    #[test]
    fn image_index_table() {
        assert_eq!(image_index(SampleMode::Timing, 1, 0), Ok(6));
        assert_eq!(image_index(SampleMode::Timing, 2, 0), Ok(7));
        assert_eq!(image_index(SampleMode::State, 0, 3), Ok(4));
    }
}
