//! Decode real vendor `.LPF` example captures with the lp-proto protocol
//! decoders and check them against protocol invariants the vendor did NOT store
//! in the file: CAN CRC-15 validity, the ISO 7816-3 ATR TS byte, 1-Wire reset
//! framing + ROM commands, and clean async-serial framing.
//!
//! These captures were produced by the vendor's own LA hardware and software,
//! so a decode that satisfies the *standard's* invariants is independent
//! evidence the decoder is correct: unlike a synthetic encode->decode
//! round-trip, it cannot share a bug with our own encoder. The CAN case is the
//! sharpest: a real hardware CRC-15 will only match our `can_crc15` over the
//! destuffed bits if BOTH our destuffing and our CRC match the standard, and a
//! spurious mid-frame "frame" has only a 2^-15 chance of a valid CRC.
//!
//! No hardware required: the sample data is embedded in the checked-in `.LPF`.

use lp_lpf::{Document, Level, SampleData};
use lp_proto::decode::{
    AsyncSerialConfig, I2cEvent, Iso7816Convention, OneWireEvent, Parity, SpiConfig,
    decode_async_serial, decode_can, decode_i2c, decode_iso7816, decode_onewire, decode_parallel,
    decode_spi,
};
use std::path::PathBuf;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/vendor/examples")
}

fn load(name: &str) -> Document {
    let path = examples_dir().join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    lp_lpf::parse(&bytes).unwrap_or_else(|e| panic!("parse {path:?}: {e:?}"))
}

/// Capture sample rate in Hz from the stored `AcquiredSamplePeriod` seconds.
fn sample_rate_hz(doc: &Document) -> u64 {
    let period: f64 = doc
        .value("AcquiredSamplePeriod")
        .unwrap_or_else(|e| panic!("period record: {e:?}"))
        .parse()
        .unwrap_or_else(|e| panic!("period float: {e}"));
    (1.0 / period).round() as u64
}

/// Expand one channel of the RLE sample data into a per-sample level (High =
/// true), clamping any single run to `max_run` samples. Clamping only ever
/// SHORTENS a constant-level stretch, so it collapses multi-million-sample idle
/// gaps in the huge captures while preserving every real bit/pulse (whose runs
/// are far shorter than the clamp). Unknown expands as `false`.
fn expand_channel(data: &SampleData, channel: usize, max_run: usize) -> Vec<bool> {
    let mut out = Vec::new();
    for run in &data.runs {
        let level = matches!(run.channels[channel], Level::High);
        let n = usize::try_from(run.count)
            .unwrap_or(usize::MAX)
            .min(max_run);
        out.resize(out.len() + n, level);
    }
    out
}

/// Expand several channels at once from the *shared* global RLE runs, clamping
/// each run uniformly so every returned channel stays sample-aligned in time.
/// (Clamping channels independently would drop different sample counts per
/// channel and shear their timelines apart, which breaks any two-wire decode.)
fn expand_aligned(data: &SampleData, channels: &[usize], max_run: usize) -> Vec<Vec<bool>> {
    let mut out: Vec<Vec<bool>> = vec![Vec::new(); channels.len()];
    for run in &data.runs {
        let n = usize::try_from(run.count)
            .unwrap_or(usize::MAX)
            .min(max_run);
        for (slot, &ch) in out.iter_mut().zip(channels) {
            let level = matches!(run.channels[ch], Level::High);
            slot.resize(slot.len() + n, level);
        }
    }
    out
}

