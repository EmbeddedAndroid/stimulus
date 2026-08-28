use crate::{
    transcript::DeviceIdentity,
    transport::{Transport, TransportError},
};
use lp_ftdi::{
    consts::{PID_LA1034, VID},
    device::BulkIo,
    nusb_io::NusbIo,
};
use nusb::MaybeFuture;
use std::time::Duration;

pub struct RealTransport {
    io: Option<NusbIo>,
    identity: DeviceIdentity,
}

impl RealTransport {
    pub fn open() -> Result<Self, TransportError> {
        let identity = descriptor()?;
        let io = NusbIo::open().map_err(convert)?;
        Ok(Self {
            io: Some(io),
            identity,
        })
    }

    /// Escalated recovery for a stale usbfs/FT245 endpoint state.
    ///
    /// This is deliberately separate from `Transport::reopen`: the latter is
    /// part of the vendor's ordinary post-configure Flush sequence and must
    /// not reset the physical USB device.
    pub fn reset_attached() -> Result<(), TransportError> {
        NusbIo::open()
            .map_err(convert)?
            .reset_device()
            .map_err(convert)
    }

    /// Confirm that the physical analyzer is still enumerated without
    /// injecting a command into its FPGA FIFO session.
    ///
    /// A periodic IMAGE_ID transaction is not a transport health check: it
    /// advances the shared packet counter and can destroy an otherwise-live
    /// session if its response is interrupted. USB enumeration is sufficient
    /// to detect the unplug/replug boundary; normal operations report command
    /// path failures themselves.
    pub fn attached_identity() -> Result<DeviceIdentity, TransportError> {
        descriptor()
    }
    fn io(&mut self) -> Result<&mut NusbIo, TransportError> {
        self.io
            .as_mut()
            .ok_or_else(|| TransportError("USB interface is not open".into()))
    }
}

impl Drop for RealTransport {
    fn drop(&mut self) {
        // nusb endpoint cleanup on Drop is asynchronous.  A failed startup
        // must release interface 0 before the daemon's next reopen attempt,
        // otherwise that same process races its stale usbfs claim and reports
        // EBUSY indefinitely.
        if let Some(io) = self.io.take() {
            let _ = io.close();
        }
    }
}

