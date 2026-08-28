use crate::stimulus::Stimulus;
use lp_proto::slot::{Sample, Slot};
use std::collections::VecDeque;

pub const PAGES: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnginePhase {
    Idle,
    Prefill,
    Armed,
    Postfill,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineSnapshot {
    pub phase: EnginePhase,
    pub status: u8,
    pub wr_ptr: u16,
    pub post_count: u16,
    pub trig_page: u16,
    pub pages: usize,
}

pub struct Engine {
    phase: EnginePhase,
    ring: VecDeque<Slot>,
    wr_ptr: u16,
    pre_target: usize,
    post_target: usize,
    pre_seen: usize,
    post_seen: usize,
    trig_page: u16,
    next_sample: u64,
    stimulus: Stimulus,
    compressed: bool,
}

impl Engine {
    pub fn new(stimulus: Stimulus, compressed: bool) -> Self {
        Self {
            phase: EnginePhase::Idle,
            ring: VecDeque::with_capacity(PAGES),
            wr_ptr: 2047,
            pre_target: 0,
            post_target: 0,
            pre_seen: 0,
            post_seen: 0,
            trig_page: 0,
            next_sample: 0,
            stimulus,
            compressed,
        }
    }
    pub fn arm(&mut self, pre: usize, post: usize) {
        self.phase = EnginePhase::Prefill;
        self.pre_target = pre.min(PAGES);
        self.post_target = post.min(PAGES);
        self.pre_seen = 0;
        self.post_seen = 0;
    }
    pub fn trigger(&mut self) {
        if self.phase == EnginePhase::Armed {
            self.phase = EnginePhase::Postfill;
            self.trig_page = self.wr_ptr;
            if self.post_target == 0 {
                self.phase = EnginePhase::Complete;
            }
        }
    }
    pub fn force_from_prefill(&mut self) {
        if self.phase == EnginePhase::Prefill {
            let remaining = self.pre_target.saturating_sub(self.pre_seen);
            self.tick(remaining);
        }
        self.trigger();
    }
    pub fn force_stop(&mut self) {
        if self.phase == EnginePhase::Postfill {
            self.phase = EnginePhase::Complete;
        }
    }
    pub fn tick(&mut self, count: usize) {
        for _ in 0..count {
            if matches!(
                self.phase,
                EnginePhase::Idle | EnginePhase::Armed | EnginePhase::Complete
            ) {
                break;
            }
            let bits = self.stimulus.bits_at(self.next_sample);
            self.next_sample += 1;
            self.push(bits);
            match self.phase {
                EnginePhase::Prefill => {
                    self.pre_seen += 1;
                    if self.pre_seen >= self.pre_target {
                        self.phase = EnginePhase::Armed;
                    }
                }
                EnginePhase::Postfill => {
                    self.post_seen += 1;
                    if self.post_seen >= self.post_target {
                        self.phase = EnginePhase::Complete;
                    }
                }
                _ => {}
            }
        }
    }
    fn push(&mut self, bits: u32) {
        let preceding = self.ring.iter().rev().find_map(|slot| match slot {
            Slot::Data { bits, .. } => Some(*bits),
            Slot::Run { .. } => None,
        });
        if self.compressed && preceding == Some(bits) {
            if let Some(Slot::Run { count }) = self.ring.back_mut()
                && *count < (1u64 << 35) - 1
            {
                *count += 1;
                return;
            }
            self.ring.push_back(Slot::Run { count: 1 });
        } else {
            self.ring.push_back(Slot::Data {
                bits,
                clk1: false,
                clk2: false,
            });
        }
        while self.ring.len() > PAGES {
            self.ring.pop_front();
        }
        self.wr_ptr = self.wr_ptr.wrapping_add(1) & 0x07ff;
    }
    pub fn halt(&mut self) {
        self.phase = EnginePhase::Idle;
    }
    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            phase: self.phase,
            status: self.status(),
            wr_ptr: self.wr_ptr,
            post_count: self.post_seen.min(2047) as u16,
            trig_page: self.trig_page,
            pages: self.ring.len(),
        }
    }
    pub fn status(&self) -> u8 {
        match self.phase {
            EnginePhase::Idle | EnginePhase::Complete => 0,
            EnginePhase::Prefill => 0x01,
            EnginePhase::Armed => 0x41,
            EnginePhase::Postfill => 0x61,
        }
    }
    pub fn samples(&self) -> Result<Vec<Sample>, lp_proto::ProtoError> {
        let slots = self.ring.iter().cloned().collect::<Vec<_>>();
        lp_proto::slot::slots_to_samples(&slots)
    }
    pub fn slots(&self) -> &VecDeque<Slot> {
        &self.ring
    }
    pub fn phase(&self) -> EnginePhase {
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seed::SimSeed;
    fn engine(compressed: bool) -> Engine {
        Engine::new(Stimulus::new(SimSeed::default(), 10_000_000), compressed)
    }
    #[test]
    fn status_sequence_and_pre_post_split() {
        let mut e = engine(false);
        e.arm(3, 2);
        assert_eq!(e.status(), 1);
        e.tick(3);
        assert_eq!(e.status(), 0x41);
        e.trigger();
        assert_eq!(e.status(), 0x61);
        e.tick(1);
        assert_eq!(e.status(), 0x61);
        e.tick(1);
        assert_eq!(e.status(), 0);
        assert_eq!(e.snapshot().post_count, 2);
    }

    #[test]
    fn phase_forces_preserve_captured_data() {
        let mut e = engine(false);
        e.arm(3, 4);
        e.force_from_prefill();
        assert_eq!(e.phase(), EnginePhase::Postfill);
        assert_eq!(e.samples().map(|samples| samples.len()), Ok(3));
        e.tick(1);
        e.force_stop();
        assert_eq!(e.phase(), EnginePhase::Complete);
        assert_eq!(e.samples().map(|samples| samples.len()), Ok(4));
    }
    #[test]
    fn ring_wraps_at_2048_pages() {
        let mut e = engine(false);
        e.arm(PAGES, 1);
        e.tick(PAGES);
        e.trigger();
        e.tick(1);
        assert_eq!(e.snapshot().pages, PAGES);
        assert_eq!(e.snapshot().wr_ptr, 0);
    }
    #[test]
    fn compressed_data_run_roundtrips() {
        let mut e = engine(true);
        e.arm(20, 0);
        e.tick(20);
        assert!(e.slots().iter().any(|s| matches!(s, Slot::Run { .. })));
        let samples = e.samples().unwrap_or_else(|x| panic!("{x}"));
        assert_eq!(lp_proto::rle::total_len(&samples), 20);
    }
}
