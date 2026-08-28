#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Addr(pub u32);

impl Addr {
    pub const fn new(bank: u16, off: u16) -> Self {
        Self(((bank as u32) << 16) | off as u32)
    }

    pub const fn bank(self) -> u16 {
        (self.0 >> 16) as u16
    }

    pub const fn off(self) -> u16 {
        self.0 as u16
    }

    pub const fn offset(self, bytes: u16) -> Self {
        Self(self.0 + bytes as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::Addr;

    #[test]
    fn address_roundtrip() {
        let addr = Addr::new(0x2000, 0x156f);
        assert_eq!(addr, Addr(0x2000_156f));
        assert_eq!(addr.bank(), 0x2000);
        assert_eq!(addr.off(), 0x156f);
    }
}
