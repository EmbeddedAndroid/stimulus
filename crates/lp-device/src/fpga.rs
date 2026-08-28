use crate::{
    clock::Clock,
    link::{DevError, Link},
    transport::Transport,
};
use lp_ftdi::{
    consts::{
        SIO_READ_PINS, SIO_RESET, SIO_RESET_PURGE_RX, SIO_RESET_PURGE_TX, SIO_SET_BAUD,
        SIO_SET_BITMODE, SIO_SET_LATENCY,
    },
    to_clkbits,
};
use lp_proto::regs;
use std::time::Duration;

/// Diagnostic override for the FTDI latency timer (ms). Vendor default is 4.
/// Read-ahead depth during the bit-bang configure phase (keeps the drain
/// well-behaved).
fn config_readahead_depth() -> usize {
    std::env::var("LP_CFG_DEPTH")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|d| *d >= 1)
        .unwrap_or(4)
}

/// Read-ahead depth during the FIFO command phase. Much higher than Windows'
/// 4 because Linux userspace re-arms IN transfers far slower than the Windows
/// kernel driver, so more must be outstanding to keep the FT245 drained.
fn command_readahead_depth() -> usize {
    std::env::var("LP_CMD_DEPTH")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|d| *d >= 1)
        .unwrap_or(64)
}

fn latency_ms() -> u16 {
    std::env::var("LP_LATENCY")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|v| *v >= 1 && *v <= 255)
        .unwrap_or(4)
}

const POLL: Duration = Duration::from_millis(1);
const DEADLINE: Duration = Duration::from_secs(1);
const IO_TIMEOUT: Duration = Duration::from_secs(1);
const DRAIN_TIMEOUT: Duration = Duration::from_millis(100);
const DRAIN_MAX: usize = 16_384;
pub(crate) const FIFO_TAIL_ZEROS: usize = 8_712;
const NCONFIG_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigureOutcome {
    pub warm: bool,
    pub id: u8,
    pub version: u16,
    pub elapsed: Duration,
    pub drained_bytes: usize,
}

impl<T: Transport, C: Clock> Link<T, C> {
    /// Return the FTDI interface to FIFO mode after an interrupted configure.
    pub fn abort_fpga_configure(&mut self) {
        let _ = self.set_bitmode(0, 0);
        let _ = self.drain_after_bitbang();
        let _ = self.purge_rx();
        let _ = self.transport.control_out(SIO_RESET, SIO_RESET_PURGE_TX, 0);
        self.invalidate();
    }

    pub fn pins(&mut self) -> Result<u8, ConfigureError> {
        self.transport
            .control_in(SIO_READ_PINS, 0, 0, 1)?
            .first()
            .copied()
            .ok_or(ConfigureError::EmptyPins)
    }

