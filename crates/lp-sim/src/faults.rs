use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    DropPktno,
    CorruptOpcode,
    StallDone,
    UsbTimeout,
}

impl FromStr for Fault {
    type Err = FaultParseError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "drop-pktno" => Ok(Self::DropPktno),
            "corrupt-opcode" => Ok(Self::CorruptOpcode),
            "stall-done" => Ok(Self::StallDone),
            "usb-timeout" => Ok(Self::UsbTimeout),
            other => Err(FaultParseError(other.into())),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("unknown simulator fault: {0}")]
pub struct FaultParseError(String);
