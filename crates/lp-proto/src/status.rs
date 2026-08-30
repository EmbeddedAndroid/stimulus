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
        // The engine is active while either activity bit is set: bit0 (the
        // sim/legacy encoding) or bit1 (the real hardware, which holds 0x52
        // while armed and 0x73 during postfill, dropping to 0x50 = complete).
        self.0 & 0x03 != 0
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
        // Real hardware states.
        assert_eq!(AcqStatus(0x50).phase(), Phase::Complete);
        assert_eq!(AcqStatus(0x52).phase(), Phase::Armed);
        assert_eq!(AcqStatus(0x73).phase(), Phase::Postfill);
        // Sim/legacy states.
        assert_eq!(AcqStatus(1).phase(), Phase::Prefill);
        assert_eq!(AcqStatus(0x41).phase(), Phase::Armed);
        assert_eq!(AcqStatus(0x61).phase(), Phase::Postfill);
    }
}
