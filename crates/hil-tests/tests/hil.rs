#![cfg(feature = "hardware")]

use hil_tests::{gate, verdict};
use std::time::Instant;

#[test]
fn connect_enumerate_by_vid_pid() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let device = gate::require_device()?;
    if device.serial != "23201984" || device.bcd_device != 0x0400 {
        return Err(format!(
            "unexpected analyzer identity: serial={} bcd=0x{:04x}",
            device.serial, device.bcd_device
        )
        .into());
    }
    verdict::append(&verdict::Verdict {
        test_id: "hil::connect::enumerate_by_vid_pid",
        gate: "D1",
        op_ids: &["device.enumerate"],
        pass: true,
        measured: serde_json::json!({"serial":device.serial,"bcd_device":device.bcd_device}),
        expected: serde_json::json!({"vid":0x0403,"pid":0xdc48,"serial":"23201984","bcd_device":0x0400}),
        tolerance: serde_json::Value::Null,
        duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        transcript: None,
    })?;
    Ok(())
}