/// Smallest *merged* constant-level stretch (in samples) on a channel: one bit
/// / etu time. The LA1034 compression stores each logical run as a 1-sample
/// "data slot" plus a "run-length slot", so `lp-lpf` emits two adjacent
/// same-level `SampleRun`s; per-sample expansion concatenates them correctly,
/// but individual run counts underestimate a pulse. Merge adjacent same-level
/// runs here, ignoring the giant lead-in/out idle stretches by construction
/// (they are large, so they never win the min).
fn merged_runs(data: &SampleData, channel: usize) -> Vec<(bool, u64)> {
    let mut out: Vec<(bool, u64)> = Vec::new();
    let mut level: Option<bool> = None;
    for run in &data.runs {
        match run.channels[channel] {
            Level::Unknown => level = None,
            lvl => {
                let hi = matches!(lvl, Level::High);
                if level == Some(hi) {
                    match out.last_mut() {
                        Some(last) => last.1 += run.count,
                        None => unreachable!("level set implies a run exists"),
                    }
                } else {
                    out.push((hi, run.count));
                    level = Some(hi);
                }
            }
        }
    }
    out
}

/// The ISO 7816-3 initial etu (in samples), measured from the ATR's start bit.
/// The line idles high; the first low run after that long idle is the TS start
/// bit, and because the first data bit of both valid TS values (0x3B direct,
/// 0x3F inverse) is high, that start bit is an isolated one-etu low. Measuring
/// it here (rather than the global shortest recurring pulse) is what lets us
/// decode a card that speeds up its etu after the ATR (PPS): the fast phase's
/// shorter bits must not be mistaken for the initial etu.
fn iso_initial_etu(data: &SampleData, channel: usize) -> u64 {
    let m = merged_runs(data, channel);
    for i in 1..m.len() {
        let (lvl, len) = m[i];
        let (plvl, plen) = m[i - 1];
        if !lvl && plvl && len > 0 && plen >= 4 * len {
            return len;
        }
    }
    panic!("no ATR start bit (idle-high -> start-low) found");
}

/// One bit/etu time in samples: the smallest *recurring* merged-run width. A
/// single logic bit yields a merged run of one bit time, and it recurs many
/// times; a lone shorter run is a capture glitch (the SIM/smart-card and
/// return-line captures each carry one), so we take the smallest run width that
/// has at least a few near-equal siblings (within +/-12% for jitter) rather
/// than the absolute minimum, then return that cluster's median.
fn min_bit_samples(data: &SampleData, channel: usize) -> u64 {
    let mut runs: Vec<u64> = merged_runs(data, channel)
        .into_iter()
        .map(|(_, len)| len)
        .collect();
    assert!(!runs.is_empty(), "channel has no runs");
    runs.sort_unstable();
    for &r in &runs {
        let lo = r - r / 8; // ~-12%
        let hi = r + r / 8; // ~+12%
        let mut cluster: Vec<u64> = runs
            .iter()
            .copied()
            .filter(|x| *x >= lo && *x <= hi)
            .collect();
        if cluster.len() >= 3 {
            cluster.sort_unstable();
            return cluster[cluster.len() / 2];
        }
    }
    runs[0]
}

/// Snap a raw detected line rate to the nearest common async baud.
fn nearest_baud(raw: f64) -> u32 {
    const STD: [u32; 9] = [
        1_200, 2_400, 4_800, 9_600, 19_200, 38_400, 57_600, 115_200, 230_400,
    ];
    STD.iter()
        .min_by(|a, b| {
            (raw - f64::from(**a))
                .abs()
                .total_cmp(&(raw - f64::from(**b)).abs())
        })
        .copied()
        .unwrap_or_else(|| unreachable!("STD is non-empty"))
}

/// Snap a raw detected bit-rate to the nearest standard CAN bit-rate.
fn nearest_can_bitrate(raw: f64) -> u32 {
    const STD: [u32; 8] = [
        10_000, 20_000, 50_000, 125_000, 250_000, 500_000, 800_000, 1_000_000,
    ];
    STD.iter()
        .min_by(|a, b| {
            (raw - f64::from(**a))
                .abs()
                .total_cmp(&(raw - f64::from(**b)).abs())
        })
        .copied()
        .unwrap_or_else(|| unreachable!("STD is non-empty"))
}

// --- CAN: every real frame's hardware CRC-15 must match our destuffed CRC -----

