fn main() -> anyhow::Result<()> {
    let info = lp_ftdi::probe()?;
    println!(
        "opened 0403:dc48 serial {} product={} pins=0x{:02x}",
        info.serial, info.product, info.pins
    );
    Ok(())
}
