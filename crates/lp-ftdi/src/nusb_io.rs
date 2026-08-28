use std::collections::VecDeque;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nusb::{
    Device, Endpoint, Interface, MaybeFuture,
    transfer::{Bulk, ControlIn, ControlOut, In, Out},
};

use crate::{
    consts::{EP_IN, EP_OUT, PID_LA1034, VID},
    device::{BulkIo, FtdiError},
};

pub const IN_TRANSFER_SIZE: usize = 16_384;

/// Default IN transfer (URB) size for the read-ahead pump. The FT245 max packet
/// is 64 bytes; reading packet-sized URBs keeps the device->host FIFO drained
/// promptly and the FPGA command FSM alive. 16 KiB URBs stay open across many
/// packets and batch the drain, which wedges the FPGA after a few commands.
pub const PUMP_IN_XFER: usize = 64;

/// Depth of the continuous IN read-ahead pool. The FT245 device->host FIFO must
/// be drained continuously: keeping a few IN transfers outstanding at all times
/// stops it from backing up. A demand-driven read leaves nothing outstanding
/// between reads, the TX FIFO fills, the FPGA's response write stalls on TXE#,
/// and its command parser wedges after a few transactions until a power cycle.
/// Override with LP_READAHEAD_DEPTH.
const READAHEAD_DEPTH: usize = 8;

/// How long the pump waits on a completion before looping to re-check the stop
/// flag and re-arm the pool. Short enough that the pool is effectively always
/// full; the loop also returns immediately whenever a transfer completes.
const PUMP_TICK: Duration = Duration::from_millis(20);

fn readahead_depth() -> usize {
    std::env::var("LP_READAHEAD_DEPTH")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|d| *d >= 1)
        .unwrap_or(READAHEAD_DEPTH)
}

/// IN transfer (URB) size for the read-ahead pump. Small packet-oriented reads
/// drain the FT245 device->host FIFO promptly; large URBs stay open across many
/// 64-byte packets until a short packet or the latency timer closes them, which
/// delivers command responses in coarse batches and can wedge the command FSM.
/// LP_IN_XFER overrides it so packet-sized reads can be A/B'd on hardware.
fn in_xfer_size() -> usize {
    std::env::var("LP_IN_XFER")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n >= 64)
        .unwrap_or(PUMP_IN_XFER)
}

struct PumpState {
    items: VecDeque<Vec<u8>>,
    err: Option<String>,
    done: bool,
}

struct Shared {
    state: Mutex<PumpState>,
    cv: Condvar,
    stop: AtomicBool,
    /// Desired number of IN transfers to keep in flight. Adjustable at runtime:
    /// the bit-bang configure phase wants a modest depth; the FIFO command phase
    /// wants a high depth so enough transfers stay in flight to keep the FT245
    /// device->host FIFO drained and the FPGA FSM alive despite userspace
    /// re-arm latency.
    depth: AtomicUsize,
}

/// Background IN reader: owns the IN endpoint and continuously keeps a pool of
/// transfers in flight, pushing each completed transfer verbatim onto a shared
/// queue. Keeping the pool re-armed even while the command loop is busy
/// elsewhere is the whole point: it is what stops the FT245/FPGA interface from
/// wedging.
struct InPump {
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
}

impl InPump {
    fn start(ep_in: Endpoint<Bulk, In>) -> Result<Self, FtdiError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(PumpState {
                items: VecDeque::new(),
                err: None,
                done: false,
            }),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
            depth: AtomicUsize::new(readahead_depth()),
        });
        let thread_shared = shared.clone();
        let handle = std::thread::Builder::new()
            .name("lp-ftdi-in-pump".to_owned())
            .spawn(move || pump_loop(ep_in, thread_shared))
            .map_err(|error| FtdiError::Io(format!("spawn lp-ftdi in-pump thread: {error}")))?;
        Ok(InPump {
            shared,
            handle: Some(handle),
        })
    }

    fn set_depth(&self, depth: usize) {
        self.shared.depth.store(depth.max(1), Ordering::SeqCst);
    }

    fn queued_len(&self) -> usize {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .items
            .len()
    }

    /// Discard every completed-but-unconsumed IN transfer, returning the count
    /// discarded. Called at capture boundaries: the continuously-armed pool can
    /// over-read past what a command needed, leaving stale transfers queued;
    /// the next readback would pop those first and desync FT245/FPGA command
    /// framing (the root cause of the sustained-load wedge). Clearing the queue
    /// keeps each capture's readback starting from clean, current bytes.
    fn drain(&self) -> usize {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let discarded = state.items.len();
        state.items.clear();
        discarded
    }

    /// Pop the next completed IN transfer (raw bytes, FTDI status prefixes
    /// intact and USB-packet aligned), blocking up to `timeout`.
    fn pop(&self, timeout: Duration) -> Result<Vec<u8>, FtdiError> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        loop {
            if let Some(item) = state.items.pop_front() {
                return Ok(item);
            }
            if let Some(err) = &state.err {
                return Err(FtdiError::Io(err.clone()));
            }
            let now = Instant::now();
            if state.done || now >= deadline {
                return Err(FtdiError::Timeout { wanted: 0, got: 0 });
            }
            let (guard, _timeout) = self
                .shared
                .cv
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|poison| poison.into_inner());
            state = guard;
        }
    }

    fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for InPump {
    fn drop(&mut self) {
        self.stop();
    }
}