#[test]
fn golden_can_multiple_frame_crc15() {
    let doc = load("A. CAN Interpreter - Multiple Frame.LPF");
    let rate = sample_rate_hz(&doc);
    let ch = 8;
    let bit_samples = min_bit_samples(&doc.sample_data, ch);
    let bitrate = nearest_can_bitrate(rate as f64 / bit_samples as f64);
    let spb = (rate / u64::from(bitrate)) as usize;
    let levels = expand_channel(&doc.sample_data, ch, spb * 24);

    let frames = decode_can(&levels, rate, bitrate);
    let valid: Vec<_> = frames.iter().filter(|f| f.crc_ok).collect();
    eprintln!(
        "CAN(A): rate={rate} bitrate={bitrate} spb={spb} frames={} crc_ok={}",
        frames.len(),
        valid.len()
    );
    for f in valid.iter().take(8) {
        eprintln!(
            "  id=0x{:03x} dlc={} data={:02x?} crc=0x{:04x}",
            f.id, f.dlc, f.data, f.crc
        );
    }
    // Real captured CAN traffic: several frames, each with a hardware CRC-15
    // that matches our destuffed can_crc15. This is the independent check.
    assert!(
        valid.len() >= 3,
        "expected >=3 CRC-valid CAN frames, got {}",
        valid.len()
    );
}

// --- 1-Wire: a reset then a standard ROM command byte -------------------------

#[test]
fn golden_onewire_reset_and_rom_command() {
    let doc = load("D. 1-Wire Interpreter.LPF");
    let rate = sample_rate_hz(&doc);
    let levels = expand_channel(&doc.sample_data, 0, usize::MAX);
    let events = decode_onewire(&levels, rate);

    let resets = events
        .iter()
        .filter(|e| matches!(e, OneWireEvent::Reset))
        .count();
    // First data byte following the first reset.
    let mut after_reset = events
        .iter()
        .skip_while(|e| !matches!(e, OneWireEvent::Reset))
        .filter_map(|e| match e {
            OneWireEvent::Byte(b) => Some(*b),
            OneWireEvent::Reset => None,
        });
    let first = after_reset.next();
    eprintln!("1-Wire(D): rate={rate} resets={resets} first_rom=0x{first:02x?}");
    // Standard ROM commands (Read/Skip/Match/Search/Overdrive-Skip/Alarm).
    const ROM: [u8; 6] = [0x33, 0xCC, 0x55, 0xF0, 0x3C, 0xEC];
    assert!(resets >= 1, "expected >=1 reset");
    let Some(first) = first else {
        panic!("expected a data byte after the first reset");
    };
    assert!(
        ROM.contains(&first),
        "first byte after reset 0x{first:02x} is not a ROM command"
    );
}

// --- ISO 7816-3: the ATR's first byte is TS = 0x3B (direct) or 0x3F (inverse) -

fn assert_iso7816_ts(name: &str, channel: usize) {
    let doc = load(name);
    let rate = sample_rate_hz(&doc);
    // 1 etu measured from the ATR start bit (robust to a post-ATR speed change).
    let etu_baud = (rate / iso_initial_etu(&doc.sample_data, channel)).max(1) as u32;
    let levels = expand_channel(&doc.sample_data, channel, usize::MAX);

    let direct = decode_iso7816(&levels, rate, etu_baud, Iso7816Convention::Direct);
    let inverse = decode_iso7816(&levels, rate, etu_baud, Iso7816Convention::Inverse);
    let ts_direct = direct.first().map(|b| b.value);
    let ts_inverse = inverse.first().map(|b| b.value);
    eprintln!(
        "ISO7816({name}): rate={rate} etu_baud={etu_baud} ts_direct={ts_direct:02x?} ts_inverse={ts_inverse:02x?}"
    );
    // Exactly one convention yields a valid TS as the first ATR character, and
    // in that convention the leading ATR characters must be parity-clean (the
    // decoder checks even parity in the direct sense, odd in the sampled
    // inverse sense). A wrong convention or etu would mis-frame and error.
    let matched = if ts_direct == Some(0x3B) {
        direct
    } else if ts_inverse == Some(0x3F) {
        inverse
    } else {
        panic!("neither convention produced a valid ATR TS byte (0x3B/0x3F)");
    };
    let head = matched.iter().take(4).collect::<Vec<_>>();
    let errs = head.iter().filter(|b| b.error.is_some()).count();
    assert_eq!(
        errs, 0,
        "leading ATR characters should be parity/framing clean"
    );
}

