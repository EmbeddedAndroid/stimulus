#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Complete,
    Prefill,
    Armed,
    Postfill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcqStatus(pub u8);

impl AcqStatus {
    pub const fn acquiring(self) -> bool {
        self.0 & 1 != 0
    }
    pub const fn prefill_done(self) -> bool {
        self.0 & 0x40 != 0
    }
    pub const fn triggered(self) -> bool {
        self.0 & 0x20 != 0
    }
    pub const fn phase(self) -> Phase {
        if !self.acquiring() {
            Phase::Complete
        } else if !self.prefill_done() {
            Phase::Prefill
        } else if !self.triggered() {
            Phase::Armed
        } else {
            Phase::Postfill
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_truth_table() {
        assert_eq!(AcqStatus(0xfe).phase(), Phase::Complete);
        assert_eq!(AcqStatus(1).phase(), Phase::Prefill);
        assert_eq!(AcqStatus(0x41).phase(), Phase::Armed);
        assert_eq!(AcqStatus(0x61).phase(), Phase::Postfill);
    }
}
