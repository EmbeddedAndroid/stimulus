use crate::addr::Addr;
use crate::packet::Cmd;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BankTracker {
    current: Option<u16>,
}

impl BankTracker {
    pub fn plan(&mut self, addr: Addr) -> Option<Cmd> {
        let bank = addr.bank();
        if self.current == Some(bank) {
            None
        } else {
            self.current = Some(bank);
            Some(Cmd::Bank(bank))
        }
    }

    pub fn invalidate(&mut self) {
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_select_only_on_change() {
        let mut banks = BankTracker::default();
        assert_eq!(banks.plan(Addr(0x0010_0001)), Some(Cmd::Bank(0x0010)));
        assert_eq!(banks.plan(Addr(0x0010_ffff)), None);
        assert_eq!(banks.plan(Addr(0x0200_0000)), Some(Cmd::Bank(0x0200)));
        banks.invalidate();
        assert_eq!(banks.plan(Addr(0x0200_0001)), Some(Cmd::Bank(0x0200)));
    }
}