#[test]
fn golden_iso7816_sim_card_ts() {
    assert_iso7816_ts("E. ISO7815-3 Interpreter - SIM Card.LPF", 0);
}

#[test]
fn golden_iso7816_smart_card_ts() {
    assert_iso7816_ts("F. ISO7815-3 Interpreter - Smart Card.LPF", 0);
}

// --- Async serial (RS-232): a real capture decodes with clean framing ---------

#[test]
fn golden_async_serial_cnc() {
    let doc = load("6. CNC Serial Port Compression.LPF");
    let rate = sample_rate_hz(&doc);
    // Channel 2 carries the CNC controller's serial output. The line is probed
    // at true RS-232 levels (idle marking = negative = logic low here), and the
    // controller frames 7 data bits + even parity + 1 stop at 9600 baud. Detect
    // the baud from the shortest recurring pulse and decode with that framing.
    let ch = 2usize;
    let bit_samples = min_bit_samples(&doc.sample_data, ch);
    let baud = nearest_baud(rate as f64 / bit_samples as f64);
    let spb = (rate / u64::from(baud)) as usize;
    let levels = expand_channel(&doc.sample_data, ch, spb * 24);
    let cfg = AsyncSerialConfig {
        sample_rate_hz: rate,
        baud,
        data_bits: 7,
        parity: Parity::Even,
        stop_bits: 1,
        idle_high: false,
        lsb_first: true,
    };
    let bytes = decode_async_serial(&levels, &cfg);
    let framing_errs = bytes.iter().filter(|b| b.error.is_some()).count();
    let text: String = bytes
        .iter()
        .map(|b| {
            let v = b.value as u8;
            if v.is_ascii_graphic() || v == b' ' {
                v as char
            } else {
                '.'
            }
        })
        .collect();
    let printable = bytes
        .iter()
        .filter(|b| {
            let v = b.value as u8;
            v.is_ascii_graphic() || v == b' ' || v == b'\r' || v == b'\n' || v == b'\t'
        })
        .count();
    eprintln!(
        "RS232(6) ch{ch}: rate={rate} baud={baud} 7E1 inverted bytes={n} framing_errs={framing_errs} printable={printable} :: {text:?}",
        n = bytes.len()
    );
    // A real 7E1 capture: a full byte stream, zero framing/parity errors, and
    // essentially all printable ASCII (the CNC controller emits G-code text).
    assert!(
        bytes.len() >= 32,
        "expected a real byte stream, got {}",
        bytes.len()
    );
    assert_eq!(
        framing_errs, 0,
        "7E1 decode should have no framing/parity errors"
    );
    assert!(
        printable * 20 >= bytes.len() * 19,
        "expected >=95% printable ASCII, got {printable}/{}",
        bytes.len()
    );
}

// --- I2C: a real two-wire capture yields well-formed START/data/ACK/STOP ------

