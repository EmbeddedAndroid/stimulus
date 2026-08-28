use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Freshness {
    pub device_epoch: u64,
    pub capture_id: Option<String>,
    pub capture_seq: u64,
    pub acq_state: String,
    pub ts: String,
}
