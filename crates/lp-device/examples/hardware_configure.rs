use lp_device::{
    clock::WallClock,
    link::{Link, LinkConfig},
    real::RealTransport,
    recording::RecordingTransport,
    transcript::{SCHEMA, Transcript},
};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    if !args
        .iter()
        .any(|arg| arg == "--i-understand-this-reconfigures-hardware")
    {
        return Err(
            "refusing hardware mutation without --i-understand-this-reconfigures-hardware".into(),
        );
    }
    let image = args
        .windows(2)
        .find(|pair| pair[0] == "--image")
        .and_then(|pair| pair[1].parse::<u8>().ok())
        .unwrap_or(7);
    if image > 7 {
        return Err("image must be 0..=7".into());
    }
    let output = args
        .windows(2)
        .find(|pair| pair[0] == "--record")
        .map(|pair| PathBuf::from(&pair[1]))
        .unwrap_or_else(|| PathBuf::from("evidence/hardware/configure.json"));
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let real = RealTransport::open()?;
    let identity = lp_device::transport::Transport::identity(&real);
    let transcript = Transcript {
        schema: SCHEMA.into(),
        recorded_at: "2026-08-25T00:00:00-04:00".into(),
        tool: "hardware_configure".into(),
        device: identity.clone(),
        scenario: format!("configure-image-{image}"),
        notes: vec![
            "explicit operator-authorized FPGA reconfiguration; EEPROM writes unavailable".into(),
        ],
        events: vec![],
    };
    let recording = RecordingTransport::new(real, transcript);
    let mut link = Link::new(recording, WallClock::default(), LinkConfig::default());
    let ccf = lp_ccf::Ccf::load("fixtures/vendor/LogicPort.ccf", true)?;
    let upload = ccf.image_for_upload(image)?;
    let force = args.iter().any(|arg| arg == "--force");
    let warm_noop_after = args.iter().any(|arg| arg == "--warm-noop-after");
    let mut outcome = link.configure_fpga(&upload, image, force);
    if outcome.is_ok() && warm_noop_after {
        link.clear_recording_events();
        outcome = link.configure_fpga(&upload, image, false);
    }
    if outcome.is_err() {
        link.abort_fpga_configure();
    }
    let (recording, _) = link.into_parts();
    recording.finish(&output)?;
    let outcome = outcome?;
    println!(
        "configured serial={} image=0x{:02x} version=0x{:04x} warm={} elapsed_ms={} drained_bytes={} transcript={}",
        identity.serial,
        outcome.id,
        outcome.version,
        outcome.warm,
        outcome.elapsed.as_millis(),
        outcome.drained_bytes,
        output.display()
    );
    Ok(())
}