    pub fn configure_fpga(
        &mut self,
        image: &[u8],
        idx: u8,
        force: bool,
    ) -> Result<ConfigureOutcome, ConfigureError> {
        let started = self.clock.elapsed();
        let no_cfg_drain = std::env::var("LP_NO_CFG_DRAIN").as_deref() == Ok("1");
        self.transport.set_readahead_depth(config_readahead_depth());
        let baud = to_clkbits(460_800).map_err(|error| ConfigureError::Setup(error.to_string()))?;
        // The vendor Flush/open path establishes FIFO mode only when opening
        // the session. Repeating SIO_RESET before an in-session warm request
        // stalls image 6's parser and violates the same-image IMAGE_ID-only
        // short circuit.
        if self.configured_idx.is_none() || force {
            self.transport.control_out(SIO_RESET, 0, 0)?;
            self.transport
                .control_out(SIO_SET_LATENCY, latency_ms(), 0)?;
            self.transport
                .control_out(SIO_SET_BAUD, baud.value, baud.index)?;
            self.set_bitmode(0, 0)?;
            let _ = self.drain_after_bitbang()?;
            self.purge_rx()?;
            self.transport
                .control_out(SIO_RESET, SIO_RESET_PURGE_TX, 0)?;
        }
        // Do not send a FIFO bank-select to an FPGA that is visibly outside
        // user mode. It cannot acknowledge the command, and leaving that OUT
        // packet queued across the bit-bang transition makes the subsequent
        // post-configure parser startup nondeterministic.
        let initial_pins = self.pins()?;
        eprintln!("FPGA configure image {idx}: initial pins=0x{initial_pins:02x}");
        let mut observed_id = if initial_pins & 0x10 == 0 {
            None
        } else {
            match self
                .read(regs::ctrl::IMAGE_ID, 1)
                .map_err(ConfigureError::from)
            {
                Ok(bytes) => Some(
                    bytes
                        .first()
                        .copied()
                        .ok_or(ConfigureError::EmptyRegister)?,
                ),
                Err(ConfigureError::Device(DevError::Timeout { .. })) => {
                    self.invalidate();
                    None
                }
                Err(ConfigureError::Device(DevError::Transport(error)))
                    if error.0.contains("transfer was cancelled") =>
                {
                    self.invalidate();
                    None
                }
                Err(ConfigureError::Device(DevError::Protocol(_))) => {
                    // A process can inherit the delayed acknowledgement of a
                    // release write after resetting persistent bit-bang mode.
                    // Its opcode cannot be the response to this IMAGE_ID probe.
                    self.purge_rx()?;
                    self.invalidate();
                    None
                }
                Err(error) => return Err(error),
            }
        };
        // A different image is loaded (observed_id is Some but != target): no
        // 0x61 "open handshake" is needed - the probe already read the running
        // image id. Fall through to the release + cold-stream path below, which
        // asserts nCONFIG and re-primes the FIFO for the new image.
        if observed_id.is_none()
            && initial_pins & 0x10 != 0
            && !(force && self.configured_idx == Some(idx))
        {
            // Configured board whose first IMAGE_ID probe did not answer. The
            // vendor never writes 0x61 to "unlock" it; retry the read on a
            // freshly-invalidated bank so a real C3 10 is re-emitted. If it
            // still does not answer the parser genuinely lost session state and
            // the ProtocolUnresponsive guard below surfaces it for a deliberate
            // reconfigure.
            self.purge_rx()?;
            self.invalidate();
            observed_id = match self
                .read(regs::ctrl::IMAGE_ID, 1)
                .map_err(ConfigureError::from)
            {
                Ok(bytes) => bytes.first().copied(),
                Err(ConfigureError::Device(DevError::Timeout { .. })) => None,
                Err(ConfigureError::Device(DevError::Transport(error)))
                    if error.0.contains("transfer was cancelled") =>
                {
                    None
                }
                Err(error) => return Err(error),
            };
        }
        // A silent FIFO is not proof of an unconfigured FPGA.  In particular,
        // an interrupted command can leave user mode (DONE high) alive while
        // its parser stops answering.  Treat that as a recoverable connection
        // fault and preserve the running image; asserting nCONFIG here turns a
        // host-session fault into a power-cycle requirement.
        if observed_id.is_none() {
            let pins = self.pins()?;
            eprintln!("FPGA configure image {idx}: no IMAGE_ID, pins=0x{pins:02x}");
            if pins & 0x10 != 0 {
                // A single forced retry is permitted only when this Link
                // itself just streamed the requested image and its first
                // post-config verification stalled. In that narrow case the
                // target image is known, so asserting nCONFIG is safe and
                // avoids turning a recoverable parser race into a power-cycle.
                if !(force && self.configured_idx == Some(idx)) {
                    return Err(ConfigureError::ProtocolUnresponsive { pins });
                }
            }
        }
        if observed_id == Some(idx | 0x10) && !force {
            // Same image already loaded and its IMAGE_ID probe above answered
            // correctly, so the FIFO parser is already live for this session
            // (a new host session inherits the running parser; hardware-proven
            // on Windows: 500/500 command reads with no re-prime). The vendor
            // NEVER writes 0x61 here - the probe read IS the readiness proof.
            // The old new-session 0x61 "enable" only wedged the command FSM.
            self.configured_idx = Some(idx);
            // The parser is live and about to serve the command/readback phase,
            // which needs the deep IN pool (line 89 dropped it to the shallow
            // configure depth). Without this, a warm/same-image configure leaves
            // the readback starved -> the FT245 backs up and wedges under load.
            self.transport
                .set_readahead_depth(command_readahead_depth());
            return Ok(ConfigureOutcome {
                warm: true,
                id: idx | 0x10,
                version: 0,
                elapsed: self.clock.elapsed().saturating_sub(started),
                drained_bytes: 0,
            });
        }

        let mut drained_bytes = 0;
        let initial = if observed_id.is_some() {
            eprintln!("FPGA configure image {idx}: releasing live image {observed_id:?}");
            self.write_release(regs::ctrl::CMD_PORT)?;
            let mut last = 0xff;
            let mut released = false;
            for _ in 0..20 {
                self.purge_rx()?;
                self.set_bitmode(0, 0)?;
                self.set_bitmode(0x07, 1)?;
                self.transport.bulk_out(&[0], IO_TIMEOUT)?;
                last = self.pins()?;
                drained_bytes += self.drain_bitbang()?;
                if last & 0x10 == 0 {
                    released = true;
                    break;
                }
            }
            if !released {
                return Err(ConfigureError::ReleaseFailed { last_pins: last });
            }
            last
        } else {
            eprintln!("FPGA configure image {idx}: entering cold configuration path");
            self.transport.control_out(SIO_RESET, 0, 0)?;
            self.transport
                .control_out(SIO_SET_LATENCY, latency_ms(), 0)?;
            self.transport
                .control_out(SIO_SET_BAUD, baud.value, baud.index)?;
            self.purge_rx()?;
            self.transport
                .control_out(SIO_RESET, SIO_RESET_PURGE_TX, 0)?;
            let mut last = 0xff;
            let mut asserted = false;
            // A completed USB bulk transfer does not by itself prove that the
            // FT245 output latch advanced. Verify nCONFIG/DBUS2 went low before
            // releasing it; otherwise 0x04 merely preserves a false-high reset
            // and every subsequent INIT poll is misleading. Each attempt uses
            // the vendor's unchanged reset/mask/value sequence and is visible
            // in hardware transcripts.
            for _ in 0..NCONFIG_ATTEMPTS {
                self.set_bitmode(0, 0)?;
                self.purge_rx()?;
                self.transport
                    .control_out(SIO_RESET, SIO_RESET_PURGE_TX, 0)?;
                self.set_bitmode(0x07, 1)?;
                self.transport.bulk_out(&[0], IO_TIMEOUT)?;
                if !no_cfg_drain {
                    drained_bytes += self.drain_bitbang()?;
                }
                last = self.pins()?;
                if last & 0x04 == 0 {
                    asserted = true;
                    break;
                }
            }
            if !asserted {
                return Err(ConfigureError::ResetAssertFailed {
                    attempts: NCONFIG_ATTEMPTS,
                    last_pins: last,
                });
            }
            last
        };

        self.transport.bulk_out(&[0x04], IO_TIMEOUT)?;
        if !no_cfg_drain {
            drained_bytes += self.drain_bitbang()?;
        }
        self.wait_pin(0x08, ConfigureError::InitTimeout { last_pins: initial })?;
        // Vendor usbConfigureFPGA streams the image in 10000-byte FT_Write
        // chunks with NO bulk-IN drain between them (it only reads status pins
        // via FT_GetBitMode = a control transfer). Our extra bulk-IN drain
        // interleaved into the bit-bang stream disrupts config timing and
        // leaves the FPGA marginal.
        for chunk in image.chunks(10_000) {
            self.transport.bulk_out(chunk, IO_TIMEOUT)?;
            if !no_cfg_drain {
                drained_bytes += self.drain_bitbang()?;
            }
        }
        eprintln!("FPGA configure image {idx}: image stream complete");
        self.wait_pin(0x10, ConfigureError::DoneTimeout { last_pins: 0 })?;
        if !no_cfg_drain {
            drained_bytes += self.drain_bitbang()?;
        }
        // Cyclone passive-serial needs extra DCLK cycles AFTER CONF_DONE to run
        // the initialization phase before user mode. Stopping at DONE leaves the
        // logic uninitialized/marginal. Clock DIN=0, PROG_B=1 (0x04), toggling
        // DCLK (0x02) for LP_INIT_CLOCKS cycles while still in bit-bang.
        let init_clocks = std::env::var("LP_INIT_CLOCKS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        if init_clocks > 0 {
            let mut pulses = Vec::with_capacity(init_clocks * 2);
            for _ in 0..init_clocks {
                pulses.push(0x06u8); // PROG_B=1, DCLK=1, DIN=0
                pulses.push(0x04u8); // PROG_B=1, DCLK=0, DIN=0
            }
            for chunk in pulses.chunks(10_000) {
                self.transport.bulk_out(chunk, IO_TIMEOUT)?;
                drained_bytes += self.drain_bitbang()?;
            }
            eprintln!("FPGA configure image {idx}: {init_clocks} post-DONE init clocks");
        }
        let final_pins = self.pins()?;
        if final_pins & 0x10 == 0 {
            return Err(ConfigureError::NotConfigured { pins: final_pins });
        }
        // From this point onward nCONFIG must never be asserted merely because
        // FIFO-session initialization fails. Preserve the hardware-proven
        // image identity so the owning backend can retain this Link and retry
        // only non-destructive session/setup work.
        self.configured_idx = Some(idx);
        self.set_bitmode(0, 0)?;
        drained_bytes += self.drain_after_bitbang()?;

        // Flush + FIFO-prime sequence. The FT245 is reset in place
        // (SIO_RESET, NOT a USB re-open), buffers purged, then the FIFO parser
        // is primed with the full tail: 65535 zeros, 30000 zeros, then 0x01
        // (95535 total). A shorter tail under-primes the FPGA command FSM,
        // which then wedges after a handful of commands.
        self.transport
            .set_readahead_depth(command_readahead_depth());
        self.ftdi_reset_purge(&baud)?;
        self.transport.bulk_out(&vec![0u8; 65_535], IO_TIMEOUT)?;
        self.transport.bulk_out(&vec![0u8; 30_000], IO_TIMEOUT)?;
        self.transport.bulk_out(&[0x01], IO_TIMEOUT)?;
        self.ftdi_purge_only()?;

        // Fresh host-side segment/packet cache for the new FIFO session.
        self.bank.invalidate();
        self.pktno.reset_to_zero();
        // The vendor warm-start trace does NOT write the 0x61 enable after the
        // FIFO prime tail; the tail itself readies the parser. Skippable to A/B.
        // The vendor NEVER writes 0x61 to CMD_PORT: verified across 33k+ short
        // writes spanning cold config, idle, and full acquisition captures. The
        // FIFO prime tail (above) alone readies the parser; a 0x61 write instead
        // kicks the sample/prefill engine and starves the command-fetch FSM.
        // The bank stays INVALIDATED on purpose so the IMAGE_ID read below emits
        // a real C3 10 (planning it without writing would target a stale bank).
        self.rx.clear();
        // The vendor verifies IMAGE_ID after Flush + the one-time 0x61 before
        // settings traffic. Besides validating the selected image, this C2
        // response is the FIFO parser's readiness proof for the new session.
        let id = *self
            .read(regs::ctrl::IMAGE_ID, 1)?
            .first()
            .ok_or(ConfigureError::EmptyRegister)?;
        let expected = idx | 0x10;
        if id != expected {
            return Err(ConfigureError::VerifyFailed { expected, got: id });
        }
        let version = 0;
        self.configured_idx = Some(idx);
        Ok(ConfigureOutcome {
            warm: false,
            id,
            version,
            elapsed: self.clock.elapsed().saturating_sub(started),
            drained_bytes,
        })
    }

