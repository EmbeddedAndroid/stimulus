pub const VID: u16 = 0x0403;
pub const PID_LA1034: u16 = 0xdc48;
pub const EP_OUT: u8 = 0x02;
pub const EP_IN: u8 = 0x81;
pub const PKT: usize = 64;

pub const SIO_RESET: u8 = 0x00;
pub const SIO_SET_BAUD: u8 = 0x03;
pub const SIO_SET_LATENCY: u8 = 0x09;
pub const SIO_SET_BITMODE: u8 = 0x0b;
pub const SIO_READ_PINS: u8 = 0x0c;
pub const SIO_READ_EEPROM: u8 = 0x90;

pub const SIO_RESET_PURGE_RX: u16 = 1;
pub const SIO_RESET_PURGE_TX: u16 = 2;

#[cfg(test)]
mod tests {
    #[test]
    fn no_eeprom_write_request_constant() {
        let source = include_str!("consts.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(!source.contains("write_eeprom"));
        assert!(!source.contains("erase_eeprom"));
        assert!(!source.contains("0x91"));
        assert!(!source.contains("0x92"));
    }
}
