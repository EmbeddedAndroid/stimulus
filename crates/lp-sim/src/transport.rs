use crate::{
    engine::{Engine, EnginePhase, EngineSnapshot},
    faults::Fault,
    regfile::RegFile,
    seed::{SimSeed, StartState},
    stimulus::Stimulus,
};
use lp_device::{
    transcript::DeviceIdentity,
    transport::{Transport, TransportError},
};
use lp_proto::{
    addr::Addr,
    packet::{BANK, READ, WRITE},
    regs,
};
use std::{collections::VecDeque, time::Duration};

const IMAGE_LENGTHS: [usize; 8] = [
    1_057_840, 1_044_512, 1_046_896, 1_047_328, 1_046_512, 1_054_992, 1_031_232, 1_058_384,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimSnapshot {
    pub pins: u8,
    pub bank: u16,
    pub packet_number: u16,
    pub configured_image: Option<u8>,
    pub streamed_bytes: usize,
    pub engine: EngineSnapshot,
}

pub struct SimTransport {
    seed: SimSeed,
    regs: RegFile,
    pins: u8,
    bitbang: bool,
    configured_image: Option<u8>,
    streamed: usize,
    bank: u16,
    packet_number: u16,
    pending_write: Option<(Addr, usize)>,
    incoming: VecDeque<Vec<u8>>,
    fault_used: bool,
    engine: Engine,
}

impl SimTransport {
    pub fn new(seed: SimSeed) -> Self {
        let (regs, pins, configured_image) = match seed.start_state {
            StartState::Cold => (RegFile::cold(), 0xe0, None),
            StartState::Warm { image } => (RegFile::warm(image), 0xf8, Some(image)),
        };
        let engine = Engine::new(Stimulus::new(seed.clone(), 10_000_000), false);
        Self {
            seed,
            regs,
            pins,
            bitbang: false,
            configured_image,
            streamed: 0,
            bank: 0,
            packet_number: 0,
            pending_write: None,
            incoming: VecDeque::new(),
            fault_used: false,
            engine,
        }
    }
    pub fn snapshot(&self) -> SimSnapshot {
        SimSnapshot {
            pins: self.pins,
            bank: self.bank,
            packet_number: self.packet_number,
            configured_image: self.configured_image,
            streamed_bytes: self.streamed,
            engine: self.engine.snapshot(),
        }
    }
    pub fn expected_capture(&self) -> Result<Vec<lp_proto::slot::Sample>, lp_proto::ProtoError> {
        self.engine.samples()
    }
    fn response(&mut self, mut opcode: u8, payload: &[u8]) {
        if !self.fault_used && self.seed.faults.contains(&Fault::CorruptOpcode) {
            opcode ^= 1;
            self.fault_used = true;
        }
        let mut number = self.packet_number;
        if !self.fault_used && self.seed.faults.contains(&Fault::DropPktno) {
            number = number.wrapping_add(1);
            self.fault_used = true;
        }
        self.packet_number = number.wrapping_add(1);
        let mut logical = vec![opcode, (number >> 8) as u8, number as u8];
        logical.extend_from_slice(payload);
        let mut raw = Vec::with_capacity(logical.len() + logical.len().div_ceil(62) * 2);
        for packet in logical.chunks(62) {
            raw.extend_from_slice(&[0x31, 0x60]);
            raw.extend_from_slice(packet);
        }
        self.incoming.push_back(raw);
    }
    fn fifo_out(&mut self, data: &[u8]) -> Result<(), TransportError> {
        if let Some((addr, len)) = self.pending_write.take() {
            if data.len() != len {
                return Err(TransportError(format!(
                    "sim write payload: expected {len} bytes, got {}",
                    data.len()
                )));
            }
            self.regs.write(addr, data);
            self.write_hook(addr, data);
            self.response(WRITE, &[]);
            return Ok(());
        }
        // FIFO prime tail: the vendor primes the parser with a long run of zeros
        // (65535 then 30000) that the parser ignores, then a single 0x01 that
        // resets the packet counter for the new session.
        if !data.is_empty() && data.iter().all(|&b| b == 0) {
            return Ok(());
        }
        if data == [1] {
            self.packet_number = 0;
            return Ok(());
        }
        if data.len() < 5 {
            return Err(TransportError(format!(
                "sim command must contain a 5-byte header, got {} bytes",
                data.len()
            )));
        }
        let opcode = data[0];
        let low = u16::from_le_bytes([data[1], data[2]]);
        let len = u16::from_le_bytes([data[3], data[4]]) as usize + 1;
        match opcode {
            BANK => {
                if data.len() != 5 {
                    return Err(TransportError("sim bank command has trailing bytes".into()));
                }
                self.bank = low;
                self.response(BANK, &[]);
            }
            READ => {
                if data.len() != 5 {
                    return Err(TransportError("sim read command has trailing bytes".into()));
                }
                let value = self.regs.read(Addr::new(self.bank, low), len);
                self.response(READ, &value);
                self.after_read(Addr::new(self.bank, low));
            }
            WRITE => {
                let addr = Addr::new(self.bank, low);
                let payload = &data[5..];
                if payload.is_empty() {
                    self.pending_write = Some((addr, len));
                } else if payload.len() == len {
                    self.regs.write(addr, payload);
                    self.write_hook(addr, payload);
                    self.response(WRITE, &[]);
                } else {
                    return Err(TransportError(format!(
                        "sim write payload: expected {len} bytes, got {}",
                        payload.len()
                    )));
                }
            }
            _ => return Err(TransportError(format!("unknown sim opcode 0x{opcode:02x}"))),
        }
        Ok(())
    }
    fn write_hook(&mut self, addr: Addr, data: &[u8]) {
        if addr == regs::ctrl::CMD_PORT && data == [0] {
            self.configured_image = None;
            self.pins = 0xe8;
            self.streamed = 0;
        } else if addr == regs::ctrl::ARM {
            if data.first() == Some(&1) {
                let pre = usize::from(self.regs.read16(regs::ctrl::PRE_COUNT));
                let post = usize::from(self.regs.read16(regs::ctrl::POST_COUNT));
                self.engine.arm(pre, post);
            } else {
                self.engine.halt();
            }
        } else if addr == regs::ctrl::RESET {
            self.engine.halt();
        } else if addr == regs::ctrl::FORCE_FROM_PREFILL {
            self.engine.force_from_prefill();
        } else if addr == regs::ctrl::FORCE_FROM_ARMED {
            self.engine.trigger();
        } else if addr == regs::ctrl::FORCE_STOP_POSTFILL {
            self.engine.force_stop();
        }
        self.sync_engine_registers();
    }
    fn after_read(&mut self, addr: Addr) {
        if addr == regs::ctrl::STATUS {
            match self.engine.phase() {
                EnginePhase::Prefill => self.engine.tick(64),
                EnginePhase::Armed => self.engine.trigger(),
                EnginePhase::Postfill => self.engine.tick(64),
                _ => {}
            }
            self.sync_engine_registers();
        }
    }
    fn sync_engine_registers(&mut self) {
        let s = self.engine.snapshot();
        self.regs.write(regs::ctrl::STATUS, &[s.status]);
        self.regs.write(regs::ram::WR_PTR, &s.wr_ptr.to_le_bytes());
        self.regs
            .write(regs::ctrl::POST_COUNT_RD, &s.post_count.to_le_bytes());
        self.regs
            .write(regs::cap::TRIG_PAGE, &s.trig_page.to_le_bytes());
        if s.phase == EnginePhase::Complete {
            self.materialize_capture();
        }
    }
    fn materialize_capture(&mut self) {
        let Ok(samples) = self.engine.samples() else {
            return;
        };
        let mut slots = Vec::new();
        for sample in samples {
            slots.push((sample.bits, false, 0_u64));
            if sample.repeat > 0 {
                slots.push((0, true, sample.repeat));
            }
        }
        if slots.len() > 2048 {
            let excess = slots.len() - 2048;
            slots.drain(..excess);
        }
        if slots.is_empty() {
            return;
        }
        for (page, (bits, is_run, count)) in slots.iter().copied().enumerate() {
            let page = page as u16;
            let bytes = if is_run {
                count.to_le_bytes()
            } else {
                bits.to_le_bytes()
            };
            for block in 1..=4 {
                if let Ok(addr) = regs::ram::block(block, page) {
                    self.regs.write(addr, &[bytes[usize::from(block - 1)]]);
                }
            }
            if let Ok(addr) = regs::ram::flags(page) {
                let flags = if is_run {
                    8 | ((count >> 32) as u8 & 7)
                } else {
                    (((bits >> 32) as u8 & 1) << 1) | (((bits >> 33) as u8 & 1) << 2)
                };
                self.regs.write(addr, &[flags]);
            }
        }
        let slot_count = slots.len() as u16;
        self.regs
            .write(regs::ram::WR_PTR, &(slot_count - 1).to_le_bytes());
        self.regs.write(
            regs::ctrl::POST_COUNT_RD,
            &(2048_u16 - slot_count).to_le_bytes(),
        );
        let trigger_slot = slot_count
            .saturating_sub(self.engine.snapshot().post_count)
            .saturating_sub(1);
        let trig_page = slot_count - 1 - trigger_slot;
        self.regs
            .write(regs::cap::TRIG_PAGE, &trig_page.to_le_bytes());
    }
    fn bitbang_out(&mut self, data: &[u8]) {
        if data == [0] {
            if self.configured_image.is_none() {
                self.pins = 0xe0;
                self.streamed = 0;
            }
            return;
        }
        if data == [4] {
            self.pins = 0xe8;
            return;
        }
        self.streamed = self.streamed.saturating_add(data.len());
        if let Some((idx, _)) = IMAGE_LENGTHS
            .iter()
            .enumerate()
            .find(|(_, len)| **len == self.streamed)
            && !self.seed.faults.contains(&Fault::StallDone)
        {
            self.configured_image = Some(idx as u8);
            self.regs.write(regs::ctrl::IMAGE_ID, &[idx as u8 | 0x10]);
            self.pins = 0xf8;
        }
    }
}

impl Transport for SimTransport {
    fn control_out(&mut self, req: u8, value: u16, _index: u16) -> Result<(), TransportError> {
        match req {
            0x00 if value == 1 => self.incoming.clear(),
            0x0b => self.bitbang = (value >> 8) as u8 == 1,
            _ => {}
        }
        Ok(())
    }
    fn control_in(
        &mut self,
        req: u8,
        _value: u16,
        _index: u16,
        len: u16,
    ) -> Result<Vec<u8>, TransportError> {
        if req == 0x0c && len == 1 {
            Ok(vec![self.pins])
        } else {
            Ok(vec![0; usize::from(len)])
        }
    }
    fn bulk_out(&mut self, data: &[u8], _timeout: Duration) -> Result<(), TransportError> {
        if self.bitbang {
            self.bitbang_out(data);
            Ok(())
        } else {
            self.fifo_out(data)
        }
    }
    fn bulk_in_raw(&mut self, _max: usize, _timeout: Duration) -> Result<Vec<u8>, TransportError> {
        if self.seed.faults.contains(&Fault::UsbTimeout) {
            return Ok(vec![]);
        }
        Ok(self
            .incoming
            .pop_front()
            .unwrap_or_else(|| vec![0x31, 0x60]))
    }
    fn reopen(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
    fn identity(&self) -> DeviceIdentity {
        DeviceIdentity {
            serial: format!("SIM-{:016x}", self.seed.rng_seed),
            bcd_device: 0x0400,
            vid: 0x0403,
            pid: 0xdc48,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lp_device::{
        acquisition::{AcquisitionConfig, AcquisitionError, acquire_single, trigger_immediate},
        clock::VirtualClock,
        link::{Link, LinkConfig},
    };
    use lp_proto::status::Phase;
    #[test]
    fn wire_read_write_and_status_framing() -> Result<(), Box<dyn std::error::Error>> {
        let sim = SimTransport::new("start=warm:7".parse()?);
        let mut link = Link::new(sim, VirtualClock::default(), LinkConfig::default());
        link.write(regs::ctrl::ARM, &[1])?;
        assert_eq!(link.read(regs::ctrl::ARM, 1)?, [1]);
        let (sim, _) = link.into_parts();
        assert_eq!(sim.snapshot().bank, 0x10);
        Ok(())
    }
    #[test]
    fn done_only_after_exact_ccf_image_length() {
        let mut sim = SimTransport::new(SimSeed::default());
        sim.bitbang = true;
        sim.bitbang_out(&[4]);
        sim.bitbang_out(&vec![5; IMAGE_LENGTHS[7] - 1]);
        assert_eq!(sim.pins & 0x10, 0);
        sim.bitbang_out(&[5]);
        assert_eq!(sim.snapshot().configured_image, Some(7));
    }
    #[test]
    fn packet_skip_fault_is_injected_once() {
        let seed = SimSeed {
            start_state: StartState::Warm { image: 7 },
            faults: vec![Fault::DropPktno],
            ..SimSeed::default()
        };
        let mut sim = SimTransport::new(seed);
        sim.fifo_out(&[BANK, 0x10, 0, 0, 0])
            .unwrap_or_else(|e| panic!("{e}"));
        let first = sim.incoming.pop_front().unwrap_or_default();
        sim.fifo_out(&[READ, 0x0f, 0, 0, 0])
            .unwrap_or_else(|e| panic!("{e}"));
        let second = sim.incoming.pop_front().unwrap_or_default();
        assert_eq!(&first[3..5], [0, 1]);
        assert_eq!(&second[3..5], [0, 2]);
    }
    #[test]
    fn real_ccf_configures_through_wire_model() -> Result<(), Box<dyn std::error::Error>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/vendor/LogicPort.ccf");
        let ccf = lp_ccf::Ccf::load(path, true)?;
        let sim = SimTransport::new(SimSeed::default());
        let mut link = Link::new(sim, VirtualClock::default(), LinkConfig::default());
        let upload = ccf.image_for_upload(7)?;
        let outcome = link.configure_fpga(&upload, 7, false)?;
        assert_eq!(outcome.id, 0x17);
        let (sim, _) = link.into_parts();
        assert_eq!(sim.snapshot().configured_image, Some(7));
        Ok(())
    }
    #[test]
    fn warm_reconfigure_switches_both_directions_and_noop_only_reads_id()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/vendor/LogicPort.ccf");
        let ccf = lp_ccf::Ccf::load(path, true)?;
        let sim = SimTransport::new("start=warm:7".parse()?);
        let mut link = Link::new(sim, VirtualClock::default(), LinkConfig::default());

        let six = ccf.image_for_upload(6)?;
        let outcome = link.configure_fpga(&six, 6, false)?;
        assert_eq!(outcome.id, 0x16);
        assert!(!outcome.warm);

        let seven = ccf.image_for_upload(7)?;
        let outcome = link.configure_fpga(&seven, 7, false)?;
        assert_eq!(outcome.id, 0x17);
        assert!(!outcome.warm);

        let bytes_before_noop = link.stats().bytes_out;
        let outcome = link.configure_fpga(&seven, 7, false)?;
        assert_eq!(outcome.id, 0x17);
        assert!(outcome.warm);
        assert_eq!(link.stats().bytes_out - bytes_before_noop, 5);
        let (sim, _) = link.into_parts();
        assert_eq!(sim.snapshot().configured_image, Some(7));
        assert_eq!(sim.snapshot().streamed_bytes, IMAGE_LENGTHS[7]);
        Ok(())
    }
    #[test]
    fn arm_and_status_poll_drive_engine() -> Result<(), Box<dyn std::error::Error>> {
        let sim = SimTransport::new("start=warm:7;stim=count16".parse()?);
        let mut link = Link::new(sim, VirtualClock::default(), LinkConfig::default());
        link.write(regs::ctrl::PRE_COUNT, &3u16.to_le_bytes())?;
        link.write(regs::ctrl::POST_COUNT, &2u16.to_le_bytes())?;
        link.write(regs::ctrl::ARM, &[1])?;
        assert_eq!(link.read(regs::ctrl::STATUS, 1)?, [0x01]);
        assert_eq!(link.read(regs::ctrl::STATUS, 1)?, [0x41]);
        assert_eq!(link.read(regs::ctrl::STATUS, 1)?, [0x61]);
        assert_eq!(link.read(regs::ctrl::STATUS, 1)?, [0]);
        let (sim, _) = link.into_parts();
        assert_eq!(sim.snapshot().engine.phase, EnginePhase::Complete);
        assert_eq!(lp_proto::rle::total_len(&sim.expected_capture()?), 5);
        Ok(())
    }

    #[test]
    fn acquisition_controller_matches_simulator_ground_truth()
    -> Result<(), Box<dyn std::error::Error>> {
        let sim = SimTransport::new("start=warm:7;stim=count16".parse()?);
        let mut device = Link::new(sim, VirtualClock::default(), LinkConfig::default());
        device.write(regs::ctrl::PRE_COUNT, &3_u16.to_le_bytes())?;
        device.write(regs::ctrl::POST_COUNT, &2_u16.to_le_bytes())?;
        let mut clock = VirtualClock::default();
        let result = acquire_single(&mut device, &mut clock, AcquisitionConfig::default())?;
        assert_eq!(
            result
                .timeline
                .iter()
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            [
                Phase::Prefill,
                Phase::Armed,
                Phase::Postfill,
                Phase::Complete
            ]
        );
        let expected = device.into_parts().0.expected_capture()?;
        assert_eq!(result.readback.samples, expected);
        Ok(())
    }

    #[test]
    fn trigger_immediate_rejects_idle_device() -> Result<(), Box<dyn std::error::Error>> {
        let sim = SimTransport::new("start=warm:7".parse()?);
        let mut device = Link::new(sim, VirtualClock::default(), LinkConfig::default());
        assert!(matches!(
            trigger_immediate(&mut device),
            Err(AcquisitionError::CannotTrigger(Phase::Complete))
        ));
        Ok(())
    }

    #[test]
    fn acquisition_timeout_forces_each_phase() -> Result<(), Box<dyn std::error::Error>> {
        let sim = SimTransport::new("start=warm:7;stim=count16".parse()?);
        let mut device = Link::new(sim, VirtualClock::default(), LinkConfig::default());
        device.write(regs::ctrl::PRE_COUNT, &3_u16.to_le_bytes())?;
        device.write(regs::ctrl::POST_COUNT, &4_u16.to_le_bytes())?;
        let mut clock = VirtualClock::default();
        let result = acquire_single(
            &mut device,
            &mut clock,
            AcquisitionConfig {
                poll_interval: Duration::from_millis(1),
                prefill_timeout: Some(Duration::ZERO),
                armed_timeout: Some(Duration::ZERO),
                postfill_timeout: Some(Duration::ZERO),
                ..AcquisitionConfig::default()
            },
        )?;
        assert!(result.timeline.iter().any(|event| event.forced));
        assert_eq!(
            result.timeline.last().map(|event| event.phase),
            Some(Phase::Complete)
        );
        assert!(!result.readback.samples.is_empty());
        Ok(())
    }
}
