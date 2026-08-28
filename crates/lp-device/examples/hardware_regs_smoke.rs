use lp_device::{
    clock::WallClock,
    link::{Link, LinkConfig},
    real::RealTransport,
    recording::RecordingTransport,
    transcript::{SCHEMA, Transcript},
};
use lp_proto::regs;
use std::{path::PathBuf, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let output = args
        .windows(2)
        .find(|pair| pair[0] == "--record")
        .map(|pair| PathBuf::from(&pair[1]))
        .unwrap_or_else(|| PathBuf::from("evidence/hardware/regs-smoke.json"));
    let cold_config_record = args
        .windows(2)
        .find(|pair| pair[0] == "--cold-config-record")
        .map(|pair| PathBuf::from(&pair[1]));
    if cold_config_record.is_some()
        && !args
            .iter()
            .any(|arg| arg == "--i-understand-this-reconfigures-hardware")
    {
        return Err(
            "refusing hardware mutation without --i-understand-this-reconfigures-hardware".into(),
        );
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let real = RealTransport::open()?;
    let identity = lp_device::transport::Transport::identity(&real);
    let transcript = Transcript {
        schema: SCHEMA.into(),
        recorded_at: "2026-08-25T00:00:00-04:00".into(),
        tool: "hardware_regs_smoke".into(),
        device: identity.clone(),
        scenario: "post-warm-reconfigure-register-smoke".into(),
        notes: vec![
            "register smoke; writes only the volatile frequency-counter source, never EEPROM"
                .into(),
            "hardware-observed reset divider is 0000; 2100 is the vendor-written 10 MHz setting"
                .into(),
        ],
        events: vec![],
    };
    let recording = RecordingTransport::new(real, transcript);
    let mut link = Link::new(recording, WallClock::default(), LinkConfig::default());
    // Establish the vendor open-session handshake on the same connection.  On
    // an image-7 device this short-circuits after IMAGE_ID and streams nothing.
    let result = (|| -> Result<_, Box<dyn std::error::Error>> {
        let ccf = lp_ccf::Ccf::load("fixtures/vendor/LogicPort.ccf", true)?;
        let upload = ccf.image_for_upload(7)?;
        let configured = link.configure_fpga(&upload, 7, cold_config_record.is_some())?;
        if configured.id != 0x17 {
            return Err(format!(
                "register smoke expected image 7, got 0x{:02x} (warm={})",
                configured.id, configured.warm
            )
            .into());
        }
        if let Some(path) = &cold_config_record {
            link.save_recording(path)?;
            link.clear_recording_events();
        }
        // The FPGA returns only one payload byte for a combined two-byte C2
        // divider request. Match the vendor smoke and read offsets 0 and 1 as
        // separate byte transactions.
        let r0 = link
            .read(regs::rate::R0, 1)?
            .first()
            .copied()
            .ok_or("divider R0 returned no byte")?;
        let r1 = link
            .read(regs::rate::R1, 1)?
            .first()
            .copied()
            .ok_or("divider R1 returned no byte")?;
        let rate = vec![r0, r1];
        let wire = link.read(regs::ctrl::WIRE_STATUS, 9)?;
        // The vendor starts a 100 ms counter gate by writing its source, then
        // polls the aliased READY byte until it clears before reading VALUE.
        // Reading VALUE directly can return a partial response and wedge the
        // FIFO parser while the gate is still active.
        link.write(regs::freq::SRC, &[0])?;
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            let ready = link.read(regs::freq::READY, 1)?;
            if ready == [0] {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("frequency counter stayed busy: {}", hex(&ready)).into());
            }
        }
        let frequency = link.read(regs::freq::VALUE, 4)?;
        Ok((rate, wire, frequency))
    })();
    if result.is_err() {
        link.abort_fpga_configure();
    }
    let (recording, _) = link.into_parts();
    recording.finish(&output)?;
    let (rate, wire, frequency) = result?;
    println!(
        "serial={} rate={} wire_status={} frequency={} transcript={}",
        identity.serial,
        hex(&rate),
        hex(&wire),
        hex(&frequency),
        output.display()
    );
    if rate != [0x00, 0x00] {
        return Err(format!("unexpected power-on rate divider: {}", hex(&rate)).into());
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}
