pub mod addr;
pub mod bank;
pub mod decode;
pub mod encode;
pub mod freq;
pub mod packet;
pub mod readback;
pub mod regs;
pub mod rle;
pub mod setup_seq;
pub mod slot;
pub mod status;
pub mod wirestatus;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtoError {
    #[error("length must be in 1..=65536, got {0}")]
    InvalidLength(u32),
    #[error("unexpected response opcode: wanted 0x{expected:02x}, got 0x{got:02x}")]
    Opcode { expected: u8, got: u8 },
    #[error("packet sequence mismatch: wanted {expected}, got {got}")]
    PacketNumber { expected: u16, got: u16 },
    #[error("wire status must contain exactly 9 bytes, got {0}")]
    WireStatusLength(usize),
    #[error("RAM block must be in 1..=4, got {0}")]
    InvalidBlock(u8),
    #[error("page must be in 0..=2047, got {0}")]
    InvalidPage(u16),
    #[error("run-length slot cannot appear before a data slot")]
    RunWithoutData,
    #[error("run length exceeds the 35-bit hardware limit: {0}")]
    RunTooLong(u64),
    #[error("unknown sample rate: {0} Hz")]
    UnknownRate(u64),
    #[error("compression is not supported above 200 MHz")]
    CompressionRate,
    #[error("invalid state clock code: {0}")]
    InvalidStateCode(u8),
}