    /// Write the FIFO resync tail: the vendor's FT_Write lengths of zeros
    /// (0xffff then 30000 in the binary) followed by the single `0x01`.
    /// Vendor reset+purge preamble before the FIFO prime tail (from the
    /// Windows capture): SIO_RESET, GET_MODEM_STATUS, SIO_RESET, purge-TX,
    /// purge-RX x6, latency, baud, purge-TX, purge-RX x6.
    fn ftdi_reset_purge(&mut self, baud: &lp_ftdi::BaudEncoding) -> Result<(), ConfigureError> {
        self.transport.control_out(SIO_RESET, 0, 0)?;
        let _ = self.transport.control_in(0x05, 0, 0, 2)?;
        self.transport.control_out(SIO_RESET, 0, 0)?;
        self.transport
            .control_out(SIO_RESET, SIO_RESET_PURGE_TX, 0)?;
        for _ in 0..6 {
            self.transport
                .control_out(SIO_RESET, SIO_RESET_PURGE_RX, 0)?;
        }
        self.transport
            .control_out(SIO_SET_LATENCY, latency_ms(), 0)?;
        self.transport
            .control_out(SIO_SET_BAUD, baud.value, baud.index)?;
        self.ftdi_purge_only()?;
        Ok(())
    }

    /// purge-TX followed by purge-RX x6, matching the vendor.
    fn ftdi_purge_only(&mut self) -> Result<(), ConfigureError> {
        self.transport
            .control_out(SIO_RESET, SIO_RESET_PURGE_TX, 0)?;
        for _ in 0..6 {
            self.transport
                .control_out(SIO_RESET, SIO_RESET_PURGE_RX, 0)?;
        }
        self.rx.clear();
        Ok(())
    }

