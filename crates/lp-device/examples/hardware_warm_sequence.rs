use lp_device::{
    clock::WallClock,
    link::{Link, LinkConfig},
    real::RealTransport,
    recording::RecordingTransport,
    transcript::{SCHEMA, Transcript},
};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !std::env::args().any(|arg| arg == "--i-understand-this-reconfigures-hardware") {
        return Err(
            "refusing hardware mutation without --i-understand-this-reconfigures-hardware".into(),
        );
    }
    let evidence = Path::new("evidence/hardware");
    std::fs::create_dir_all(evidence)?;
    let real = RealTransport::open()?;
    let identity = lp_device::transport::Transport::identity(&real);
    let transcript = Transcript {
        schema: SCHEMA.into(),
        recorded_at: "2026-08-26T00:00:00-04:00".into(),
        tool: "hardware_warm_sequence".into(),
        device: identity.clone(),
        scenario: "persistent-session-warm-reconfigure".into(),
        notes: vec![
            "one persistent Link; EEPROM writes unavailable".into(),
            "recording windows are saved without closing or reopening the host session".into(),
        ],
        events: vec![],
    };
    let recording = RecordingTransport::new(real, transcript);
    let mut link = Link::new(recording, WallClock::default(), LinkConfig::default());
    let ccf = lp_ccf::Ccf::load("fixtures/vendor/LogicPort.ccf", true)?;
    let image7 = ccf.image_for_upload(7)?;
    let image6 = ccf.image_for_upload(6)?;

    macro_rules! configure_stage {
        ($expression:expr, $name:literal) => {
            match $expression {
                Ok(outcome) => outcome,
                Err(error) => {
                    let failure = evidence.join(concat!("warm-sequence-failure-", $name, ".json"));
                    let _ = link.save_recording(&failure);
                    link.abort_fpga_configure();
                    return Err(format!(
                        "stage {} failed: {}; transcript={}",
                        $name,
                        error,
                        failure.display()
                    )
                    .into());
                }
            }
        };
    }

    eprintln!("stage=cold-image-7");
    let cold = configure_stage!(link.configure_fpga(&image7, 7, false), "cold-image-7");
    if cold.id != 0x17 {
        return Err(format!("cold image 7 verification returned 0x{:02x}", cold.id).into());
    }

    link.save_recording(evidence.join("configure-image-7-sequence-cold.json"))?;
    link.clear_recording_events();
    eprintln!("stage=warm-image-6-over-7");
    let six = configure_stage!(
        link.configure_fpga(&image6, 6, false),
        "warm-image-6-over-7"
    );
    if six.id != 0x16 || six.warm {
        return Err(format!(
            "warm 7->6 verification returned id=0x{:02x} warm={}",
            six.id, six.warm
        )
        .into());
    }
    link.save_recording(evidence.join("configure-image-6-over-7.json"))?;

    link.clear_recording_events();
    eprintln!("stage=warm-image-7-over-6");
    let seven = configure_stage!(
        link.configure_fpga(&image7, 7, false),
        "warm-image-7-over-6"
    );
    if seven.id != 0x17 || seven.warm {
        return Err(format!(
            "warm 6->7 verification returned id=0x{:02x} warm={}",
            seven.id, seven.warm
        )
        .into());
    }
    link.save_recording(evidence.join("configure-image-7-over-6.json"))?;

    link.clear_recording_events();
    eprintln!("stage=warm-noop-image-7");
    let no_op = configure_stage!(link.configure_fpga(&image7, 7, false), "warm-noop-image-7");
    if no_op.id != 0x17 || !no_op.warm {
        return Err(format!(
            "same-image no-op returned id=0x{:02x} warm={}",
            no_op.id, no_op.warm
        )
        .into());
    }
    let (recording, _) = link.into_parts();
    recording.finish(evidence.join("warm-noop.json"))?;

    println!(
        "serial={} cold=0x{:02x} warm_6=0x{:02x} warm_7=0x{:02x} noop=0x{:02x}",
        identity.serial, cold.id, six.id, seven.id, no_op.id
    );
    Ok(())
}
