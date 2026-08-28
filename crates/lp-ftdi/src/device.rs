use std::time::{Duration, Instant};

use crate::rx::RxBuffer;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FtdiError {
    #[error("bulk I/O failed: {0}")]
    Io(String),
    #[error("timed out waiting for {wanted} bytes; received {got}")]
    Timeout { wanted: usize, got: usize },
    #[error("bulk write made no progress with {remaining} bytes remaining")]
    WriteZero { remaining: usize },
}

pub trait BulkIo {
    fn bulk_out(&mut self, data: &[u8], timeout: Duration) -> Result<usize, FtdiError>;
    fn bulk_in(&mut self, data: &mut [u8], timeout: Duration) -> Result<usize, FtdiError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    pub read: Duration,
    pub write: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            read: Duration::from_secs(5),
            write: Duration::from_secs(5),
        }
    }
}

pub struct FtdiDevice<B: BulkIo> {
    io: B,
    rx: RxBuffer,
    timeouts: Timeouts,
}

impl<B: BulkIo> FtdiDevice<B> {
    pub fn new(io: B, timeouts: Timeouts) -> Self {
        Self {
            io,
            rx: RxBuffer::default(),
            timeouts,
        }
    }

    pub fn write_all(&mut self, mut data: &[u8]) -> Result<(), FtdiError> {
        while !data.is_empty() {
            let chunk_len = data.len().min(16_384);
            let written = self.io.bulk_out(&data[..chunk_len], self.timeouts.write)?;
            if written == 0 {
                return Err(FtdiError::WriteZero {
                    remaining: data.len(),
                });
            }
            data = &data[written..];
        }
        Ok(())
    }

    pub fn fill(&mut self, max_wait: Duration) -> Result<usize, FtdiError> {
        let mut raw = [0u8; 512];
        let read = self.io.bulk_in(&mut raw, max_wait)?;
        let before = self.rx.available();
        self.rx.push_raw(&raw[..read]);
        Ok(self.rx.available() - before)
    }

    pub fn wait_for(&mut self, wanted: usize, timeout: Duration) -> Result<(), FtdiError> {
        let deadline = Instant::now() + timeout;
        while self.rx.available() < wanted {
            let now = Instant::now();
            if now >= deadline {
                return Err(FtdiError::Timeout {
                    wanted,
                    got: self.rx.available(),
                });
            }
            let remaining = deadline.saturating_duration_since(now);
            let _ = self.fill(remaining.min(self.timeouts.read))?;
            std::thread::yield_now();
        }
        Ok(())
    }

    pub fn read_exact(&mut self, wanted: usize, timeout: Duration) -> Result<Vec<u8>, FtdiError> {
        self.wait_for(wanted, timeout)?;
        self.rx.take(wanted).ok_or(FtdiError::Timeout {
            wanted,
            got: self.rx.available(),
        })
    }

    pub fn clear_rx(&mut self) {
        self.rx.clear();
    }
    pub(crate) fn io_mut(&mut self) -> &mut B {
        &mut self.io
    }
    pub fn into_inner(self) -> B {
        self.io
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct FakeBulk {
        reads: VecDeque<Vec<u8>>,
        writes: Vec<u8>,
    }

    impl BulkIo for FakeBulk {
        fn bulk_out(&mut self, data: &[u8], _timeout: Duration) -> Result<usize, FtdiError> {
            self.writes.extend_from_slice(data);
            Ok(data.len())
        }
        fn bulk_in(&mut self, data: &mut [u8], _timeout: Duration) -> Result<usize, FtdiError> {
            let Some(read) = self.reads.pop_front() else {
                return Ok(0);
            };
            let n = read.len().min(data.len());
            data[..n].copy_from_slice(&read[..n]);
            Ok(n)
        }
    }

    #[test]
    fn wait_for_times_out() {
        let mut dev = FtdiDevice::new(FakeBulk::default(), Timeouts::default());
        assert_eq!(
            dev.wait_for(10, Duration::ZERO),
            Err(FtdiError::Timeout { wanted: 10, got: 0 })
        );
    }

    #[test]
    fn read_exact_strips_status_and_preserves_remainder() {
        let mut fake = FakeBulk::default();
        fake.reads.push_back(vec![0x31, 0x60, 1, 2, 3, 4]);
        let mut dev = FtdiDevice::new(fake, Timeouts::default());
        assert_eq!(
            dev.read_exact(3, Duration::from_millis(10)),
            Ok(vec![1, 2, 3])
        );
        assert_eq!(dev.read_exact(1, Duration::from_millis(10)), Ok(vec![4]));
    }

    #[test]
    fn write_all_chunks_large_payloads() {
        let mut dev = FtdiDevice::new(FakeBulk::default(), Timeouts::default());
        let payload = vec![0x55; 40_000];
        dev.write_all(&payload).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(dev.into_inner().writes, payload);
    }
}
