use lp_device::{
    clock::WallClock,
    link::{Link, LinkConfig},
    real::RealTransport,
};
use lp_proto::regs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !std::env::args().any(|arg| arg == "--i-understand-this-configures-hardware") {
        return Err("refusing hardware mutation without explicit acknowledgement".into());
    }
    let transport = RealTransport::open()?;
    let mut link = Link::new(transport, WallClock::default(), LinkConfig::default());
    let ccf = lp_ccf::Ccf::load("fixtures/vendor/LogicPort.ccf", true)?;
    let upload = ccf.image_for_upload(7)?;
    let configured = link.configure_fpga(&upload, 7, false)?;
    if configured.id != 0x17 {
        return Err(format!("expected IMAGE_ID 0x17, got 0x{:02x}", configured.id).into());
    }
    for iteration in 1..=100 {
        let id = link.read(regs::ctrl::IMAGE_ID, 1)?;
        if id != [0x17] {
            return Err(format!("IMAGE_ID iteration {iteration}: {id:02x?}").into());
        }
        println!("IMAGE_ID iteration {iteration}=0x17");
    }
    Ok(())
}
