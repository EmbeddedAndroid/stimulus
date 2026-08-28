use crate::transcript::{Event, EventKind, Transcript};
use crate::{
    transcript::{Payload, encode_hex},
    transport::{Transport, TransportError},
};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

pub struct Recording {
    transcript: Transcript,
    path: PathBuf,
    checkpoint_every: usize,
}

impl Recording {
    pub fn new(path: impl Into<PathBuf>, transcript: Transcript) -> Self {
        Self {
            transcript,
            path: path.into(),
            checkpoint_every: 100,
        }
    }
    pub fn push(&mut self, t_us: u64, kind: EventKind) -> io::Result<()> {
        let i = self.transcript.events.len() as u64;
        self.transcript.events.push(Event { i, t_us, kind });
        if self
            .transcript
            .events
            .len()
            .is_multiple_of(self.checkpoint_every)
        {
            self.flush()?;
        }
        Ok(())
    }
    pub fn flush(&self) -> io::Result<()> {
        let temporary = self.path.with_extension("json.tmp");
        let mut file = File::create(&temporary)?;
        serde_json::to_writer_pretty(&mut file, &self.transcript).map_err(io::Error::other)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &self.path)?;
        if let Some(parent) = self.path.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    }
    pub fn finish(self) -> io::Result<Transcript> {
        self.flush()?;
        Ok(self.transcript)
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    OpenOptions::new().read(true).open(path)?.sync_all()
}

pub struct RecordingTransport<T> {
    inner: T,
    transcript: Transcript,
    epoch: Instant,
}
impl<T> RecordingTransport<T> {
    pub fn new(inner: T, transcript: Transcript) -> Self {
        Self {
            inner,
            transcript,
            epoch: Instant::now(),
        }
    }
    fn event(&mut self, kind: EventKind) {
        let i = self.transcript.events.len() as u64;
        let micros = self.epoch.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        self.transcript.events.push(Event {
            i,
            t_us: micros,
            kind,
        });
    }
    pub fn finish(self, path: impl Into<PathBuf>) -> io::Result<Transcript> {
        Recording::new(path, self.transcript).finish()
    }
    fn save(&self, path: impl Into<PathBuf>) -> io::Result<Transcript> {
        Recording::new(path, self.transcript.clone()).finish()
    }
    fn clear_events(&mut self) {
        self.transcript.events.clear();
        self.epoch = Instant::now();
    }
}

impl<T: Transport, C: crate::clock::Clock> crate::link::Link<RecordingTransport<T>, C> {
    /// Start a new evidence window without reopening the live device or losing
    /// protocol cache state.
    pub fn clear_recording_events(&mut self) {
        self.transport.clear_events();
    }

    /// Persist the current evidence window without closing the live transport.
    pub fn save_recording(&self, path: impl Into<PathBuf>) -> io::Result<Transcript> {
        self.transport.save(path)
    }
}
impl<T: Transport> Transport for RecordingTransport<T> {
    fn control_out(&mut self, req: u8, value: u16, index: u16) -> Result<(), TransportError> {
        self.inner.control_out(req, value, index)?;
        self.event(EventKind::ControlOut { req, value, index });
        Ok(())
    }
    fn control_in(
        &mut self,
        req: u8,
        value: u16,
        index: u16,
        len: u16,
    ) -> Result<Vec<u8>, TransportError> {
        let resp = self.inner.control_in(req, value, index, len)?;
        self.event(EventKind::ControlIn {
            req,
            value,
            index,
            len: usize::from(len),
            resp: encode_hex(&resp),
        });
        Ok(resp)
    }
    fn bulk_out(&mut self, data: &[u8], timeout: Duration) -> Result<(), TransportError> {
        self.inner.bulk_out(data, timeout)?;
        let payload = if data.len() >= 4096 {
            Payload::Digest {
                data_sha256: format!("{:x}", Sha256::digest(data)),
                len: data.len(),
            }
        } else {
            Payload::Inline {
                data: encode_hex(data),
            }
        };
        self.event(EventKind::BulkOut { data: payload });
        Ok(())
    }
    fn bulk_in_raw(&mut self, max: usize, timeout: Duration) -> Result<Vec<u8>, TransportError> {
        let raw = self.inner.bulk_in_raw(max, timeout)?;
        self.event(EventKind::BulkIn {
            max,
            timeout_ms: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
            raw: encode_hex(&raw),
        });
        Ok(raw)
    }
    fn reopen(&mut self) -> Result<(), TransportError> {
        self.inner.reopen()?;
        self.event(EventKind::Reopen);
        Ok(())
    }
    fn identity(&self) -> crate::transcript::DeviceIdentity {
        self.inner.identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::{DeviceIdentity, SCHEMA};

    #[test]
    fn roundtrip_schema_on_disk() -> Result<(), Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!("lp-recording-{}.json", std::process::id()));
        let transcript = Transcript {
            schema: SCHEMA.into(),
            recorded_at: "now".into(),
            tool: "test".into(),
            device: DeviceIdentity {
                serial: "x".into(),
                bcd_device: 0,
                vid: 0x403,
                pid: 0xdc48,
            },
            scenario: "unit".into(),
            notes: vec![],
            events: vec![],
        };
        let mut recording = Recording::new(&path, transcript);
        recording.push(10, EventKind::Open)?;
        recording.push(20, EventKind::Close)?;
        let expected = recording.finish()?;
        let actual: Transcript = serde_json::from_reader(File::open(&path)?)?;
        std::fs::remove_file(path)?;
        assert_eq!(actual, expected);
        Ok(())
    }
}
