use crate::{
    clock::Clock,
    device::{DeviceError, LogicPortDevice},
    readback::{Readback, ReadbackError, read_sdr},
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
    pub compressed: bool,
    pub trigger_adjustment: i64,
}
impl Default for AcquisitionConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(20),
            start_timeout: Duration::from_secs(1),
            prefill_timeout: None,
            armed_timeout: None,
            postfill_timeout: None,
            compressed: false,
            trigger_adjustment: 1,
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
    device.write(regs::ctrl::ARM, &[1])?;
    // Trigger Immediate can fill all 2,048 samples before the first USB
    // status round-trip (about 205 us at the default 10 MHz rate).  The
    // vendor treats bit0-clear as data-ready even when it never observed the
    // active state; requiring a witnessed transition turns a valid capture
    // into a false start timeout.
    let first_status = device.read8(regs::ctrl::STATUS)?;
    let mut timeline = Vec::new();
    let mut previous = None;
    let mut phase_started = clock.elapsed();
    let mut pending_status = Some(first_status);
    loop {
        let status = match pending_status.take() {
            Some(status) => status,
            None => device.read8(regs::ctrl::STATUS)?,
        };
        let phase = AcqStatus(status).phase();
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
        if phase == Phase::Complete {
            break;
        }
        if let Some(limit) = phase_timeout(phase, config)
            && clock.elapsed().saturating_sub(phase_started) >= limit
        {
            force_phase(device, phase)?;
            if let Some(event) = timeline.last_mut() {
                event.forced = true;
            }
        }
        clock.sleep(config.poll_interval);
    }
    let readback = read_sdr(device, config.compressed, config.trigger_adjustment)?;
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
