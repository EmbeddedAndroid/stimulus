use crate::ProtoError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireStatus {
    pub seen_high: u32,
    pub seen_low: u32,
    pub clk: u8,
}

pub fn parse_wire_status(bytes: &[u8]) -> Result<WireStatus, ProtoError> {
    if bytes.len() != 9 {
        return Err(ProtoError::WireStatusLength(bytes.len()));
    }
    Ok(WireStatus {
        seen_high: u32::from_le_bytes(
            bytes[0..4]
                .try_into()
                .map_err(|_| ProtoError::WireStatusLength(bytes.len()))?,
        ),
        seen_low: u32::from_le_bytes(
            bytes[4..8]
                .try_into()
                .map_err(|_| ProtoError::WireStatusLength(bytes.len()))?,
        ),
        clk: bytes[8],
    })
}

impl WireStatus {
    pub const fn clk1_seen_low(self) -> bool {
        self.clk & 1 != 0
    }
    pub const fn clk2_seen_low(self) -> bool {
        self.clk & 2 != 0
    }
    pub const fn clk1_seen_high(self) -> bool {
        self.clk & 4 != 0
    }
    pub const fn clk2_seen_high(self) -> bool {
        self.clk & 8 != 0
    }
    pub const fn clk1_level(self) -> bool {
        self.clk & 0x10 != 0
    }
    pub const fn clk2_level(self) -> bool {
        self.clk & 0x20 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clk_table() {
        let status =
            parse_wire_status(&[1, 2, 3, 4, 5, 6, 7, 8, 0x35]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(status.seen_high, 0x04030201);
        assert_eq!(status.seen_low, 0x08070605);
        assert!(status.clk1_seen_low() && !status.clk2_seen_low());
        assert!(status.clk1_seen_high() && !status.clk2_seen_high());
        assert!(status.clk1_level() && status.clk2_level());
    }
}
