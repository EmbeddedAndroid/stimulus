use nusb::MaybeFuture;

pub const VID: u16 = 0x0403;
pub const PID: u16 = 0xdc48;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEvidence {
    pub serial: String,
    pub bcd_device: u16,
}

pub fn require_device() -> Result<DeviceEvidence, String> {
    let devices = nusb::list_devices()
        .wait()
        .map_err(|error| format!("HIL_ENUM_FAILED: {error}"))?
        .filter(|device| device.vendor_id() == VID && device.product_id() == PID)
        .collect::<Vec<_>>();
    match devices.as_slice() {
        [] => Err("HIL_NO_DEVICE: no 0403:dc48 analyzer is attached".to_owned()),
        [device] => Ok(DeviceEvidence {
            serial: device.serial_number().unwrap_or("unknown").to_owned(),
            bcd_device: device.device_version(),
        }),
        _ => Err(format!(
            "HIL_DEVICE_AMBIGUOUS: expected one 0403:dc48 analyzer, found {}",
            devices.len()
        )),
    }
}