impl Transport for RealTransport {
    fn control_out(&mut self, req: u8, value: u16, index: u16) -> Result<(), TransportError> {
        self.io()?
            .vendor_control_out(req, value, index, Duration::from_secs(5))
            .map_err(convert)
    }
    fn control_in(
        &mut self,
        req: u8,
        value: u16,
        index: u16,
        len: u16,
    ) -> Result<Vec<u8>, TransportError> {
        self.io()?
            .vendor_control_in(req, value, index, len, Duration::from_secs(5))
            .map_err(convert)
    }
    fn bulk_out(&mut self, mut data: &[u8], timeout: Duration) -> Result<(), TransportError> {
        while !data.is_empty() {
            let n = self
                .io()?
                .bulk_out(&data[..data.len().min(16_384)], timeout)
                .map_err(convert)?;
            if n == 0 {
                return Err(TransportError("bulk OUT made no progress".into()));
            }
            data = &data[n..];
        }
        Ok(())
    }
    fn bulk_out_unprimed(
        &mut self,
        mut data: &[u8],
        timeout: Duration,
    ) -> Result<(), TransportError> {
        while !data.is_empty() {
            let n = self
                .io()?
                .bulk_out_unprimed(&data[..data.len().min(16_384)], timeout)
                .map_err(convert)?;
            if n == 0 {
                return Err(TransportError("bulk OUT made no progress".into()));
            }
            data = &data[n..];
        }
        Ok(())
    }
    fn bulk_out_pair(
        &mut self,
        first: &[u8],
        second: &[u8],
        timeout: Duration,
    ) -> Result<(), TransportError> {
        self.io()?
            .bulk_out_pair(first, second, timeout)
            .map_err(convert)
    }
    fn bulk_in_raw(&mut self, max: usize, timeout: Duration) -> Result<Vec<u8>, TransportError> {
        // FTDI status bytes belong to physical USB packet boundaries. Never
        // ask NusbIo to split a completed transfer merely because the protocol
        // currently needs only a few bytes: returning the remainder through a
        // byte-only spill buffer would make its first two payload bytes look
        // like a new status prefix on the next call.
        // Diagnostic lever (LP_IN_SIZE): the size of the bulk-IN URB used for
        // ordinary FIFO command responses. A large full-speed IN request behind
        // an xHCI hub transaction-translator generates a long NAK burst while
        // the FPGA is still composing its answer; shrinking it cuts that burst.
        // The bit-bang drain path (max >= IN_TRANSFER_SIZE) is never shrunk.
        let read_floor = std::env::var("LP_IN_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v >= 2)
            .unwrap_or(lp_ftdi::nusb_io::IN_TRANSFER_SIZE);
        let buf = if max >= lp_ftdi::nusb_io::IN_TRANSFER_SIZE {
            max
        } else {
            max.max(read_floor)
        };
        let mut data = vec![0; buf.max(2)];
        let n = self.io()?.bulk_in(&mut data, timeout).map_err(convert)?;
        data.truncate(n);
        if max >= lp_ftdi::nusb_io::IN_TRANSFER_SIZE || data.len() < 2 {
            return Ok(data);
        }

        // The command layer expects an exact logical byte count. nusb exposes
        // the underlying USB completion instead, which may contain many 64-byte
        // FTDI packets. Reconstruct the exact-length logical read while
        // respecting every packet's two-byte status prefix; bytes beyond this
        // command's declared response cannot
        // leak into the next transaction.
        let wanted = max.saturating_sub(2);
        let status = [data[0], data[1]];
        let mut payload = Vec::with_capacity(wanted);
        for packet in data.chunks(lp_ftdi::consts::PKT) {
            if packet.len() > 2 {
                let remaining = wanted.saturating_sub(payload.len());
                payload.extend_from_slice(&packet[2..packet.len().min(2 + remaining)]);
            }
            if payload.len() == wanted {
                break;
            }
        }
        let mut logical = Vec::with_capacity(payload.len() + 2);
        logical.extend_from_slice(&status);
        logical.extend_from_slice(&payload);
        Ok(logical)
    }
    fn wait_tx_empty(&mut self, timeout: Duration) -> Result<(), TransportError> {
        self.io()?.wait_tx_empty(timeout).map_err(convert)
    }
    fn set_readahead_depth(&mut self, depth: usize) {
        if let Some(io) = self.io.as_ref() {
            io.set_readahead_depth(depth);
        }
    }
    fn debug_input_queued(&self) -> usize {
        self.io.as_ref().map_or(0, NusbIo::input_queued)
    }
    fn drain_input(&mut self) -> usize {
        self.io.as_ref().map_or(0, NusbIo::drain_input)
    }
    fn reopen(&mut self) -> Result<(), TransportError> {
        if let Some(io) = self.io.take() {
            io.close().map_err(convert)?;
        }
        self.io = Some(NusbIo::open().map_err(convert)?);
        self.identity = descriptor()?;
        Ok(())
    }
    fn identity(&self) -> DeviceIdentity {
        self.identity.clone()
    }
}

fn descriptor() -> Result<DeviceIdentity, TransportError> {
    let info = nusb::list_devices()
        .wait()
        .map_err(|e| TransportError(e.to_string()))?
        .find(|d| d.vendor_id() == VID && d.product_id() == PID_LA1034)
        .ok_or_else(|| TransportError("LogicPort 0403:dc48 is not attached".into()))?;
    Ok(DeviceIdentity {
        serial: info.serial_number().unwrap_or("unknown").into(),
        bcd_device: info.device_version(),
        vid: info.vendor_id(),
        pid: info.product_id(),
    })
}
fn convert(error: lp_ftdi::device::FtdiError) -> TransportError {
    TransportError(error.to_string())
}