    fn write_fifo_tail(&mut self, zero_writes: &[usize]) -> Result<(), ConfigureError> {
        for &zeros in zero_writes {
            if zeros > 0 {
                self.transport.bulk_out(&vec![0; zeros], IO_TIMEOUT)?;
            }
        }
        self.transport.bulk_out(&[0x01], IO_TIMEOUT)?;
        Ok(())
    }

    /// Flush, applied after a failed write/read/bank command: close and
    /// reopen the FTDI handle (reset, timeouts, latency 4, baud 460800), purge
    /// TX then RX, write the zero tail and `0x01`, purge again, reset the
    /// shared packet counter to zero and forget the current bank, then mark
    /// every settings group dirty so the whole setup is re-sent.
    pub fn vendor_flush(&mut self, zero_writes: &[usize]) -> Result<(), ConfigureError> {
        let baud = to_clkbits(460_800).map_err(|error| ConfigureError::Setup(error.to_string()))?;
        self.transport.reopen()?;
        self.transport.control_out(SIO_RESET, 0, 0)?;
        self.transport
            .control_out(SIO_SET_LATENCY, latency_ms(), 0)?;
        self.transport
            .control_out(SIO_SET_BAUD, baud.value, baud.index)?;
        self.transport
            .control_out(SIO_RESET, SIO_RESET_PURGE_TX, 0)?;
        self.purge_rx()?;
        self.write_fifo_tail(zero_writes)?;
        self.transport
            .control_out(SIO_RESET, SIO_RESET_PURGE_TX, 0)?;
        self.purge_rx()?;
        self.bank.invalidate();
        self.pktno.reset_to_zero();
        self.rx.clear();
        Ok(())
    }

