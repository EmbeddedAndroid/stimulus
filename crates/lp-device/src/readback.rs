use crate::device::{DeviceError, LogicPortDevice};
use lp_proto::{
    ProtoError,
    addr::Addr,
    readback::{Blocks, Window, adjust_for_run_probe, plan_window, trigger_index},
    regs,
    slot::{Sample, Slot, decode_slot, slots_to_samples},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readback {
    pub window: Window,
    pub slots: Vec<Slot>,
    pub samples: Vec<Sample>,
    pub trigger_slot: usize,
    pub trigger_sample: u64,
}

pub fn read_sdr(
    device: &mut dyn LogicPortDevice,
    compressed: bool,
    trigger_adjustment: i64,
) -> Result<Readback, ReadbackError> {
    read_sdr_windowed(device, compressed, trigger_adjustment, false)
}

/// `triggered` reads back a capture that stopped before the ring wrapped: the
/// valid slots are `[0..WR_PTR]`, so the window is taken from page 0 up to the
/// write pointer instead of the full 2048-slot ring (which would include stale
/// slots left over from before the capture).
pub fn read_sdr_windowed(
    device: &mut dyn LogicPortDevice,
    compressed: bool,
    trigger_adjustment: i64,
    triggered: bool,
) -> Result<Readback, ReadbackError> {
    let wr = device.read16(regs::ram::WR_PTR)?.min(2047);
    let post_plus_one = if compressed {
        0
    } else {
        device.read16(regs::ctrl::POST_COUNT_RD)?.min(2047)
    };
    let mut window = if triggered {
        Window {
            page0: 0,
            n: wr.saturating_add(1),
        }
    } else {
        plan_window(wr, post_plus_one, compressed)
    };
    if window.n == 0 {
        return Err(ReadbackError::EmptyWindow);
    }
    let probe = read_ring(device, regs::ram::flags, window.page0, 1)?[0];
    window = adjust_for_run_probe(window, probe);
    if window.n == 0 {
        return Err(ReadbackError::EmptyWindow);
    }
    let blocks = Blocks {
        b: [
            read_block(device, 1, window)?,
            read_block(device, 2, window)?,
            read_block(device, 3, window)?,
            read_block(device, 4, window)?,
        ],
        flags: read_ring(device, regs::ram::flags, window.page0, window.n)?,
        ddr: None,
    };
    decode(
        blocks,
        window,
        device.read16(regs::cap::TRIG_PAGE)?,
        trigger_adjustment,
    )
}

fn read_block(
    device: &mut dyn LogicPortDevice,
    block: u8,
    window: Window,
) -> Result<Vec<u8>, ReadbackError> {
    read_ring(
        device,
        |page| regs::ram::block(block, page),
        window.page0,
        window.n,
    )
}

fn read_ring<F>(
    device: &mut dyn LogicPortDevice,
    address: F,
    page: u16,
    count: u16,
) -> Result<Vec<u8>, ReadbackError>
where
    F: Fn(u16) -> Result<Addr, ProtoError>,
{
    let first_len = count.min(2048 - page);
    let mut bytes = device.read(address(page)?, first_len)?;
    let remaining = count - first_len;
    if remaining > 0 {
        bytes.extend_from_slice(&device.read(address(0)?, remaining)?);
    }
    if bytes.len() != usize::from(count) {
        return Err(ReadbackError::ShortBlock {
            expected: usize::from(count),
            got: bytes.len(),
        });
    }
    Ok(bytes)
}

fn decode(
    blocks: Blocks,
    window: Window,
    trig_page: u16,
    trigger_adjustment: i64,
) -> Result<Readback, ReadbackError> {
    let raw_slots = (0..usize::from(window.n))
        .map(|index| {
            decode_slot(
                blocks.b[0][index],
                blocks.b[1][index],
                blocks.b[2][index],
                blocks.b[3][index],
                blocks.flags[index],
            )
        })
        .collect::<Vec<_>>();
    // Ring-wrap tolerance: a wrapped compressed ring's oldest slots can be the
    // tail of a run whose DATA anchor was overwritten and now sits before
    // page0. Those leading run samples are unrecoverable (their value is lost to
    // the wrap), so start the capture at the first DATA slot. This is correct
    // handling of the window boundary, not a desync mask -- the remaining slots
    // form a self-consistent capture that slots_to_samples accepts.
    let skip = raw_slots
        .iter()
        .position(|slot| matches!(slot, Slot::Data { .. }))
        .unwrap_or(raw_slots.len());
    let slots = raw_slots.get(skip..).unwrap_or(&[]).to_vec();
    if slots.is_empty() {
        return Err(ReadbackError::EmptyWindow);
    }
    let samples = slots_to_samples(&slots)?;
    let trigger_slot = trigger_index(window.n.saturating_sub(1), trig_page)
        .saturating_sub(skip)
        .min(slots.len() - 1);
    let expanded_before = slots[..trigger_slot]
        .iter()
        .try_fold(0_u64, |total, slot| {
            total
                .checked_add(match slot {
                    Slot::Data { .. } => 1,
                    Slot::Run { count } => *count,
                })
                .ok_or(ReadbackError::LengthOverflow)
        })?;
    let trigger_sample = if trigger_adjustment >= 0 {
        expanded_before.saturating_add(trigger_adjustment as u64)
    } else {
        expanded_before.saturating_sub(trigger_adjustment.unsigned_abs())
    };
    Ok(Readback {
        window,
        slots,
        samples,
        trigger_slot,
        trigger_sample,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ReadbackError {
    #[error(transparent)]
    Device(#[from] DeviceError),
    #[error(transparent)]
    Protocol(#[from] ProtoError),
    #[error("capture readback window is empty")]
    EmptyWindow,
    #[error("capture block was short: expected {expected}, got {got}")]
    ShortBlock { expected: usize, got: usize },
    #[error("expanded capture length overflow")]
    LengthOverflow,
}

#[cfg(test)]
mod tests {
    use super::*;
    use lp_device_test_support::FakeDevice;

    mod lp_device_test_support {
        use crate::{
            device::{Configured, DeviceError, LogicPortDevice},
            fpga::ConfigureOutcome,
            link::DevStats,
            transcript::DeviceIdentity,
        };
        use lp_proto::addr::Addr;
        use std::{collections::BTreeMap, time::Duration};

        #[derive(Default)]
        pub struct FakeDevice {
            bytes: BTreeMap<u32, u8>,
        }
        impl FakeDevice {
            pub fn set(&mut self, addr: Addr, data: &[u8]) {
                for (offset, byte) in data.iter().enumerate() {
                    self.bytes.insert(addr.0 + offset as u32, *byte);
                }
            }
        }
        impl LogicPortDevice for FakeDevice {
            fn read(&mut self, addr: Addr, len: u16) -> Result<Vec<u8>, DeviceError> {
                Ok((0..u32::from(len))
                    .map(|offset| self.bytes.get(&(addr.0 + offset)).copied().unwrap_or(0))
                    .collect())
            }
            fn write(&mut self, addr: Addr, data: &[u8]) -> Result<(), DeviceError> {
                self.set(addr, data);
                Ok(())
            }
            fn pins(&mut self) -> Result<u8, DeviceError> {
                Ok(0xf8)
            }
            fn configure_fpga(
                &mut self,
                _: &[u8],
                idx: u8,
                _: bool,
            ) -> Result<ConfigureOutcome, DeviceError> {
                Ok(ConfigureOutcome {
                    warm: true,
                    id: idx | 0x10,
                    version: 1,
                    elapsed: Duration::ZERO,
                    drained_bytes: 0,
                })
            }
            fn probe_configured(&mut self) -> Result<Configured, DeviceError> {
                Ok(Configured {
                    pins: 0xf8,
                    image_id: 0x17,
                    version: 1,
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
                    serial: "fake".into(),
                    bcd_device: 0x400,
                    vid: 0x403,
                    pid: 0xdc48,
                }
            }
        }
    }

    #[test]
    fn reconstructs_sdr_data_and_run_pages() -> Result<(), Box<dyn std::error::Error>> {
        let mut device = FakeDevice::default();
        device.set(regs::ram::WR_PTR, &3_u16.to_le_bytes());
        device.set(regs::ctrl::POST_COUNT_RD, &2044_u16.to_le_bytes());
        device.set(regs::cap::TRIG_PAGE, &2_u16.to_le_bytes());
        for (block, data) in [
            (1, [0x11, 2, 0x22, 0x33]),
            (2, [0, 0, 0, 0]),
            (3, [0, 0, 0, 0]),
            (4, [0, 0, 0, 0]),
        ] {
            device.set(regs::ram::block(block, 0)?, &data);
        }
        device.set(regs::ram::flags(0)?, &[0, 8, 2, 4]);

        let result = read_sdr(&mut device, false, 1)?;
        assert_eq!(result.window, Window { page0: 0, n: 4 });
        assert_eq!(result.samples.len(), 3);
        assert_eq!(result.samples[0].bits, 0x11);
        assert_eq!(result.samples[0].repeat, 2);
        assert_eq!(result.samples[1].bits, 0x22 | (1 << 32));
        assert_eq!(result.samples[2].bits, 0x33 | (1 << 33));
        assert_eq!(result.trigger_slot, 2);
        assert_eq!(result.trigger_sample, 4);
        Ok(())
    }

    #[test]
    fn ring_reads_wrap_at_page_2048() -> Result<(), Box<dyn std::error::Error>> {
        let mut device = FakeDevice::default();
        device.set(regs::ram::flags(2046)?, &[1, 2]);
        device.set(regs::ram::flags(0)?, &[3, 4]);
        assert_eq!(
            read_ring(&mut device, regs::ram::flags, 2046, 4)?,
            [1, 2, 3, 4]
        );
        Ok(())
    }

    /// Lay a compression-on capture into the fake device the way the hardware
    /// presents one: a full 2048-slot ring (each RLE sample = a DATA page plus,
    /// when it repeats, a 35-bit RUN page), WR_PTR at the newest slot, and a
    /// STALE post-count that the compressed read path must ignore in favour of
    /// walking the whole ring. Returns the number of pages written.
    fn install_compressed_ring(
        device: &mut FakeDevice,
        samples: &[Sample],
    ) -> Result<u16, Box<dyn std::error::Error>> {
        let mut page: u16 = 0;
        for sample in samples {
            let data = (sample.bits & 0xffff_ffff).to_le_bytes();
            for block in 1..=4u8 {
                device.set(
                    regs::ram::block(block, page)?,
                    &[data[usize::from(block - 1)]],
                );
            }
            let clk1 = u8::from((sample.bits >> 32) & 1 == 1);
            let clk2 = u8::from((sample.bits >> 33) & 1 == 1);
            device.set(regs::ram::flags(page)?, &[(clk1 << 1) | (clk2 << 2)]);
            page += 1;
            if sample.repeat > 0 {
                let run = sample.repeat.to_le_bytes();
                for block in 1..=4u8 {
                    device.set(
                        regs::ram::block(block, page)?,
                        &[run[usize::from(block - 1)]],
                    );
                }
                let top = (sample.repeat >> 32) as u8 & 7;
                device.set(regs::ram::flags(page)?, &[8 | top]);
                page += 1;
            }
        }
        device.set(regs::ram::WR_PTR, &(page - 1).to_le_bytes());
        // Post-count is deliberately garbage: compression-on captures leave it
        // stale, which is exactly why the read path takes compressed = true.
        device.set(regs::ctrl::POST_COUNT_RD, &1234_u16.to_le_bytes());
        device.set(regs::cap::TRIG_PAGE, &0_u16.to_le_bytes());
        Ok(page)
    }

    // D4 (compression & depth): a compression-on capture whose expanded length
    // far exceeds the 2048-slot buffer is reconstructed EXACTLY through the
    // production read_sdr(compressed = true) full-ring path. This is the no-HW
    // synthetic equivalent of the vendor's "19.3K samples" 8051 scenario.
    #[test]
    fn compressed_capture_reconstructs_19k_samples_exactly()
    -> Result<(), Box<dyn std::error::Error>> {
        // 1024 runs of four rotating bus values (adjacent values always differ,
        // so nothing merges); run lengths sum to exactly 19,300 samples, and
        // each run has length >= 2 so every sample emits both a DATA and a RUN
        // page -> exactly 2048 pages, a full ring.
        const VALUES: [u64; 4] = [0x0000_00ff, 0x0000_aa55, 0x00ff_00ff, 0x1234_5678];
        const RUNS: usize = 1024;
        const TOTAL: usize = 19_300;
        let base = TOTAL / RUNS; // 18
        let extra = TOTAL % RUNS; // 868 runs get one more sample
        let mut raw: Vec<u64> = Vec::with_capacity(TOTAL);
        for i in 0..RUNS {
            let len = base + usize::from(i < extra);
            raw.extend(std::iter::repeat_n(VALUES[i % VALUES.len()], len));
        }
        assert_eq!(raw.len(), TOTAL);

        // Compress with the production RLE, then lay it into the ring.
        let samples = lp_proto::rle::compress(raw.iter().copied());
        assert_eq!(samples.len(), RUNS, "rotating values must not merge");
        let mut device = FakeDevice::default();
        device.set(regs::cap::TRIG_PAGE, &0_u16.to_le_bytes());
        let pages = install_compressed_ring(&mut device, &samples)?;
        assert_eq!(pages, 2048, "a compression-on capture fills the whole ring");

        // Read it back the way the driver does when compression is on.
        let result = read_sdr(&mut device, true, 1)?;
        assert_eq!(result.window, Window { page0: 0, n: 2048 });
        assert_eq!(result.slots.len(), 2048);
        assert_eq!(result.samples.len(), RUNS);
        // Depth: far more than 2048 raw samples reconstructed.
        assert_eq!(lp_proto::rle::total_len(&result.samples), TOTAL as u64);
        // Exactness: the expanded stream is bit-identical to the stimulus.
        assert_eq!(
            lp_proto::rle::expand(&result.samples).collect::<Vec<_>>(),
            raw
        );
        Ok(())
    }

    // D4: a 35-bit RUN count (a single value held for more than 2^32 samples)
    // survives the readback path intact, proving the wide-count reconstruction
    // the vendor's deep captures depend on.
    #[test]
    fn compressed_capture_reconstructs_35bit_run() -> Result<(), Box<dyn std::error::Error>> {
        // One DATA sample repeated (1<<33)+7 times = a run needing the top
        // flag bits, then a second distinct value so the ring has two slots
        // of real content; pad the rest of the full ring with a benign tail.
        let long = (1_u64 << 33) + 7;
        let samples = vec![
            Sample {
                bits: 0x00ab_cdef,
                repeat: long,
            },
            Sample {
                bits: 0x0055_00aa,
                repeat: 0,
            },
        ];
        let mut device = FakeDevice::default();
        // Only two slots -> not a full ring; use the non-compressed path whose
        // window is exactly the written slot count so we isolate the 35-bit run
        // decode without ring-wrap concerns.
        let mut page: u16 = 0;
        for sample in &samples {
            let data = (sample.bits & 0xffff_ffff).to_le_bytes();
            for block in 1..=4u8 {
                device.set(
                    regs::ram::block(block, page)?,
                    &[data[usize::from(block - 1)]],
                );
            }
            device.set(regs::ram::flags(page)?, &[0]);
            page += 1;
            if sample.repeat > 0 {
                let run = sample.repeat.to_le_bytes();
                for block in 1..=4u8 {
                    device.set(
                        regs::ram::block(block, page)?,
                        &[run[usize::from(block - 1)]],
                    );
                }
                device.set(
                    regs::ram::flags(page)?,
                    &[8 | ((sample.repeat >> 32) as u8 & 7)],
                );
                page += 1;
            }
        }
        device.set(regs::ram::WR_PTR, &(page - 1).to_le_bytes());
        device.set(regs::ctrl::POST_COUNT_RD, &(2048 - page).to_le_bytes());
        device.set(regs::cap::TRIG_PAGE, &0_u16.to_le_bytes());

        let result = read_sdr(&mut device, false, 1)?;
        assert_eq!(result.samples[0].bits, 0x00ab_cdef);
        assert_eq!(result.samples[0].repeat, long);
        assert_eq!(lp_proto::rle::total_len(&result.samples), long + 1 + 1);
        Ok(())
    }
}
