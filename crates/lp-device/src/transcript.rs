use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "lp-usb-transcript/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub serial: String,
    pub bcd_device: u16,
    pub vid: u16,
    pub pid: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub schema: String,
    pub recorded_at: String,
    pub tool: String,
    pub device: DeviceIdentity,
    pub scenario: String,
    #[serde(default)]
    pub notes: Vec<String>,
    pub events: Vec<Event>,
}

impl Transcript {
    pub fn validate(&self) -> Result<(), SchemaError> {
        if self.schema != SCHEMA {
            return Err(SchemaError::Schema(self.schema.clone()));
        }
        for (position, event) in self.events.iter().enumerate() {
            if event.i != position as u64 {
                return Err(SchemaError::Index {
                    position,
                    got: event.i,
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub i: u64,
    pub t_us: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    Open,
    ControlOut {
        req: u8,
        value: u16,
        index: u16,
    },
    ControlIn {
        req: u8,
        value: u16,
        index: u16,
        len: usize,
        resp: String,
    },
    BulkOut {
        data: Payload,
    },
    BulkIn {
        max: usize,
        timeout_ms: u64,
        raw: String,
    },
    Sleep {
        ms: u64,
    },
    Note {
        text: String,
    },
    Reopen,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Payload {
    Inline { data: String },
    Reference { data_ref: DataRef },
    Digest { data_sha256: String, len: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataRef {
    pub file: String,
    pub image: u8,
    pub offset: u64,
    pub len: usize,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SchemaError {
    #[error("unsupported transcript schema: {0}")]
    Schema(String),
    #[error("event at position {position} has index {got}")]
    Index { position: usize, got: u64 },
}

pub fn encode_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn decode_hex(value: &str) -> Result<Vec<u8>, HexError> {
    value
        .split_whitespace()
        .map(|part| u8::from_str_radix(part, 16).map_err(|_| HexError(part.to_owned())))
        .collect()
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid transcript hex byte: {0}")]
pub struct HexError(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_json_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let transcript = Transcript {
            schema: SCHEMA.into(),
            recorded_at: "2026-08-25T00:00:00Z".into(),
            tool: "test".into(),
            device: DeviceIdentity {
                serial: "23201984".into(),
                bcd_device: 0x400,
                vid: 0x403,
                pid: 0xdc48,
            },
            scenario: "warm".into(),
            notes: vec![],
            events: vec![Event {
                i: 0,
                t_us: 0,
                kind: EventKind::Open,
            }],
        };
        let json = serde_json::to_string(&transcript)?;
        let decoded: Transcript = serde_json::from_str(&json)?;
        decoded.validate()?;
        assert_eq!(decoded, transcript);
        Ok(())
    }

    #[test]
    fn hex_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = [0xc2, 1, 0, 8, 0];
        assert_eq!(decode_hex(&encode_hex(&bytes))?, bytes);
        Ok(())
    }
}