    fn drain_bitbang(&mut self) -> Result<usize, ConfigureError> {
        let mut total = 0;
        while total < DRAIN_MAX {
            match self
                .transport
                .bulk_in_raw(DRAIN_MAX.saturating_sub(total), DRAIN_TIMEOUT)
            {
                Ok(raw) => {
                    let read = raw.len();
                    total = total.saturating_add(read);
                    if read < lp_ftdi::nusb_io::IN_TRANSFER_SIZE {
                        break;
                    }
                }
                Err(error)
                    if error.0.contains("transfer was cancelled")
                        || error.0.contains("timed out waiting") =>
                {
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(total)
    }

    fn drain_after_bitbang(&mut self) -> Result<usize, ConfigureError> {
        let mut total = 0;
        loop {
            let drained = self.drain_bitbang()?;
            total += drained;
            if drained < DRAIN_MAX {
                return Ok(total);
            }
        }
    }

    fn purge_rx(&mut self) -> Result<(), ConfigureError> {
        self.transport
            .control_out(SIO_RESET, SIO_RESET_PURGE_RX, 0)?;
        self.rx.clear();
        Ok(())
    }

    fn set_bitmode(&mut self, mask: u8, mode: u8) -> Result<(), ConfigureError> {
        self.transport
            .control_out(SIO_SET_BITMODE, (u16::from(mode) << 8) | u16::from(mask), 0)?;
        Ok(())
    }
    fn wait_pin(&mut self, mask: u8, timeout_error: ConfigureError) -> Result<u8, ConfigureError> {
        let deadline = self.clock.elapsed().saturating_add(DEADLINE);
        let mut last = 0;
        while self.clock.elapsed() < deadline {
            last = self.pins()?;
            if last & mask != 0 {
                return Ok(last);
            }
            self.clock.sleep(POLL);
        }
        match timeout_error {
            ConfigureError::InitTimeout { .. } => {
                Err(ConfigureError::InitTimeout { last_pins: last })
            }
            _ => Err(ConfigureError::DoneTimeout { last_pins: last }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigureError {
    #[error(transparent)]
    Transport(#[from] crate::transport::TransportError),
    #[error(transparent)]
    Device(#[from] DevError),
    #[error("FTDI setup failed: {0}")]
    Setup(String),
    #[error("pin read returned no byte")]
    EmptyPins,
    #[error("register read returned no bytes")]
    EmptyRegister,
    #[error("FPGA is configured but its FIFO protocol is unresponsive (pins 0x{pins:02x})")]
    ProtocolUnresponsive { pins: u8 },
    #[error("FPGA INIT timed out (last pins 0x{last_pins:02x})")]
    InitTimeout { last_pins: u8 },
    #[error(
        "FPGA nCONFIG did not assert low after {attempts} attempts (last pins 0x{last_pins:02x})"
    )]
    ResetAssertFailed { attempts: usize, last_pins: u8 },
    #[error("FPGA DONE timed out (last pins 0x{last_pins:02x})")]
    DoneTimeout { last_pins: u8 },
    #[error("Error Configuring FPGA (pins 0x{pins:02x})")]
    NotConfigured { pins: u8 },
    #[error("Error Configuring FPGA: CONF_DONE did not drop (last pins 0x{last_pins:02x})")]
    ReleaseFailed { last_pins: u8 },
    #[error("FPGA image verification failed: expected 0x{expected:02x}, got 0x{got:02x}")]
    VerifyFailed { expected: u8, got: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        clock::VirtualClock,
        link::LinkConfig,
        transcript::DeviceIdentity,
        transport::{Transport, TransportError},
    };
    use std::collections::VecDeque;
    #[derive(Default)]
    struct Mock {
        pins: VecDeque<u8>,
        default_pin: Option<u8>,
        reads: VecDeque<Vec<u8>>,
        bulk: Vec<Vec<u8>>,
        controls: Vec<(u8, u16, u16)>,
    }
    impl Transport for Mock {
        fn control_out(&mut self, r: u8, v: u16, i: u16) -> Result<(), TransportError> {
            self.controls.push((r, v, i));
            Ok(())
        }
        fn control_in(&mut self, _: u8, _: u16, _: u16, _: u16) -> Result<Vec<u8>, TransportError> {
            Ok(self
                .pins
                .pop_front()
                .or(self.default_pin)
                .map_or_else(Vec::new, |p| vec![p]))
        }
        fn bulk_out(&mut self, d: &[u8], _: Duration) -> Result<(), TransportError> {
            self.bulk.push(d.to_vec());
            Ok(())
        }
        fn bulk_in_raw(&mut self, _: usize, _: Duration) -> Result<Vec<u8>, TransportError> {
            Ok(self.reads.pop_front().unwrap_or_default())
        }
        fn reopen(&mut self) -> Result<(), TransportError> {
            Ok(())
        }
        fn identity(&self) -> DeviceIdentity {
            DeviceIdentity {
                serial: "m".into(),
                bcd_device: 0,
                vid: 0x403,
                pid: 0xdc48,
            }
        }
    }
    fn raw(p: &[u8]) -> Vec<u8> {
        let mut v = vec![0x31, 0x60];
        v.extend_from_slice(p);
        v
    }
    #[test]
    fn warm_reconfigure_releases_then_streams_chunks_and_enables() -> Result<(), ConfigureError> {
        let mut m = Mock::default();
        m.pins.extend([0xf8, 0xe0, 0xe8, 0xf8, 0xf8]);
        // New (no-0x61) reconfigure flow: probe reads the old image, we release
        // it with a single C1 write, stream, prime the FIFO tail (which resets
        // the packet counter), then verify IMAGE_ID. No 0x61 and no block-1
        // re-read, so the mock is much shorter than the old gated flow.
        m.reads.extend([
            vec![],                   // probe drain
            raw(&[0xc3, 0, 0]),       // probe C3 bank ack (pktno 0)
            raw(&[]),                 // drain
            raw(&[0xc2, 0, 1, 0x16]), // probe C2 IMAGE_ID = 0x16 (old image)
            raw(&[]),                 // drain
            raw(&[0xc1, 0, 2]),       // write_release C1 ack (pktno 2)
            // release loop + image stream + FIFO tail: drained bit-bang reads
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            raw(&[]),                 // verify drain (pktno reset by the tail)
            raw(&[0xc3, 0, 0]),       // verify C3 bank ack (pktno 0)
            raw(&[]),                 // drain
            raw(&[0xc2, 0, 1, 0x17]), // verify C2 IMAGE_ID = 0x17
            raw(&[]),
        ]);
        let image = vec![5; 20_001];
        let mut link = Link::new(m, VirtualClock::default(), LinkConfig::default());
        let out = link.configure_fpga(&image, 7, false)?;
        assert!(!out.warm);
        assert_eq!(out.version, 0);
        let (m, _) = link.into_parts();
        assert!(
            m.bulk
                .windows(2)
                .any(|writes| writes[0] == [0xc1, 0, 0, 0, 0] && writes[1] == [0])
        );
        assert_eq!(m.bulk.iter().filter(|b| b.len() == 10_000).count(), 2);
        // The vendor NEVER writes 0x61; the FIFO prime tail alone readies the
        // parser: 65535 zeros, then 30000 zeros, then the single 0x01.
        assert!(!m.bulk.iter().any(|b| b.as_slice() == [0x61]));
        assert!(
            m.bulk
                .iter()
                .any(|b| b.len() == 65_535 && b.iter().all(|&x| x == 0))
        );
        assert!(m.bulk.windows(2).any(|writes| {
            writes[0].len() == 30_000
                && writes[0].iter().all(|&x| x == 0)
                && writes[1].as_slice() == [1]
        }));
        Ok(())
    }

    #[test]
    fn warm_path_skips_image_stream() -> Result<(), ConfigureError> {
        let mut mock = Mock {
            default_pin: Some(0xf8),
            ..Mock::default()
        };
        mock.reads.extend([
            vec![],
            raw(&[0xc3, 0, 0]),
            raw(&[]),
            raw(&[0xc2, 0, 1, 0x17]),
            raw(&[]),
            raw(&[0xc3, 0, 2]),
            raw(&[]),
            raw(&[0xc1, 0, 3]),
            raw(&[]),
            raw(&[0xc2, 0, 4, 0x17]),
            raw(&[]),
        ]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        let outcome = link.configure_fpga(&[5; 16], 7, false)?;
        assert!(outcome.warm);
        assert_eq!(outcome.version, 0);
        let (mock, _) = link.into_parts();
        assert!(!mock.bulk.iter().any(|write| write == &[5; 16]));
        // The vendor NEVER writes 0x61; a warm no-op is only the IMAGE_ID probe
        // (a real C3 bank-select + C2 read), never a post-config "enable".
        assert!(!mock.bulk.iter().any(|w| w.as_slice() == [0x61]));
        assert!(mock.bulk.iter().any(|w| w == &[0xc2, 0x0f, 0, 0, 0]));
        Ok(())
    }

    #[test]
    fn second_same_image_request_is_only_an_image_id_read() -> Result<(), ConfigureError> {
        let mut mock = Mock {
            default_pin: Some(0xf8),
            ..Mock::default()
        };
        // First configure: warm no-op = C3 bank-select + C2 IMAGE_ID probe (no
        // 0x61). Second configure: bank still cached, so only the C2 re-read.
        mock.reads.extend([
            vec![],
            raw(&[0xc3, 0, 0]),
            raw(&[]),
            raw(&[0xc2, 0, 1, 0x17]),
            raw(&[]),
            raw(&[0xc2, 0, 2, 0x17]),
        ]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        link.configure_fpga(&[5; 16], 7, false)?;
        let writes_before = link.transport.bulk.len();
        let controls_before = link.transport.controls.len();
        let outcome = link.configure_fpga(&[5; 16], 7, false)?;
        assert!(outcome.warm);
        let new_writes = &link.transport.bulk[writes_before..];
        assert_eq!(new_writes, &[vec![0xc2, 0x0f, 0, 0, 0]]);
        assert_eq!(link.transport.controls.len(), controls_before);
        Ok(())
    }

    #[test]
    fn init_timeout_uses_virtual_one_second_deadline() {
        let mut mock = Mock {
            default_pin: Some(0xe0),
            ..Mock::default()
        };
        mock.pins.push_back(0xe0);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        assert!(matches!(
            link.configure_fpga(&[5; 16], 7, false),
            Err(ConfigureError::InitTimeout { last_pins: 0xe0 })
        ));
        let (_, clock) = link.into_parts();
        assert!(clock.elapsed() >= Duration::from_secs(1));
    }

    #[test]
    fn cold_path_refuses_to_release_an_unlatched_nconfig() {
        let mock = Mock {
            default_pin: Some(0x04),
            ..Mock::default()
        };
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        assert!(matches!(
            link.configure_fpga(&[5; 16], 7, false),
            Err(ConfigureError::ResetAssertFailed {
                attempts: NCONFIG_ATTEMPTS,
                last_pins: 0x04
            })
        ));
        let (mock, _) = link.into_parts();
        assert_eq!(
            mock.bulk
                .iter()
                .filter(|write| write.as_slice() == [0])
                .count(),
            3
        );
        assert!(!mock.bulk.iter().any(|write| write.as_slice() == [0x04]));
    }

    #[test]
    fn configured_but_silent_fifo_is_not_reconfigured() {
        let mock = Mock {
            default_pin: Some(0xf8),
            ..Mock::default()
        };
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        assert!(matches!(
            link.configure_fpga(&[5; 16], 7, false),
            Err(ConfigureError::ProtocolUnresponsive { pins: 0xf8 })
        ));
        let (mock, _) = link.into_parts();
        assert!(!mock.bulk.iter().any(|write| write.as_slice() == [0]));
        assert!(!mock.bulk.iter().any(|write| write.as_slice() == [0x04]));
    }
}
