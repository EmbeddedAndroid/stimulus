use super::{Provenance, RegWrite};
use crate::{ProtoError, regs};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateEntry {
    pub idx: u8,
    pub hz: u64,
    pub period_s: f64,
    pub mult: u8,
    pub r0: u8,
    pub r1: u8,
    pub image_timing: u8,
    pub compression_ok: bool,
    pub vendor_ui: bool,
    pub verified: bool,
}

pub static RATES: [RateEntry; 20] = [
    r(0, 1_000_000_000, 4, 0x00, 0x00, false, false),
    r(1, 500_000_000, 2, 0x00, 0x00, false, true),
    r(2, 250_000_000, 1, 0x00, 0x00, false, false),
    r(3, 200_000_000, 1, 0x01, 0x00, true, true),
    r(4, 100_000_000, 1, 0x11, 0x00, true, true),
    r(5, 50_000_000, 1, 0x11, 0x11, true, true),
    r(6, 20_000_000, 1, 0x11, 0x44, true, true),
    r(7, 10_000_000, 1, 0x21, 0x00, true, true),
    r(8, 5_000_000, 1, 0x21, 0x11, true, true),
    r(9, 2_000_000, 1, 0x21, 0x44, true, true),
    r(10, 1_000_000, 1, 0x31, 0x00, true, true),
    r(11, 500_000, 1, 0x31, 0x11, true, true),
    r(12, 200_000, 1, 0x31, 0x44, true, true),
    r(13, 100_000, 1, 0x41, 0x00, true, true),
    r(14, 50_000, 1, 0x41, 0x11, true, true),
    r(15, 20_000, 1, 0x41, 0x44, true, true),
    r(16, 10_000, 1, 0x51, 0x00, true, true),
    r(17, 5_000, 1, 0x51, 0x11, true, true),
    r(18, 2_000, 1, 0x51, 0x44, true, true),
    r(19, 1_000, 1, 0x61, 0x00, true, true),
];

const fn r(
    idx: u8,
    hz: u64,
    mult: u8,
    r0: u8,
    r1: u8,
    compression_ok: bool,
    vendor_ui: bool,
) -> RateEntry {
    RateEntry {
        idx,
        hz,
        period_s: 1.0 / hz as f64,
        mult,
        r0,
        r1,
        image_timing: if idx < 2 { 6 } else { 7 },
        compression_ok,
        vendor_ui,
        verified: false,
    }
}

pub fn rate_table() -> &'static [RateEntry; 20] {
    &RATES
}

pub fn encode_rate(
    hz: u64,
    compression: bool,
    extended_rates: bool,
) -> Result<[RegWrite; 2], ProtoError> {
    let entry = RATES
        .iter()
        .find(|e| e.hz == hz && (e.vendor_ui || extended_rates))
        .ok_or(ProtoError::UnknownRate(hz))?;
    if compression && !entry.compression_ok {
        return Err(ProtoError::CompressionRate);
    }
    let provenance = if entry.verified {
        Provenance::Verified
    } else {
        Provenance::Provisional
    };
    Ok([
        RegWrite {
            addr: regs::rate::R0,
            value: entry.r0,
            provenance,
        },
        RegWrite {
            addr: regs::rate::R1,
            value: entry.r1,
            provenance,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rate_table_20_rows_match_reference() {
        assert_eq!(RATES.len(), 20);
        assert_eq!((RATES[1].r0, RATES[1].r1, RATES[1].mult), (0, 0, 2));
        assert_eq!((RATES[18].r0, RATES[18].r1), (0x51, 0x44));
        assert_eq!((RATES[19].r0, RATES[19].r1), (0x61, 0));
    }
    #[test]
    fn refuses_compression_above_200mhz() {
        assert_eq!(
            encode_rate(500_000_000, true, false),
            Err(ProtoError::CompressionRate)
        );
        assert!(encode_rate(200_000_000, true, false).is_ok());
    }
}
