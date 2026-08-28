use crate::transcript::DeviceIdentity;
use std::time::Duration;

pub trait Transport: Send {
    fn control_out(&mut self, req: u8, value: u16, index: u16) -> Result<(), TransportError>;
    fn control_in(
        &mut self,
        req: u8,
        value: u16,
        index: u16,
        len: u16,
    ) -> Result<Vec<u8>, TransportError>;
    fn bulk_out(&mut self, data: &[u8], timeout: Duration) -> Result<(), TransportError>;
    fn bulk_out_unprimed(&mut self, data: &[u8], timeout: Duration) -> Result<(), TransportError> {
        self.bulk_out(data, timeout)
    }
    fn bulk_out_pair(
        &mut self,
        first: &[u8],
        second: &[u8],
        timeout: Duration,
    ) -> Result<(), TransportError> {
        self.bulk_out(first, timeout)?;
        self.bulk_out(second, timeout)
    }
    fn bulk_in_raw(&mut self, max: usize, timeout: Duration) -> Result<Vec<u8>, TransportError>;
    fn wait_tx_empty(&mut self, _timeout: Duration) -> Result<(), TransportError> {
        Ok(())
    }
    fn set_readahead_depth(&mut self, _depth: usize) {}
    /// Number of completed IN transfers queued in the read-ahead pool but not
    /// yet consumed. Diagnostic only (0 for transports without a pool); used to
    /// log the drain backlog at a readback wedge.
    fn debug_input_queued(&self) -> usize {
        0
    }
    /// Discard completed IN transfers queued in the read-ahead pool but not yet
    /// consumed, returning the count discarded. Called at capture boundaries so
    /// a new readback never starts on a stale backlog left by the previous
    /// capture's over-read. No-op (0) for transports without a pool.
    fn drain_input(&mut self) -> usize {
        0
    }
    fn reopen(&mut self) -> Result<(), TransportError>;
    fn identity(&self) -> DeviceIdentity;
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("transport error: {0}")]
pub struct TransportError(pub String);
