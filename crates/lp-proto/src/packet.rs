use crate::ProtoError;

pub const READ: u8 = 0xc2;
pub const WRITE: u8 = 0xc1;
pub const BANK: u8 = 0xc3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Read { addr: u16, len: u32 },
    Write { addr: u16, len: u32 },
    Bank(u16),
}

impl Cmd {
    pub const fn opcode(self) -> u8 {
        match self {
            Self::Read { .. } => READ,
            Self::Write { .. } => WRITE,
            Self::Bank(_) => BANK,
        }
    }
}

pub fn encode(cmd: Cmd) -> Result<[u8; 5], ProtoError> {
    let (opcode, addr, len) = match cmd {
        Cmd::Read { addr, len } => (READ, addr, len),
        Cmd::Write { addr, len } => (WRITE, addr, len),
        Cmd::Bank(bank) => (BANK, bank, 1),
    };
    if !(1..=65_536).contains(&len) {
        return Err(ProtoError::InvalidLength(len));
    }
    let biased = (len - 1) as u16;
    Ok([
        opcode,
        addr as u8,
        (addr >> 8) as u8,
        biased as u8,
        (biased >> 8) as u8,
    ])
}

pub const fn decode_biased_len(packet: [u8; 5]) -> u32 {
    u16::from_le_bytes([packet[3], packet[4]]) as u32 + 1
}

pub fn expected_response_len(cmd: Cmd) -> Result<usize, ProtoError> {
    match cmd {
        Cmd::Read { len, .. } if (1..=65_536).contains(&len) => Ok(3 + len as usize),
        Cmd::Write { len, .. } if (1..=65_536).contains(&len) => Ok(3),
        Cmd::Bank(_) => Ok(3),
        Cmd::Read { len, .. } | Cmd::Write { len, .. } => Err(ProtoError::InvalidLength(len)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RespHeader {
    pub opcode: u8,
    pub pktno: u16,
}

pub const fn parse_header(bytes: [u8; 3]) -> RespHeader {
    RespHeader {
        opcode: bytes[0],
        pktno: u16::from_be_bytes([bytes[1], bytes[2]]),
    }
}

pub fn check_header(cmd: Cmd, header: RespHeader) -> Result<(), ProtoError> {
    let expected = cmd.opcode();
    if header.opcode != expected {
        return Err(ProtoError::Opcode {
            expected,
            got: header.opcode,
        });
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PktNo {
    next: Option<u16>,
}

impl PktNo {
    pub const fn expected(self) -> Option<u16> {
        self.next
    }

    pub fn check(&mut self, got: u16) -> Result<(), ProtoError> {
        if let Some(expected) = self.next
            && got != expected
        {
            return Err(ProtoError::PacketNumber { expected, got });
        }
        self.next = Some(got.wrapping_add(1));
        Ok(())
    }

    pub fn resync(&mut self) {
        self.next = None;
    }

    pub fn reset_to_zero(&mut self) {
        self.next = Some(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn encode_vectors() {
        let vectors = [
            (Cmd::Read { addr: 1, len: 9 }, [0xc2, 1, 0, 8, 0]),
            (Cmd::Write { addr: 3, len: 5 }, [0xc1, 3, 0, 4, 0]),
            (Cmd::Bank(0x20), [0xc3, 0x20, 0, 0, 0]),
            (
                Cmd::Read {
                    addr: 0x156f,
                    len: 2047,
                },
                [0xc2, 0x6f, 0x15, 0xfe, 0x07],
            ),
            (
                Cmd::Read {
                    addr: 0,
                    len: 65_536,
                },
                [0xc2, 0, 0, 0xff, 0xff],
            ),
        ];
        for (cmd, wanted) in vectors {
            assert_eq!(encode(cmd), Ok(wanted));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn proptest_len_bias_roundtrip(addr in any::<u16>(), len in 1u32..=65_536) {
            let packet = encode(Cmd::Read { addr, len })?;
            prop_assert_eq!(decode_biased_len(packet), len);
        }
    }

    #[test]
    fn header_and_pktno_wraps() {
        assert_eq!(
            parse_header([0xc2, 0xab, 0xcd]),
            RespHeader {
                opcode: 0xc2,
                pktno: 0xabcd
            }
        );
        let mut seq = PktNo::default();
        seq.check(0xffff).unwrap_or_else(|e| panic!("{e}"));
        seq.check(0).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            seq.check(2),
            Err(ProtoError::PacketNumber {
                expected: 1,
                got: 2
            })
        );
    }
}
