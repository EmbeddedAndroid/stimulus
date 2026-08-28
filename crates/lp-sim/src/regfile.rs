use lp_proto::{addr::Addr, regs};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegFile {
    bytes: BTreeMap<u32, u8>,
}

impl RegFile {
    pub fn cold() -> Self {
        let mut r = Self {
            bytes: BTreeMap::new(),
        };
        r.write(regs::ctrl::STATUS, &[0]);
        r.write(regs::ctrl::VERSION, &0x0100u16.to_le_bytes());
        r
    }
    pub fn warm(image: u8) -> Self {
        let mut r = Self::cold();
        r.write(regs::ctrl::IMAGE_ID, &[image | 0x10]);
        r
    }
    pub fn read(&self, addr: Addr, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| self.bytes.get(&(addr.0 + i as u32)).copied().unwrap_or(0))
            .collect()
    }
    pub fn write(&mut self, addr: Addr, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            self.bytes.insert(addr.0 + i as u32, *b);
        }
    }
    pub fn read8(&self, addr: Addr) -> u8 {
        self.read(addr, 1)[0]
    }
    pub fn read16(&self, addr: Addr) -> u16 {
        let v = self.read(addr, 2);
        u16::from_le_bytes([v[0], v[1]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn banked_bytes_and_typed_defaults() {
        let mut r = RegFile::warm(7);
        assert_eq!(r.read8(regs::ctrl::IMAGE_ID), 0x17);
        assert_eq!(r.read16(regs::ctrl::VERSION), 0x100);
        let a = Addr(0x2000_ffff);
        r.write(a, &[1, 2]);
        assert_eq!(r.read(a, 2), [1, 2]);
    }
}
