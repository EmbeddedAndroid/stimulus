use crate::{
    clock::Clock,
    device::{DeviceError, LogicPortDevice},
    readback::{Readback, ReadbackError, read_sdr_windowed},
};
use lp_proto::{
    regs,
    status::{AcqStatus, Phase},
};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcquisitionConfig {
    pub poll_interval: Duration,
    pub start_timeout: Duration,
    pub prefill_timeout: Option<Duration>,
    pub armed_timeout: Option<Duration>,
    pub postfill_timeout: Option<Duration>,
    /// Hard cap on the whole acquisition, independent of the per-phase limits.
    /// `None` per-phase timeouts let a stuck engine spin forever; this backstop
    /// guarantees `acquire_single` always returns (forcing the engine idle and
    /// erroring out) so a non-completing capture can never wedge the daemon by
    /// holding the device lock. All captures are immediate-trigger, which fills
    /// 2,048 samples in bounded time even at the 1 kHz floor (~3 s), so a
    /// generous constant is safe here; a future triggered-wait mode would set
    /// this to `None` deliberately.
    pub overall_timeout: Option<Duration>,
    pub compressed: bool,
    pub trigger_adjustment: i64,
    /// Trigger combine mode re-applied after the pre-arm RESET. RESET clears the
    /// COMBINE control register back to immediate, so an armed edge/pattern
    /// trigger needs its combine value restored between RESET and ARM or it
    /// fires immediately. 0 = immediate (the default).
    pub combine: u8,
    /// Pre/post sample split re-applied after the pre-arm RESET, which resets
    /// POST_COUNT to 2047 (collapsing the window to a single sample). Both 0 =
    /// leave the device's counts as-is (the default, for immediate captures).
    pub pre_count: u16,
    pub post_count: u16,
}
impl Default for AcquisitionConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(20),
            start_timeout: Duration::from_secs(1),
            prefill_timeout: None,
            armed_timeout: None,
            postfill_timeout: None,
            overall_timeout: Some(Duration::from_secs(10)),
            compressed: false,
            trigger_adjustment: 1,
            combine: 0,
            pre_count: 0,
            post_count: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseEvent {
    pub phase: Phase,
    pub status: u8,
    pub at: Duration,
    pub forced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquisitionResult {
    pub timeline: Vec<PhaseEvent>,
    pub readback: Readback,
}

pub fn acquire_single<C: Clock>(
    device: &mut dyn LogicPortDevice,
    clock: &mut C,
    config: AcquisitionConfig,
) -> Result<AcquisitionResult, AcquisitionError> {
    // Begin every acquisition from a clean host-side readback queue: discard any
    // IN transfers the previous capture's read-ahead pool over-read and left
    // queued. Without this the next readback pops those stale bytes first and
    // desyncs FT245/FPGA command framing -- the root cause of the sustained-load
    // degradation (fast 502s after tens of back-to-back captures). Prevention,
    // not recovery; a no-op for backends (sim, tests) without a read-ahead pool.
    device.flush_input();
    // Begin every acquisition from a clean engine state, matching the vendor's
    // disarm+reset-before-arm sequence (w8 0x100003=0; w8 0x100002=1). Relying
    // only on the previous capture's end-of-run RESET is timing-fragile for
    // back-to-back captures: if the next ARM races that reset, the engine keeps
    // its 2047 post-count and the capture comes back empty (window collapses to
    // n = 2048-2047 = 1). Resetting here makes each capture deterministic.
    device.write(regs::ctrl::ARM, &[0])?;
    device.write(regs::ctrl::RESET, &[1])?;
    // RESET clears the COMBINE control register back to immediate, so an armed
    // edge/pattern trigger must have its combine mode restored between RESET and
    // ARM; otherwise the engine triggers immediately instead of waiting.
    device.write(regs::ctrl::COMBINE, &[config.combine])?;
    // RESET also collapses the pre/post window (POST_COUNT -> 2047 = 1 sample);
    // restore the configured split so a triggered capture fills its buffer.
    if config.pre_count != 0 || config.post_count != 0 {
        device.write(regs::ctrl::PRE_COUNT, &config.pre_count.to_le_bytes())?;
        device.write(regs::ctrl::POST_COUNT, &config.post_count.to_le_bytes())?;
    }
    device.write(regs::ctrl::ARM, &[1])?;
    // Trigger Immediate can fill all 2,048 samples before the first USB
    // status round-trip (about 205 us at the default 10 MHz rate).  The
    // vendor treats bit0-clear as data-ready even when it never observed the
    // active state; requiring a witnessed transition turns a valid capture
    // into a false start timeout.
    let first_status = device.read8(regs::ctrl::STATUS)?;
    let mut timeline = Vec::new();
    let mut previous = None;
    let acquisition_started = clock.elapsed();
    let mut phase_started = acquisition_started;
    let mut pending_status = Some(first_status);
    // An armed trigger reports 0x00 for one poll before it settles to 0x52
    // (armed) -- and 0x00 decodes as Complete. Immediate captures accept the
    // first Complete as data-ready; a triggered capture (combine != 0) must
    // first witness the engine active so it does not finish on that 0x00.
    let mut witnessed_active = config.combine == 0;
    // Set when the capture is finalized by the write-pointer stall rather than by
    // reaching Complete. Such a capture holds a full ring with no stale slots, so
    // its readback must use the windowed [0..wr] read, not the planned window.
    let mut stalled = false;
    let mut last_wr = 0u16;
    let mut last_wr_change = clock.elapsed();
    loop {
        let status = match pending_status.take() {
            Some(status) => status,
            None => device.read8(regs::ctrl::STATUS)?,
        };
        let phase = AcqStatus(status).phase();
        if AcqStatus(status).acquiring() {
            witnessed_active = true;
        }
        if previous != Some(phase) {
            phase_started = clock.elapsed();
            timeline.push(PhaseEvent {
                phase,
                status,
                at: phase_started,
                forced: false,
            });
            previous = Some(phase);
        }
        if phase == Phase::Complete && witnessed_active {
            break;
        }
        // A capture can hold at postfill (0x73) instead of returning to Complete
        // (0x50): a triggered capture always does, and an immediate capture does
        // at high sample rates when a fast input keeps the engine from ever
        // signalling Complete. In both cases the ring keeps filling until it is
        // full and the write pointer stalls; the samples are real, so read them
        // back once the pointer holds steady rather than waiting out the overall
        // timeout. The readback mode still follows the trigger (windowed for a
        // triggered capture, planned for an immediate one).
        if witnessed_active && phase == Phase::Postfill {
            let wr = device.read16(regs::ram::WR_PTR)?.min(2047);
            if wr != last_wr {
                last_wr = wr;
                last_wr_change = clock.elapsed();
            }
            if wr > 4
                && clock.elapsed().saturating_sub(last_wr_change) >= Duration::from_millis(400)
            {
                stalled = true;
                break;
            }
        }
        if let Some(limit) = phase_timeout(phase, config)
            && clock.elapsed().saturating_sub(phase_started) >= limit
        {
            force_phase(device, phase)?;
            if let Some(event) = timeline.last_mut() {
                event.forced = true;
            }
        }
        // Backstop: no per-phase limit is required to be set, so an engine that
        // never reaches Complete would otherwise loop forever holding the device
        // lock. Once the overall budget is spent, force the current phase idle
        // and error out so the caller drops the lock and the daemon stays live.
        if let Some(limit) = config.overall_timeout
            && clock.elapsed().saturating_sub(acquisition_started) >= limit
        {
            let _ = force_phase(device, phase);
            let _ = device.write(regs::ctrl::ARM, &[0]);
            let _ = device.write(regs::ctrl::RESET, &[1]);
            return Err(AcquisitionError::OverallTimeout {
                waited: limit,
                status,
            });
        }
        clock.sleep(config.poll_interval);
    }
    let readback = read_sdr_windowed(
        device,
        config.compressed,
        config.trigger_adjustment,
        config.combine != 0 || stalled,
    )?;
    device.write(regs::ctrl::ARM, &[0])?;
    device.write(regs::ctrl::RESET, &[1])?;
    Ok(AcquisitionResult { timeline, readback })
}

pub fn halt<C: Clock>(
    device: &mut dyn LogicPortDevice,
    clock: &mut C,
    config: AcquisitionConfig,
) -> Result<Phase, AcquisitionError> {
    let status = device.read8(regs::ctrl::STATUS)?;
    let phase = AcqStatus(status).phase();
    if phase != Phase::Complete {
        force_phase(device, phase)?;
        let deadline = clock.elapsed().saturating_add(config.start_timeout);
        loop {
            let status = device.read8(regs::ctrl::STATUS)?;
            if AcqStatus(status).phase() == Phase::Complete {
                break;
            }
            if clock.elapsed() >= deadline {
                return Err(AcquisitionError::HaltTimeout {
                    waited: config.start_timeout,
                    status,
                });
            }
            clock.sleep(config.poll_interval);
        }
    }
    device.write(regs::ctrl::ARM, &[0])?;
    device.write(regs::ctrl::RESET, &[1])?;
    Ok(phase)
}

pub fn trigger_immediate(device: &mut dyn LogicPortDevice) -> Result<Phase, AcquisitionError> {
    let status = device.read8(regs::ctrl::STATUS)?;
    let phase = AcqStatus(status).phase();
    match phase {
        Phase::Prefill => device.write(regs::ctrl::FORCE_FROM_PREFILL, &[1])?,
        Phase::Armed => device.write(regs::ctrl::FORCE_FROM_ARMED, &[1])?,
        Phase::Postfill | Phase::Complete => return Err(AcquisitionError::CannotTrigger(phase)),
    }
    Ok(phase)
}

fn phase_timeout(phase: Phase, config: AcquisitionConfig) -> Option<Duration> {
    match phase {
        Phase::Prefill => config.prefill_timeout,
        Phase::Armed => config.armed_timeout,
        Phase::Postfill => config.postfill_timeout,
        Phase::Complete => None,
    }
}

fn force_phase(device: &mut dyn LogicPortDevice, phase: Phase) -> Result<(), AcquisitionError> {
    let addr = match phase {
        Phase::Prefill => regs::ctrl::FORCE_FROM_PREFILL,
        Phase::Armed => regs::ctrl::FORCE_FROM_ARMED,
        Phase::Postfill => regs::ctrl::FORCE_STOP_POSTFILL,
        Phase::Complete => return Ok(()),
    };
    device.write(addr, &[1])?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum AcquisitionError {
    #[error(transparent)]
    Device(#[from] DeviceError),
    #[error(transparent)]
    Readback(#[from] ReadbackError),
    #[error("trigger immediate is invalid during {0:?}")]
    CannotTrigger(Phase),
    #[error("capture construction failed: {0}")]
    Capture(String),
    #[error("setup failed: {0}")]
    Setup(String),
    #[error("acquisition did not leave idle within {waited:?} (last status 0x{status:02x})")]
    StartTimeout { waited: Duration, status: u8 },
    #[error("acquisition halt did not complete within {waited:?} (last status 0x{status:02x})")]
    HaltTimeout { waited: Duration, status: u8 },
    #[error("acquisition did not complete within {waited:?} (last status 0x{status:02x})")]
    OverallTimeout { waited: Duration, status: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        clock::VirtualClock, device::Configured, fpga::ConfigureOutcome, link::DevStats,
        transcript::DeviceIdentity,
    };
    use lp_proto::addr::Addr;
    use std::collections::VecDeque;

    struct HaltDevice {
        statuses: VecDeque<u8>,
        writes: Vec<(Addr, Vec<u8>)>,
    }

    impl LogicPortDevice for HaltDevice {
        fn read(&mut self, addr: Addr, _len: u16) -> Result<Vec<u8>, DeviceError> {
            if addr == regs::ctrl::STATUS {
                return Ok(vec![self.statuses.pop_front().unwrap_or(0)]);
            }
            Ok(vec![0])
        }

        fn write(&mut self, addr: Addr, data: &[u8]) -> Result<(), DeviceError> {
            self.writes.push((addr, data.to_vec()));
            Ok(())
        }

        fn pins(&mut self) -> Result<u8, DeviceError> {
            Ok(0xf8)
        }

        fn configure_fpga(
            &mut self,
            _image: &[u8],
            idx: u8,
            _force: bool,
        ) -> Result<ConfigureOutcome, DeviceError> {
            Ok(ConfigureOutcome {
                warm: true,
                id: idx | 0x10,
                version: 0,
                elapsed: Duration::ZERO,
                drained_bytes: 0,
            })
        }

        fn probe_configured(&mut self) -> Result<Configured, DeviceError> {
            Ok(Configured {
                pins: 0xf8,
                image_id: 0x17,
                version: 0,
                configured: true,
            })
        }

        fn recover(&mut self) -> Result<(), DeviceError> {
            Ok(())
        }

        fn stats(&self) -> DevStats {
            DevStats::default()
        }

        fn identity(&self) -> DeviceIdentity {
            DeviceIdentity {
                vid: 0x0403,
                pid: 0xdc48,
                serial: "test".into(),
                bcd_device: 0x0400,
            }
        }
    }

    /// A device whose acquisition engine never leaves the acquiring state:
    /// STATUS always reads 0x01 (bit0 set, prefill not done), so the phase is
    /// forever `Prefill` and `Phase::Complete` is never observed.
    struct StuckDevice {
        writes: Vec<(Addr, Vec<u8>)>,
    }

    impl LogicPortDevice for StuckDevice {
        fn read(&mut self, addr: Addr, _len: u16) -> Result<Vec<u8>, DeviceError> {
            if addr == regs::ctrl::STATUS {
                return Ok(vec![0x01]);
            }
            Ok(vec![0])
        }
        fn write(&mut self, addr: Addr, data: &[u8]) -> Result<(), DeviceError> {
            self.writes.push((addr, data.to_vec()));
            Ok(())
        }
        fn pins(&mut self) -> Result<u8, DeviceError> {
            Ok(0xf8)
        }
        fn configure_fpga(
            &mut self,
            _image: &[u8],
            idx: u8,
            _force: bool,
        ) -> Result<ConfigureOutcome, DeviceError> {
            Ok(ConfigureOutcome {
                warm: true,
                id: idx | 0x10,
                version: 0,
                elapsed: Duration::ZERO,
                drained_bytes: 0,
            })
        }
        fn probe_configured(&mut self) -> Result<Configured, DeviceError> {
            Ok(Configured {
                pins: 0xf8,
                image_id: 0x17,
                version: 0,
                configured: true,
            })
        }
        fn recover(&mut self) -> Result<(), DeviceError> {
            Ok(())
        }
        fn stats(&self) -> DevStats {
            DevStats::default()
        }
        fn identity(&self) -> DeviceIdentity {
            DeviceIdentity {
                vid: 0x0403,
                pid: 0xdc48,
                serial: "stuck".into(),
                bcd_device: 0x0400,
            }
        }
    }

    // Regression: a capture whose engine never completes must return an error
    // within the overall budget rather than spin forever holding the device
    // lock (which previously wedged the whole daemon; see acquire_single). The
    // default per-phase timeouts are None, so only the overall backstop can end
    // the loop here.
    #[test]
    fn acquire_times_out_instead_of_wedging_when_engine_never_completes() {
        let mut device = StuckDevice { writes: Vec::new() };
        let mut clock = VirtualClock::default();
        let result = acquire_single(
            &mut device,
            &mut clock,
            AcquisitionConfig {
                poll_interval: Duration::from_millis(1),
                overall_timeout: Some(Duration::from_millis(50)),
                ..AcquisitionConfig::default()
            },
        );
        match result {
            Err(AcquisitionError::OverallTimeout { waited, status }) => {
                assert_eq!(waited, Duration::from_millis(50));
                assert_eq!(status, 0x01);
            }
            Ok(_) => panic!("a never-completing engine must error, not hang"),
            Err(other) => panic!("expected OverallTimeout, got {other:?}"),
        }
        // The engine is left idle so the next capture starts clean.
        assert!(device.writes.contains(&(regs::ctrl::ARM, vec![0])));
        assert!(device.writes.contains(&(regs::ctrl::RESET, vec![1])));
    }

    // Holds at Postfill (0x73) with a fixed write pointer, modelling an immediate
    // capture that a fast input keeps from ever reaching Complete at a high rate.
    struct StallDevice {
        writes: Vec<(Addr, Vec<u8>)>,
        wr: u16,
    }
    impl LogicPortDevice for StallDevice {
        fn read(&mut self, addr: Addr, len: u16) -> Result<Vec<u8>, DeviceError> {
            if addr == regs::ctrl::STATUS {
                return Ok(vec![0x73]);
            }
            if addr == regs::ram::WR_PTR {
                return Ok(self.wr.to_le_bytes().to_vec());
            }
            Ok(vec![0u8; usize::from(len)])
        }
        fn write(&mut self, addr: Addr, data: &[u8]) -> Result<(), DeviceError> {
            self.writes.push((addr, data.to_vec()));
            Ok(())
        }
        fn pins(&mut self) -> Result<u8, DeviceError> {
            Ok(0xf8)
        }
        fn configure_fpga(
            &mut self,
            _image: &[u8],
            idx: u8,
            _force: bool,
        ) -> Result<ConfigureOutcome, DeviceError> {
            Ok(ConfigureOutcome {
                warm: true,
                id: idx | 0x10,
                version: 0,
                elapsed: Duration::ZERO,
                drained_bytes: 0,
            })
        }
        fn probe_configured(&mut self) -> Result<Configured, DeviceError> {
            Ok(Configured {
                pins: 0xf8,
                image_id: 0x17,
                version: 0,
                configured: true,
            })
        }
        fn recover(&mut self) -> Result<(), DeviceError> {
            Ok(())
        }
        fn stats(&self) -> DevStats {
            DevStats::default()
        }
        fn identity(&self) -> DeviceIdentity {
            DeviceIdentity {
                vid: 0x0403,
                pid: 0xdc48,
                serial: "stall".into(),
                bcd_device: 0x0400,
            }
        }
    }

    // Regression: an immediate capture (combine == 0) that a fast input pins at
    // Postfill must finalize once its write pointer stalls -- its ring is full,
    // so the samples are real -- instead of spinning out the overall timeout.
    // The stalled readback must be windowed ([0..wr]), not the planned window.
    #[test]
    fn immediate_capture_stalled_at_postfill_finalizes_windowed() {
        let mut device = StallDevice {
            writes: Vec::new(),
            wr: 1500,
        };
        let mut clock = VirtualClock::default();
        let result = acquire_single(
            &mut device,
            &mut clock,
            AcquisitionConfig {
                poll_interval: Duration::from_millis(10),
                overall_timeout: Some(Duration::from_secs(5)),
                combine: 0,
                ..AcquisitionConfig::default()
            },
        );
        match result {
            Ok(outcome) => assert_eq!(
                outcome.readback.window.n, 1501,
                "a stalled capture reads [0..wr] (wr + 1), not the planned window"
            ),
            Err(other) => panic!("stalled immediate capture must finalize, got {other:?}"),
        }
    }

    #[test]
    fn halt_forces_current_phase_then_waits_disarms_and_resets() -> Result<(), AcquisitionError> {
        let mut device = HaltDevice {
            statuses: VecDeque::from([0x41, 0x41, 0]),
            writes: Vec::new(),
        };
        let mut clock = VirtualClock::default();
        let phase = halt(
            &mut device,
            &mut clock,
            AcquisitionConfig {
                poll_interval: Duration::from_millis(1),
                start_timeout: Duration::from_millis(10),
                ..AcquisitionConfig::default()
            },
        )?;

        assert_eq!(phase, Phase::Armed);
        assert_eq!(
            device.writes,
            [
                (regs::ctrl::FORCE_FROM_ARMED, vec![1]),
                (regs::ctrl::ARM, vec![0]),
                (regs::ctrl::RESET, vec![1]),
            ]
        );
        Ok(())
    }
}
