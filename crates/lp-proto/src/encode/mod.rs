pub mod mode;
pub mod rate;
pub mod tables;
pub mod threshold;
pub mod trigger;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Provisional,
    Verified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegWrite {
    pub addr: crate::addr::Addr,
    pub value: u8,
    pub provenance: Provenance,
}
