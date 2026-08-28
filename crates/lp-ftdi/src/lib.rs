use anyhow::{Context, Result, bail};
use nusb::MaybeFuture;
use nusb::transfer::{ControlIn, ControlOut, ControlType, Recipient};
use std::time::Duration;

pub mod consts;
pub mod device;
pub mod nusb_io;
pub mod rx;

pub use consts::{PID_LA1034 as PRODUCT_ID, VID as VENDOR_ID};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BitMode {
    Reset = 0,
    AsyncBitbang = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaudEncoding {
    pub value: u16,
    pub index: u16,
    pub actual: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeInfo {
    pub serial: String,
    pub product: String,
    pub pins: u8,
}

/// Opens and claims the LogicPort interface without changing FPGA state or EEPROM.
pub fn probe() -> Result<ProbeInfo> {
    let info = nusb::list_devices()
        .wait()
        .context("enumerating USB devices")?
        .find(|d| d.vendor_id() == VENDOR_ID && d.product_id() == PRODUCT_ID)
        .context("LogicPort 0403:dc48 is not attached")?;
    let serial = info.serial_number().unwrap_or("unknown").to_owned();
    let product = info.product_string().unwrap_or("LogicPort").to_owned();
    let mut ftdi = device::FtdiDevice::open(device::Timeouts::default())
        .map_err(|error| anyhow::anyhow!(error))?;
    ftdi.set_bitmode(0x07, BitMode::AsyncBitbang)
        .map_err(|error| anyhow::anyhow!(error))?;
    let pin_result = ftdi.read_pins();
    let restore_result = ftdi.set_bitmode(0, BitMode::Reset);
    restore_result.map_err(|error| anyhow::anyhow!(error))?;
    let pins = pin_result.map_err(|error| anyhow::anyhow!(error))?;
    if serial == "unknown" {
        bail!("LogicPort opened but did not expose a serial number");
    }
    Ok(ProbeInfo {
        serial,
        product,
        pins,
    })
}

/// Encodes an FTDI BM baud divisor using libftdi's integer algorithm.
pub fn to_clkbits(baud: u32) -> Result<BaudEncoding> {
    ensure_nonzero(baud)?;
    const CLK: u64 = 48_000_000;
    const CLK_DIV: u64 = 16;
    const FRAC_CODE: [u32; 8] = [0, 3, 2, 4, 1, 5, 6, 7];
    let divisor_clock = CLK * 8 / CLK_DIV;
    let mut best = (divisor_clock + u64::from(baud) / 2) / u64::from(baud);
    best = best.clamp(2, 0x1ffff);
    let best = u32::try_from(best).context("baud divisor overflow")?;
    let mut encoded = (best >> 3) | (FRAC_CODE[(best & 7) as usize] << 14);
    encoded = match encoded {
        1 => 0,
        0x4001 => 1,
        other => other,
    };
    let actual = u32::try_from(divisor_clock / u64::from(best)).context("actual baud overflow")?;
    Ok(BaudEncoding {
        value: encoded as u16,
        index: (encoded >> 16) as u16,
        actual,
    })
}

pub fn baud_divisor(baud: u32) -> Result<(u16, u16, u32)> {
    let encoded = to_clkbits(baud)?;
    Ok((encoded.value, encoded.index, encoded.actual))
}

impl device::FtdiDevice<nusb_io::NusbIo> {
    pub fn open(timeouts: device::Timeouts) -> Result<Self, device::FtdiError> {
        Ok(Self::new(nusb_io::NusbIo::open()?, timeouts))
    }

    pub fn reset(&mut self) -> Result<(), device::FtdiError> {
        self.control_out(consts::SIO_RESET, 0, 0)
    }

    pub fn purge_rx(&mut self) -> Result<(), device::FtdiError> {
        self.control_out(consts::SIO_RESET, consts::SIO_RESET_PURGE_RX, 0)?;
        self.clear_rx();
        Ok(())
    }

    pub fn purge_tx(&mut self) -> Result<(), device::FtdiError> {
        self.control_out(consts::SIO_RESET, consts::SIO_RESET_PURGE_TX, 0)
    }

    pub fn set_latency_ms(&mut self, latency: u8) -> Result<(), device::FtdiError> {
        self.control_out(consts::SIO_SET_LATENCY, u16::from(latency), 0)
    }

    pub fn set_baud(&mut self, baud: u32) -> Result<u32, device::FtdiError> {
        let encoded = to_clkbits(baud).map_err(|error| device::FtdiError::Io(error.to_string()))?;
        self.control_out(consts::SIO_SET_BAUD, encoded.value, encoded.index)?;
        Ok(encoded.actual)
    }

    pub fn set_bitmode(&mut self, mask: u8, mode: BitMode) -> Result<(), device::FtdiError> {
        self.control_out(
            consts::SIO_SET_BITMODE,
            (u16::from(mode as u8) << 8) | u16::from(mask),
            0,
        )
    }

    pub fn read_pins(&mut self) -> Result<u8, device::FtdiError> {
        let response = self.control_in(consts::SIO_READ_PINS, 0, 0, 1)?;
        response
            .first()
            .copied()
            .ok_or_else(|| device::FtdiError::Io("empty FTDI pin response".to_owned()))
    }

    pub fn read_eeprom_word(&mut self, addr: u16) -> Result<u16, device::FtdiError> {
        let response = self.control_in(consts::SIO_READ_EEPROM, addr, 0, 2)?;
        let bytes: [u8; 2] = response
            .as_slice()
            .try_into()
            .map_err(|_| device::FtdiError::Io("invalid EEPROM word response".to_owned()))?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
    ) -> Result<(), device::FtdiError> {
        self.io_mut().control_out(
            ControlOut {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request,
                value,
                index,
                data: &[],
            },
            Duration::from_secs(5),
        )
    }

    fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, device::FtdiError> {
        self.io_mut().control_in(
            ControlIn {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request,
                value,
                index,
                length,
            },
            Duration::from_secs(5),
        )
    }
}

fn ensure_nonzero(baud: u32) -> Result<()> {
    if baud == 0 {
        bail!("baud must be non-zero");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::baud_divisor;

    #[test]
    fn baud_vectors_match_ftdi_bm() {
        let vectors = [
            (460_800, 0x4006, 0, 461_538),
            (3_000_000, 0, 0, 3_000_000),
            (2_000_000, 1, 0, 2_000_000),
            (1_500_000, 2, 0, 1_500_000),
            (9_600, 0x4138, 0, 9_600),
        ];
        for (baud, value, index, actual) in vectors {
            assert_eq!(
                baud_divisor(baud).unwrap_or_else(|e| panic!("{e}")),
                (value, index, actual)
            );
        }
    }
}
