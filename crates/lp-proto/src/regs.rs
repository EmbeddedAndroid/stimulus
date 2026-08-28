use crate::{ProtoError, addr::Addr};

pub mod ctrl {
    use super::Addr;
    pub const CMD_PORT: Addr = Addr(0x0010_0000);
    pub const AUX: Addr = Addr(0x0010_0001);
    pub const RESET: Addr = Addr(0x0010_0002);
    pub const ARM: Addr = Addr(0x0010_0003);
    pub const PRE_COUNT: Addr = Addr(0x0010_0004);
    pub const POST_COUNT: Addr = Addr(0x0010_0006);
    pub const DAC: Addr = Addr(0x0010_0008);
    pub const FORCE_FROM_PREFILL: Addr = Addr(0x0010_000a);
    pub const FORCE_STOP_POSTFILL: Addr = Addr(0x0010_000b);
    pub const FORCE_FROM_ARMED: Addr = Addr(0x0010_000c);
    pub const COMBINE: Addr = Addr(0x0010_000d);
    pub const STATUS: Addr = Addr(0x0010_0000);
    pub const WIRE_STATUS: Addr = Addr(0x0010_0001);
    pub const POST_COUNT_RD: Addr = Addr(0x0010_000a);
    pub const DDR_AUX: Addr = Addr(0x0010_000c);
    pub const VERSION: Addr = Addr(0x0010_000d);
    pub const IMAGE_ID: Addr = Addr(0x0010_000f);
}

pub mod cap {
    use super::Addr;
    pub const MODE: Addr = Addr(0x0100_0000);
    pub const FLAG1: Addr = Addr(0x0100_0001);
    pub const MASK_GATE: Addr = Addr(0x0100_0002);
    pub const CH_MASK: Addr = Addr(0x0100_0003);
    pub const MASK2: Addr = Addr(0x0100_0008);
    pub const TRIG_PAGE: Addr = Addr(0x0100_0000);
}

pub mod ram {
    use super::{Addr, ProtoError};
    pub const WR_PTR: Addr = Addr(0x0200_0000);
    pub const WR_PTR2: Addr = Addr(0x0200_0100);

    fn checked(n: u8, page: u16, first: u8) -> Result<Addr, ProtoError> {
        if !(1..=4).contains(&n) {
            return Err(ProtoError::InvalidBlock(n));
        }
        if page > 2047 {
            return Err(ProtoError::InvalidPage(page));
        }
        Ok(Addr(
            0x0200_0000 + (u32::from(first + n - 1) << 12) + u32::from(page),
        ))
    }
    pub fn block(n: u8, page: u16) -> Result<Addr, ProtoError> {
        checked(n, page, 1)
    }
    pub fn ddr_block(n: u8, page: u16) -> Result<Addr, ProtoError> {
        checked(n, page, 5)
    }
    pub fn flags(page: u16) -> Result<Addr, ProtoError> {
        if page > 2047 {
            return Err(ProtoError::InvalidPage(page));
        }
        Ok(Addr(0x0200_9000 + u32::from(page)))
    }
}

pub mod trig {
    use super::Addr;
    pub const A_BASE: Addr = Addr(0x0020_0000);
    pub const EDGE_P1: u16 = 0;
    pub const EDGE_P2: u16 = 5;
    pub const COUNT0: u16 = 10;
    pub const COUNT1: u16 = 14;
    pub const RANGE_DIR: u16 = 18;
    pub const M20: u16 = 20;
    pub const M22: u16 = 22;
    pub const M23: u16 = 23;
    pub const M24: u16 = 24;
    pub const RANGE_ARMED: u16 = 25;
    pub const RANGE_VALUE: u16 = 26;
    pub const RANGE_LEFT: u16 = 31;
    pub const RANGE_RIGHT: u16 = 36;
    pub const PAT_PA: u16 = 41;
    pub const PAT_PB: u16 = 46;

    pub const fn b_base(high_layout: bool) -> Addr {
        Addr(if high_layout {
            0x0060_0000
        } else {
            0x0040_0000
        })
    }
}

pub mod rate {
    use super::Addr;
    // The upper 16 bits of the logical address select the segment. The rate
    // bank is therefore C3 00 20, distinct from trigger A's C3 20 00.
    pub const R0: Addr = Addr(0x2000_0000);
    pub const R1: Addr = Addr(0x2000_0001);
}

pub mod freq {
    use super::Addr;
    pub const SRC: Addr = Addr(0x4000_0000);
    pub const READY: Addr = Addr(0x4000_0000);
    pub const VALUE: Addr = Addr(0x4000_0001);
    pub const AUX: Addr = Addr(0x4000_0005);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VendorWrite {
    pub addr: Addr,
    pub width: u16,
    pub observed: &'static [u8],
}

pub const VENDOR_WRITTEN: &[VendorWrite] = &[
    VendorWrite {
        addr: ctrl::RESET,
        width: 1,
        observed: &[1],
    },
    VendorWrite {
        addr: ctrl::ARM,
        width: 1,
        observed: &[0, 1],
    },
    VendorWrite {
        addr: ctrl::PRE_COUNT,
        width: 2,
        observed: &[],
    },
    VendorWrite {
        addr: ctrl::POST_COUNT,
        width: 2,
        observed: &[],
    },
    VendorWrite {
        addr: ctrl::DAC,
        width: 2,
        observed: &[],
    },
    VendorWrite {
        addr: cap::MODE,
        width: 1,
        observed: &[],
    },
    VendorWrite {
        addr: cap::CH_MASK,
        width: 5,
        observed: &[],
    },
    VendorWrite {
        addr: cap::MASK2,
        width: 5,
        observed: &[],
    },
    VendorWrite {
        addr: rate::R0,
        width: 1,
        observed: &[],
    },
    VendorWrite {
        addr: rate::R1,
        width: 1,
        observed: &[],
    },
    VendorWrite {
        addr: freq::SRC,
        width: 1,
        observed: &[],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_address_layout() {
        assert_eq!(ram::block(1, 0), Ok(Addr(0x0200_1000)));
        assert_eq!(ram::block(4, 2047), Ok(Addr(0x0200_47ff)));
        assert_eq!(ram::ddr_block(1, 0), Ok(Addr(0x0200_5000)));
        assert_eq!(ram::ddr_block(4, 2047), Ok(Addr(0x0200_87ff)));
        assert_eq!(ram::flags(2047), Ok(Addr(0x0200_97ff)));
    }

    #[test]
    fn high_registers_use_hardware_proven_wire_segments() {
        assert_eq!(rate::R0, Addr(0x2000_0000));
        assert_eq!(rate::R1, Addr(0x2000_0001));
        assert_eq!(rate::R0.bank(), 0x2000);
        assert_eq!(trig::A_BASE.bank(), 0x20);
        assert_ne!(rate::R0.bank(), trig::A_BASE.bank());
        assert_eq!(freq::SRC, Addr(0x4000_0000));
        assert_eq!(freq::VALUE, Addr(0x4000_0001));
        assert_eq!(freq::SRC.bank(), 0x4000);
    }

    #[test]
    fn allowlist_has_unique_address_width_pairs() {
        for (i, a) in VENDOR_WRITTEN.iter().enumerate() {
            assert!(
                !VENDOR_WRITTEN[..i]
                    .iter()
                    .any(|b| b.addr == a.addr && b.width == a.width)
            );
        }
    }
}
