use crate::transcript::{Event, EventKind, Payload, Transcript, decode_hex, encode_hex};

#[derive(Debug)]
pub struct Replay {
    transcript: Transcript,
    index: usize,
    virtual_us: u64,
}

impl Replay {
    pub fn new(transcript: Transcript) -> Result<Self, ReplayError> {
        transcript
            .validate()
            .map_err(|error| ReplayError::Schema(error.to_string()))?;
        Ok(Self {
            transcript,
            index: 0,
            virtual_us: 0,
        })
    }

    fn next(&mut self, actual: EventKind) -> Result<Event, ReplayError> {
        let index = self.index;
        let expected =
            self.transcript
                .events
                .get(index)
                .cloned()
                .ok_or_else(|| ReplayError::Exhausted {
                    index,
                    actual: actual.clone(),
                })?;
        if !matches_kind(&expected.kind, &actual) {
            return Err(ReplayError::Divergence {
                index,
                expected: expected.kind,
                actual,
            });
        }
        self.index += 1;
        self.virtual_us = expected.t_us;
        Ok(expected)
    }

    pub fn open(&mut self) -> Result<(), ReplayError> {
        self.next(EventKind::Open).map(|_| ())
    }
    pub fn reopen(&mut self) -> Result<(), ReplayError> {
        self.next(EventKind::Reopen).map(|_| ())
    }
    pub fn close(&mut self) -> Result<(), ReplayError> {
        self.next(EventKind::Close).map(|_| ())
    }
    pub fn control_out(&mut self, req: u8, value: u16, index: u16) -> Result<(), ReplayError> {
        self.next(EventKind::ControlOut { req, value, index })
            .map(|_| ())
    }
    pub fn control_in(
        &mut self,
        req: u8,
        value: u16,
        index: u16,
        len: usize,
    ) -> Result<Vec<u8>, ReplayError> {
        match self
            .next(EventKind::ControlIn {
                req,
                value,
                index,
                len,
                resp: String::new(),
            })?
            .kind
        {
            EventKind::ControlIn { resp, .. } => {
                decode_hex(&resp).map_err(|e| ReplayError::Data(e.to_string()))
            }
            _ => unreachable!("kind checked by next"),
        }
    }
    pub fn bulk_in(&mut self, max: usize, timeout_ms: u64) -> Result<Vec<u8>, ReplayError> {
        match self
            .next(EventKind::BulkIn {
                max,
                timeout_ms,
                raw: String::new(),
            })?
            .kind
        {
            EventKind::BulkIn { raw, .. } => {
                decode_hex(&raw).map_err(|e| ReplayError::Data(e.to_string()))
            }
            _ => unreachable!("kind checked by next"),
        }
    }
    pub fn bulk_out(&mut self, data: &[u8]) -> Result<(), ReplayError> {
        self.next(EventKind::BulkOut {
            data: Payload::Inline {
                data: encode_hex(data),
            },
        })
        .map(|_| ())
    }
    pub fn sleep(&mut self, ms: u64) -> Result<(), ReplayError> {
        self.next(EventKind::Sleep { ms }).map(|event| {
            self.virtual_us = self.virtual_us.max(event.t_us + ms * 1000);
        })
    }
    pub fn virtual_us(&self) -> u64 {
        self.virtual_us
    }
    pub fn finish(self) -> Result<(), ReplayError> {
        let remaining = self.transcript.events.len() - self.index;
        if remaining == 0 {
            Ok(())
        } else {
            Err(ReplayError::Unconsumed { remaining })
        }
    }
}

fn matches_kind(expected: &EventKind, actual: &EventKind) -> bool {
    match (expected, actual) {
        (EventKind::Open, EventKind::Open)
        | (EventKind::Reopen, EventKind::Reopen)
        | (EventKind::Close, EventKind::Close) => true,
        (
            EventKind::ControlOut {
                req: a,
                value: b,
                index: c,
            },
            EventKind::ControlOut {
                req: x,
                value: y,
                index: z,
            },
        ) => (a, b, c) == (x, y, z),
        (
            EventKind::ControlIn {
                req: a,
                value: b,
                index: c,
                len: d,
                ..
            },
            EventKind::ControlIn {
                req: x,
                value: y,
                index: z,
                len: w,
                ..
            },
        ) => (a, b, c, d) == (x, y, z, w),
        (EventKind::BulkIn { max: a, .. }, EventKind::BulkIn { max: b, .. }) => a == b,
        (
            EventKind::BulkOut {
                data: Payload::Inline { data: a },
            },
            EventKind::BulkOut {
                data: Payload::Inline { data: b },
            },
        ) => a == b,
        (EventKind::Sleep { ms: a }, EventKind::Sleep { ms: b }) => a == b,
        _ => false,
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReplayError {
    #[error("invalid transcript: {0}")]
    Schema(String),
    #[error("replay diverged at event {index}: expected {expected:?}, got {actual:?}")]
    Divergence {
        index: usize,
        expected: EventKind,
        actual: EventKind,
    },
    #[error("replay exhausted at event {index}: got {actual:?}")]
    Exhausted { index: usize, actual: EventKind },
    #[error("replay has {remaining} unconsumed events")]
    Unconsumed { remaining: usize },
    #[error("invalid replay data: {0}")]
    Data(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::{DeviceIdentity, SCHEMA};
    fn transcript(events: Vec<EventKind>) -> Transcript {
        Transcript {
            schema: SCHEMA.into(),
            recorded_at: String::new(),
            tool: "test".into(),
            device: DeviceIdentity {
                serial: "x".into(),
                bcd_device: 0,
                vid: 0x403,
                pid: 0xdc48,
            },
            scenario: "test".into(),
            notes: vec![],
            events: events
                .into_iter()
                .enumerate()
                .map(|(i, kind)| Event {
                    i: i as u64,
                    t_us: i as u64 * 100,
                    kind,
                })
                .collect(),
        }
    }

    #[test]
    fn serves_recorded_and_finishes() -> Result<(), ReplayError> {
        let mut replay = Replay::new(transcript(vec![
            EventKind::Open,
            EventKind::ControlIn {
                req: 1,
                value: 2,
                index: 3,
                len: 2,
                resp: "e0 f8".into(),
            },
            EventKind::Close,
        ]))?;
        replay.open()?;
        assert_eq!(replay.control_in(1, 2, 3, 2)?, [0xe0, 0xf8]);
        replay.close()?;
        replay.finish()
    }
    #[test]
    fn divergence_and_unconsumed_are_errors() -> Result<(), ReplayError> {
        let mut replay = Replay::new(transcript(vec![EventKind::Open, EventKind::Close]))?;
        assert!(matches!(
            replay.reopen(),
            Err(ReplayError::Divergence { index: 0, .. })
        ));
        assert_eq!(
            replay.finish(),
            Err(ReplayError::Unconsumed { remaining: 2 })
        );
        Ok(())
    }
}