#[test]
fn golden_i2c_file7() {
    let doc = load("7. I2C, SPI, RS232 Interpreters.LPF");
    // Named channels in this capture: SDA = 0, SCL = 1.
    let chans = expand_aligned(&doc.sample_data, &[1usize, 0usize], 8192);
    let (scl, sda) = (&chans[0], &chans[1]);
    let events = decode_i2c(scl, sda);

    let starts = events
        .iter()
        .filter(|e| matches!(e, I2cEvent::Start))
        .count();
    let stops = events
        .iter()
        .filter(|e| matches!(e, I2cEvent::Stop))
        .count();
    let bytes: Vec<(u8, bool)> = events
        .iter()
        .filter_map(|e| match e {
            I2cEvent::Byte { value, ack } => Some((*value, *ack)),
            _ => None,
        })
        .collect();
    eprintln!(
        "I2C(7): starts={starts} stops={stops} bytes={} first={:02x?}",
        bytes.len(),
        bytes.iter().take(8).collect::<Vec<_>>()
    );

    // The event stream must be well-formed: it opens with a START, every START
    // is eventually closed by a STOP (transactions nest/close), and no data or
    // STOP appears before the first START.
    assert!(
        starts >= 1 && stops >= 1,
        "expected at least one START/STOP pair"
    );
    assert!(
        matches!(events.first(), Some(I2cEvent::Start)),
        "stream must open with START"
    );
    let mut active = false;
    let mut seen_first_start = false;
    for e in &events {
        match e {
            I2cEvent::Start => {
                active = true;
                seen_first_start = true;
            }
            I2cEvent::Stop => {
                assert!(active, "STOP without an open transaction");
                active = false;
            }
            I2cEvent::Byte { .. } => assert!(seen_first_start, "data byte before first START"),
        }
    }
    // Real bus traffic to present devices: the first byte after a START is the
    // address+R/W, and at least one addressed device ACKs.
    assert!(bytes.len() >= 2, "expected addressed data bytes");
    let acked = bytes.iter().filter(|(_, ack)| *ack).count();
    assert!(
        acked >= 1,
        "expected at least one ACKed byte from a real device"
    );
}

// --- SPI: a real SPI-EEPROM READ decodes its opcode and returned contents -----

#[test]
fn golden_spi_file7() {
    let doc = load("7. I2C, SPI, RS232 Interpreters.LPF");
    // Named channels: SS = 2, SCK = 3, MOSI = 4, MISO = 5.
    let ch = expand_aligned(&doc.sample_data, &[3usize, 4, 5], 4096);
    let (sck, mosi, miso) = (&ch[0], &ch[1], &ch[2]);
    let cfg = SpiConfig::mode0_8bit(); // this bus idles clock low, samples first edge
    let mo = decode_spi(sck, mosi, &cfg);
    let mi = decode_spi(sck, miso, &cfg);
    eprintln!("SPI(7) mode0: MOSI={mo:02x?} MISO={mi:02x?}");
    // The master issues the standard SPI-EEPROM READ opcode (0x03) on MOSI, and
    // the device returns the same sequential contents that the I2C capture read
    // back (00 01 02 03 04). A wrong SPI mode would shift every bit and destroy
    // both, so decoding them correctly reproduces the vendor frame.
    assert!(mo.len() >= 7, "expected the full SPI transaction");
    assert_eq!(mo[0], 0x03, "first MOSI byte should be the READ opcode");
    let miso_tail: Vec<u16> = mi.iter().rev().take(5).rev().copied().collect();
    assert_eq!(
        miso_tail,
        vec![0x00, 0x01, 0x02, 0x03, 0x04],
        "MISO should return the EEPROM data"
    );
}

// --- Quad-SPI / parallel: 4 IO lines clock out an incrementing nibble ramp ----

#[test]
fn golden_quad_spi_file_c() {
    let doc = load("C. Quad SPI Interface.LPF");
    // Named channels: CLK = 11, IO0..IO3 = 12..15.
    let ch = expand_aligned(&doc.sample_data, &[11usize, 12, 13, 14, 15], 4096);
    let clk = &ch[0];
    let data: Vec<&[bool]> = vec![&ch[1], &ch[2], &ch[3], &ch[4]];
    let words = decode_parallel(clk, &data, true); // sample IO0..3 on the rising edge
    eprintln!(
        "QuadSPI(C): n={} head={:x?}",
        words.len(),
        words.iter().take(16).collect::<Vec<_>>()
    );
    // The stimulus drives IO0..3 as a 4-bit counter, so the parallel decode is a
    // clean 0,1,2,...,15 ramp: exact, unambiguous ground truth for the decoder.
    assert!(words.len() >= 16, "expected a run of parallel words");
    let ramp: Vec<u32> = (0..16).collect();
    assert_eq!(words[..16], ramp[..], "quad-SPI nibbles should count 0..15");
}