fn pump_loop(ep_in: Endpoint<Bulk, In>, shared: Arc<Shared>) {
    let mut applied = shared.depth.load(Ordering::SeqCst);
    let mut reader = ep_in.reader(in_xfer_size()).with_num_transfers(applied);
    reader.set_read_timeout(PUMP_TICK);
    // Env-gated instrumentation for the readback wedge: watch whether the
    // completed-transfer queue backs up (host not draining) vs stays flat.
    let debug = std::env::var("LP_PUMP_DEBUG").is_ok();
    let mut completed: u64 = 0u64;
    let mut max_queued: usize = 0;
    while !shared.stop.load(Ordering::SeqCst) {
        let want = shared.depth.load(Ordering::SeqCst);
        if want != applied {
            reader.set_num_transfers(want);
            applied = want;
        }
        match reader.fill_buf() {
            Ok(buf) => {
                if buf.is_empty() {
                    continue;
                }
                let item = buf.to_vec();
                let len = item.len();
                reader.consume(len);
                // Every FT245 IN packet is prefixed with 2 modem-status bytes
                // that the protocol layer strips and ignores. An idle keepalive
                // is JUST that 2-byte prefix with no payload. Queuing those lets
                // the (unbounded) completed-transfer queue back up by hundreds
                // of stale entries during the idle gap between captures; the
                // next readback then consumes that backlog and desyncs, wedging
                // the FT245/FPGA command channel. Drop payload-less packets at
                // the source so only real data is ever queued.
                if len <= 2 {
                    continue;
                }
                let mut state = shared
                    .state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                state.items.push_back(item);
                let queued = state.items.len();
                shared.cv.notify_all();
                drop(state);
                if debug {
                    completed = completed.wrapping_add(1);
                    if queued > max_queued {
                        max_queued = queued;
                    }
                    if completed.is_multiple_of(200) {
                        eprintln!(
                            "lp-pump: completed={completed} queued={queued} max_queued={max_queued} depth={applied} xfer={len}"
                        );
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                // No completion this tick. fill_buf still re-armed the pool
                // (start_read runs before the wait), so the pipe stays full.
            }
            Err(error) => {
                let mut state = shared
                    .state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                state.err = Some(error.to_string());
                shared.cv.notify_all();
                break;
            }
        }
    }
    // Cancel the pool and drain the cancelled completions so the endpoint (and
    // therefore the interface) can be released/re-claimed without EBUSY.
    reader.cancel_all();
    while let Ok(buf) = reader.fill_buf() {
        if buf.is_empty() {
            break;
        }
        let len = buf.len();
        reader.consume(len);
    }
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    state.done = true;
    shared.cv.notify_all();
}

pub struct NusbIo {
    device: Device,
    interface: Interface,
    ep_out: Endpoint<Bulk, Out>,
    in_pump: Option<InPump>,
}

impl NusbIo {
    pub fn open() -> Result<Self, FtdiError> {
        let info = nusb::list_devices()
            .wait()
            .map_err(io_error)?
            .find(|device| device.vendor_id() == VID && device.product_id() == PID_LA1034)
            .ok_or_else(|| FtdiError::Io("LogicPort 0403:dc48 is not attached".to_owned()))?;
        let device = info.open().wait().map_err(io_error)?;
        let interface = device
            .detach_and_claim_interface(0)
            .wait()
            .map_err(io_error)?;
        let ep_out = interface.endpoint::<Bulk, Out>(EP_OUT).map_err(io_error)?;
        let ep_in = interface.endpoint::<Bulk, In>(EP_IN).map_err(io_error)?;
        let in_pump = InPump::start(ep_in)?;
        Ok(Self {
            device,
            interface,
            ep_out,
            in_pump: Some(in_pump),
        })
    }

    pub fn close(mut self) -> Result<(), FtdiError> {
        // Stop the reader thread first: it cancels and drains the IN pool and
        // drops the endpoint, so the interface can be released cleanly.
        if let Some(mut pump) = self.in_pump.take() {
            pump.stop();
        }
        let Self {
            device: _,
            interface,
            ep_out,
            in_pump: _,
        } = self;
        drop(ep_out);
        interface.release().wait().map_err(io_error)
    }

    pub fn reset_device(mut self) -> Result<(), FtdiError> {
        if let Some(mut pump) = self.in_pump.take() {
            pump.stop();
        }
        self.ep_out.cancel_all();
        let _ = self.ep_out.clear_halt().wait();
        self.device.reset().wait().map_err(io_error)
    }

    pub fn control_out(&self, request: ControlOut<'_>, timeout: Duration) -> Result<(), FtdiError> {
        self.interface
            .control_out(request, timeout)
            .wait()
            .map_err(io_error)
    }

    pub fn control_in(&self, request: ControlIn, timeout: Duration) -> Result<Vec<u8>, FtdiError> {
        self.interface
            .control_in(request, timeout)
            .wait()
            .map_err(io_error)
    }

    pub fn vendor_control_out(
        &self,
        request: u8,
        value: u16,
        index: u16,
        timeout: Duration,
    ) -> Result<(), FtdiError> {
        self.control_out(
            ControlOut {
                control_type: nusb::transfer::ControlType::Vendor,
                recipient: nusb::transfer::Recipient::Device,
                request,
                value,
                index,
                data: &[],
            },
            timeout,
        )
    }

    pub fn vendor_control_in(
        &self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
        timeout: Duration,
    ) -> Result<Vec<u8>, FtdiError> {
        self.control_in(
            ControlIn {
                control_type: nusb::transfer::ControlType::Vendor,
                recipient: nusb::transfer::Recipient::Device,
                request,
                value,
                index,
                length,
            },
            timeout,
        )
    }

    pub fn bulk_out_unprimed(
        &mut self,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, FtdiError> {
        self.bulk_out(data, timeout)
    }

    /// Preserve the vendor's two ordered FT_Write calls for C1.
    pub fn bulk_out_pair(
        &mut self,
        first: &[u8],
        second: &[u8],
        timeout: Duration,
    ) -> Result<(), FtdiError> {
        let first_len = self.bulk_out(first, timeout)?;
        let second_len = self.bulk_out(second, timeout)?;
        if first_len != first.len() || second_len != second.len() {
            return Err(FtdiError::Io("short C1 header/payload write".to_owned()));
        }
        Ok(())
    }

    pub fn wait_tx_empty(&mut self, _timeout: Duration) -> Result<(), FtdiError> {
        Ok(())
    }

    /// Adjust the IN read-ahead depth at runtime.
    pub fn set_readahead_depth(&self, depth: usize) {
        if let Some(pump) = self.in_pump.as_ref() {
            pump.set_depth(depth);
        }
    }

    /// Completed IN transfers queued but not yet consumed (diagnostic).
    pub fn input_queued(&self) -> usize {
        self.in_pump.as_ref().map_or(0, InPump::queued_len)
    }

    /// Discard queued-but-unconsumed IN transfers (capture-boundary flush).
    /// Returns the number discarded.
    pub fn drain_input(&self) -> usize {
        self.in_pump.as_ref().map_or(0, InPump::drain)
    }
}

impl BulkIo for NusbIo {
    fn bulk_out(&mut self, data: &[u8], timeout: Duration) -> Result<usize, FtdiError> {
        let completion = self.ep_out.transfer_blocking(data.to_vec().into(), timeout);
        completion.status.map_err(io_error)?;
        Ok(completion.actual_len)
    }

    fn bulk_in(&mut self, data: &mut [u8], timeout: Duration) -> Result<usize, FtdiError> {
        let pump = self
            .in_pump
            .as_ref()
            .ok_or_else(|| FtdiError::Io("IN reader stopped".to_owned()))?;
        // One completed transfer from the continuously-armed read-ahead pool.
        // Callers on the hardware path pass buffers >= IN_TRANSFER_SIZE, so a
        // whole transfer (<= IN_TRANSFER_SIZE) always fits and USB-packet
        // alignment is preserved for the upstream FTDI status stripping.
        let item = pump.pop(timeout)?;
        let actual = item.len().min(data.len());
        data[..actual].copy_from_slice(&item[..actual]);
        Ok(actual)
    }
}

fn io_error(error: impl std::fmt::Display) -> FtdiError {
    FtdiError::Io(error.to_string())
}
