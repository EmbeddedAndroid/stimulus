use crate::{
    clock::Clock,
    fpga::{ConfigureError, ConfigureOutcome},
    link::{DevError, DevStats, Link},
    transcript::DeviceIdentity,
    transport::Transport,
};
use lp_ftdi::consts::{SIO_RESET, SIO_RESET_PURGE_RX};
use lp_proto::{addr::Addr, regs};

pub trait LogicPortDevice: Send {
    fn read(&mut self, addr: Addr, len: u16) -> Result<Vec<u8>, DeviceError>;
    fn write(&mut self, addr: Addr, data: &[u8]) -> Result<(), DeviceError>;
    fn write_checked(
        &mut self,
        addr: Addr,
        data: &[u8],
        next: Option<Addr>,
    ) -> Result<(), DeviceError> {
        let _ = next;
        self.write(addr, data)
    }
    fn write_checked_sequence(&mut self, writes: &[(Addr, Vec<u8>)]) -> Result<(), DeviceError> {
        for (addr, data) in writes {
            self.write(*addr, data)?;
        }
        Ok(())
    }
    fn pins(&mut self) -> Result<u8, DeviceError>;
    fn configure_fpga(
        &mut self,
        image: &[u8],
        idx: u8,
        force: bool,
    ) -> Result<ConfigureOutcome, DeviceError>;
    fn probe_configured(&mut self) -> Result<Configured, DeviceError>;
    fn recover(&mut self) -> Result<(), DeviceError>;
    /// Flush any stale readback backlog before an acquisition (capture-boundary
    /// prevention against the sustained-load wedge). Default no-op for devices
    /// without a host-side read-ahead pool.
    fn flush_input(&mut self) {}
    fn stats(&self) -> DevStats;
    fn identity(&self) -> DeviceIdentity;

    fn read8(&mut self, addr: Addr) -> Result<u8, DeviceError> {
        one(self.read(addr, 1)?)
    }
    fn read16(&mut self, addr: Addr) -> Result<u16, DeviceError> {
        match self.read(addr, 2)?.as_slice() {
            [a, b] => Ok(u16::from_le_bytes([*a, *b])),
            _ => Err(DeviceError::ShortRegister),
        }
    }
    fn read32(&mut self, addr: Addr) -> Result<u32, DeviceError> {
        match self.read(addr, 4)?.as_slice() {
            [a, b, c, d] => Ok(u32::from_le_bytes([*a, *b, *c, *d])),
            _ => Err(DeviceError::ShortRegister),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Configured {
    pub pins: u8,
    pub image_id: u8,
    pub version: u16,
    pub configured: bool,
}

impl<T: Transport, C: Clock + Send> LogicPortDevice for Link<T, C> {
    fn read(&mut self, addr: Addr, len: u16) -> Result<Vec<u8>, DeviceError> {
        Link::read(self, addr, len).map_err(Into::into)
    }
    fn write(&mut self, addr: Addr, data: &[u8]) -> Result<(), DeviceError> {
        Link::write(self, addr, data).map_err(Into::into)
    }
    fn write_checked(
        &mut self,
        addr: Addr,
        data: &[u8],
        next: Option<Addr>,
    ) -> Result<(), DeviceError> {
        Link::write_checked(self, addr, data, next).map_err(Into::into)
    }
    fn write_checked_sequence(&mut self, writes: &[(Addr, Vec<u8>)]) -> Result<(), DeviceError> {
        Link::write_checked_sequence(self, writes).map_err(Into::into)
    }
    fn pins(&mut self) -> Result<u8, DeviceError> {
        Link::pins(self).map_err(Into::into)
    }
    fn configure_fpga(
        &mut self,
        image: &[u8],
        idx: u8,
        force: bool,
    ) -> Result<ConfigureOutcome, DeviceError> {
        Link::configure_fpga(self, image, idx, force).map_err(Into::into)
    }
    fn probe_configured(&mut self) -> Result<Configured, DeviceError> {
        let pins = Link::pins(self)?;
        let image_id = one(Link::read(self, regs::ctrl::IMAGE_ID, 1)?)?;
        let version = match Link::read(self, regs::ctrl::VERSION, 2)?.as_slice() {
            [a, b] => u16::from_le_bytes([*a, *b]),
            _ => return Err(DeviceError::ShortRegister),
        };
        Ok(Configured {
            pins,
            image_id,
            version,
            configured: pins & 0x10 != 0 && image_id & 0x10 != 0,
        })
    }
    fn recover(&mut self) -> Result<(), DeviceError> {
        let wanted = self.configured_idx.map(|idx| idx | 0x10);
        for attempt in 0..self.config.reopen_attempts {
            self.transport.reopen()?;
            self.transport
                .control_out(SIO_RESET, SIO_RESET_PURGE_RX, 0)?;
            self.invalidate();
            if wanted.is_none()
                || Link::read(self, regs::ctrl::IMAGE_ID, 1)
                    .ok()
                    .and_then(|v| v.first().copied())
                    == wanted
            {
                return Ok(());
            }
            if attempt + 1 < self.config.reopen_attempts {
                self.clock.sleep(self.config.reopen_gap);
            }
        }
        Err(DeviceError::FlushFailed {
            attempts: self.config.reopen_attempts,
        })
    }
    fn flush_input(&mut self) {
        self.purge_input();
    }
    fn stats(&self) -> DevStats {
        Link::stats(self)
    }
    fn identity(&self) -> DeviceIdentity {
        self.transport.identity()
    }
}

pub struct Regs<'a> {
    device: &'a mut dyn LogicPortDevice,
}
impl<'a> Regs<'a> {
    pub fn new(device: &'a mut dyn LogicPortDevice) -> Self {
        Self { device }
    }
    pub fn status(&mut self) -> Result<u8, DeviceError> {
        self.device.read8(regs::ctrl::STATUS)
    }
    pub fn image_id(&mut self) -> Result<u8, DeviceError> {
        self.device.read8(regs::ctrl::IMAGE_ID)
    }
    pub fn version(&mut self) -> Result<u16, DeviceError> {
        self.device.read16(regs::ctrl::VERSION)
    }
    pub fn arm(&mut self, enabled: bool) -> Result<(), DeviceError> {
        self.device.write(regs::ctrl::ARM, &[u8::from(enabled)])
    }
    pub fn reset(&mut self) -> Result<(), DeviceError> {
        self.device.write(regs::ctrl::RESET, &[1])
    }
}

fn one(bytes: Vec<u8>) -> Result<u8, DeviceError> {
    bytes.first().copied().ok_or(DeviceError::ShortRegister)
}

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error(transparent)]
    Link(#[from] DevError),
    #[error(transparent)]
    Configure(#[from] ConfigureError),
    #[error(transparent)]
    Transport(#[from] crate::transport::TransportError),
    #[error("short register response")]
    ShortRegister,
    #[error("device recovery failed after {attempts} attempts")]
    FlushFailed { attempts: u8 },
}
