//! Protocol interpreters. Decode a single channel's per-sample logic
//! level sequence into protocol frames. No hardware required: the golden tests
//! decode synthesized waveforms (and, as fixtures are wired in, the vendor
//! `.LPF` example captures) and assert exact frame recovery.
//!
//! First interpreter: async serial (RS-232 / UART-style). More follow (SPI,
//! I2C, CAN, 1-Wire, ISO7816-3, parallel) against the same `SampleLevels`
//! input contract.

/// Parity mode for async serial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

/// Async-serial (UART) framing parameters. `sample_rate_hz` is the capture rate;
/// `baud` the line rate; the two set how many samples cover one bit.
#[derive(Debug, Clone, Copy)]
pub struct AsyncSerialConfig {
    pub sample_rate_hz: u64,
    pub baud: u32,
    pub data_bits: u8, // 5..=9
    pub parity: Parity,
    pub stop_bits: u8, // 1 or 2
    /// Standard TTL/CMOS UART idles HIGH (mark=1, start bit is a LOW space).
    pub idle_high: bool,
    /// UART sends the least-significant data bit first.
    pub lsb_first: bool,
}

impl AsyncSerialConfig {
    /// Common 8N1, idle-high, LSB-first configuration at a given rate/baud.
    pub fn uart_8n1(sample_rate_hz: u64, baud: u32) -> Self {
        Self {
            sample_rate_hz,
            baud,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
            idle_high: true,
            lsb_first: true,
        }
    }
    fn samples_per_bit(&self) -> f64 {
        (self.sample_rate_hz as f64 / f64::from(self.baud)).max(1.0)
    }
}

/// A framing/parity fault on a decoded byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    Framing,
    Parity,
}

/// One decoded async-serial byte and where its start bit began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialByte {
    pub start_sample: usize,
    pub value: u16,
    pub error: Option<FrameError>,
}

/// Sample the line level at the centre of bit `bit` (0 = start bit) counting
/// from the start-bit leading edge.
fn bit_center(levels: &[bool], edge: usize, bit: f64, spb: f64) -> Option<bool> {
    let offset = (bit * spb).round() as usize;
    levels.get(edge + offset).copied()
}

/// Decode async-serial bytes from a per-sample logic-level sequence.
///
/// Scans for each start bit (idle -> start transition), samples every bit at its
/// centre, assembles the data word (LSB- or MSB-first), checks parity and the
/// stop bit, then skips to the end of the frame and continues. Robust to a
/// resampled (over-/under-sampled) line as long as there are >= ~4 samples/bit.
pub fn decode_async_serial(levels: &[bool], cfg: &AsyncSerialConfig) -> Vec<SerialByte> {
    let spb = cfg.samples_per_bit();
    let mark = cfg.idle_high; // logic-1 idle level
    let space = !mark; // logic-0 (start bit)
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < levels.len() {
        // A start bit is an idle(mark) -> space edge.
        if !(levels[i] == mark && levels[i + 1] == space) {
            i += 1;
            continue;
        }
        let edge = i + 1;
        // Confirm the start bit is still space at its centre (reject glitches).
        if bit_center(levels, edge, 0.5, spb) != Some(space) {
            i += 1;
            continue;
        }
        let mut value: u16 = 0;
        let mut ones = 0u32;
        let mut truncated = false;
        for b in 0..cfg.data_bits {
            match bit_center(levels, edge, 1.5 + f64::from(b), spb) {
                Some(level) => {
                    let bit_val = u16::from(level == mark); // mark=1
                    if bit_val == 1 {
                        ones += 1;
                    }
                    if cfg.lsb_first {
                        value |= bit_val << b;
                    } else {
                        value = (value << 1) | bit_val;
                    }
                }
                None => {
                    truncated = true;
                    break;
                }
            }
        }
        if truncated {
            break; // ran off the end mid-frame
        }
        let mut error: Option<FrameError> = None;
        let mut next_bit = 1.5 + f64::from(cfg.data_bits);
        if cfg.parity != Parity::None {
            if let Some(level) = bit_center(levels, edge, next_bit, spb) {
                let parity_bit = u32::from(level == mark);
                let want = match cfg.parity {
                    Parity::Even => ones % 2,
                    Parity::Odd => (ones + 1) % 2,
                    Parity::None => 0,
                };
                if parity_bit != want {
                    error = Some(FrameError::Parity);
                }
            }
            next_bit += 1.0;
        }
        // Stop bit(s) must be mark (idle) or the framing is bad.
        if bit_center(levels, edge, next_bit, spb) != Some(mark) {
            error = error.or(Some(FrameError::Framing));
        }
        out.push(SerialByte {
            start_sample: edge,
            value,
            error,
        });
        // Advance to the middle of the first stop bit, NOT past the whole frame:
        // with a fractional samples-per-bit an exact frame-length skip can
        // overshoot the next start edge and lose byte sync. From mid-stop the
        // scanner re-finds the next idle->start transition cleanly.
        let consumed =
            1.0 + f64::from(cfg.data_bits) + if cfg.parity == Parity::None { 0.0 } else { 1.0 };
        i = edge + ((consumed + 0.5) * spb) as usize;
    }
    out
}

/// Synthesize an async-serial waveform for `bytes` (used by golden tests and,
/// later, hardware stimulus verification). Idle level before/after is the mark level.
pub fn encode_async_serial(bytes: &[u8], cfg: &AsyncSerialConfig) -> Vec<bool> {
    let spb = cfg.samples_per_bit().round() as usize;
    let mark = cfg.idle_high;
    let space = !mark;
    let mut wave = Vec::new();
    let push = |wave: &mut Vec<bool>, level: bool| {
        wave.resize(wave.len() + spb, level);
    };
    // Idle for a few bits so the first start edge is detectable.
    wave.resize(wave.len() + spb * 2, mark);
    for &byte in bytes {
        push(&mut wave, space); // start bit
        let mut ones = 0u32;
        for b in 0..cfg.data_bits {
            let shift = if cfg.lsb_first {
                b
            } else {
                cfg.data_bits - 1 - b
            };
            let bit = (u16::from(byte) >> shift) & 1;
            if bit == 1 {
                ones += 1;
            }
            push(&mut wave, if bit == 1 { mark } else { space });
        }
        if cfg.parity != Parity::None {
            let parity = match cfg.parity {
                Parity::Even => ones % 2,
                Parity::Odd => (ones + 1) % 2,
                Parity::None => 0,
            };
            push(&mut wave, if parity == 1 { mark } else { space });
        }
        for _ in 0..cfg.stop_bits {
            push(&mut wave, mark); // stop bit(s)
        }
    }
    wave.resize(wave.len() + spb * 2, mark);
    wave
}

// ---------------------------------------------------------------------------
// SPI / synchronous serial interpreter. Clock-driven: sample the data
// line on each active clock edge and assemble words. Handles all four SPI modes
// via (CPOL, CPHA); the sampling edge is rising when CPOL == CPHA (modes 0 & 3),
// falling otherwise (modes 1 & 2).
// ---------------------------------------------------------------------------

/// SPI framing parameters.
#[derive(Debug, Clone, Copy)]
pub struct SpiConfig {
    pub cpol: bool, // clock idle level
    pub cpha: bool, // sample on the 2nd clock edge within a bit
    pub bits: u8,   // word size, 1..=16 (usually 8)
    pub msb_first: bool,
}

impl SpiConfig {
    /// Mode 0 (CPOL=0, CPHA=0), 8-bit, MSB-first -- the common default.
    pub fn mode0_8bit() -> Self {
        Self {
            cpol: false,
            cpha: false,
            bits: 8,
            msb_first: true,
        }
    }
    fn sample_on_rising(&self) -> bool {
        self.cpol == self.cpha
    }
}

/// Decode SPI words from parallel clock + data (MOSI or MISO) level sequences.
pub fn decode_spi(clock: &[bool], data: &[bool], cfg: &SpiConfig) -> Vec<u16> {
    decode_spi_cs(clock, data, None, cfg)
}

/// SPI decode with an optional active-low chip-select. When `cs` is supplied,
/// bits are sampled only while CS is asserted (low) and the word accumulator is
/// reset at every transfer boundary, so byte framing is recovered even when the
/// capture window starts partway through a transfer -- the common case for a
/// free-running live capture. Without CS (`None`) the decoder groups clock edges
/// by word size from the first edge, which only frames correctly when the
/// capture begins exactly at a transfer boundary.
pub fn decode_spi_cs(
    clock: &[bool],
    data: &[bool],
    cs: Option<&[bool]>,
    cfg: &SpiConfig,
) -> Vec<u16> {
    let mut n = clock.len().min(data.len());
    if let Some(cs) = cs {
        n = n.min(cs.len());
    }
    let sample_rising = cfg.sample_on_rising();
    let mut out = Vec::new();
    let mut word: u16 = 0;
    let mut count: u8 = 0;
    for i in 1..n {
        if let Some(cs) = cs {
            // Active low: drop bits sampled while deselected, and start a fresh
            // word at each transfer (the CS falling edge).
            if cs[i] {
                word = 0;
                count = 0;
                continue;
            }
            if cs[i - 1] {
                word = 0;
                count = 0;
            }
        }
        let rising = !clock[i - 1] && clock[i];
        let falling = clock[i - 1] && !clock[i];
        let sampling = if sample_rising { rising } else { falling };
        if !sampling {
            continue;
        }
        let bit = u16::from(data[i]);
        if cfg.msb_first {
            word = (word << 1) | bit;
        } else {
            word |= bit << count;
        }
        count += 1;
        if count == cfg.bits {
            out.push(word);
            word = 0;
            count = 0;
        }
    }
    out
}

/// Synthesize SPI clock + data waveforms for `words` (golden tests / stimulus).
/// The data bit is presented before its sampling edge for all modes.
pub fn encode_spi(words: &[u16], cfg: &SpiConfig) -> (Vec<bool>, Vec<bool>) {
    let sample_rising = cfg.sample_on_rising();
    // The decoder keys off edge DIRECTION only (not the idle level), so present
    // the bit and generate the actual sampling edge: low->high when sampling on
    // the rising edge, high->low otherwise. `pre` is the clock level before that
    // edge; the lead-in sits there so the first transition is a real edge.
    let pre = !sample_rising;
    let mut clk = Vec::new();
    let mut dat = Vec::new();
    for _ in 0..4 {
        clk.push(pre);
        dat.push(false);
    }
    for &word in words {
        for b in 0..cfg.bits {
            let shift = if cfg.msb_first { cfg.bits - 1 - b } else { b };
            let bit = ((word >> shift) & 1) == 1;
            clk.push(pre);
            dat.push(bit); // setup at the pre-edge level
            clk.push(!pre);
            dat.push(bit); // sampling edge here (pre -> !pre)
        }
    }
    for _ in 0..4 {
        clk.push(pre);
        dat.push(false);
    }
    (clk, dat)
}

// ---------------------------------------------------------------------------
// I2C / two-wire interpreter. START = SDA high->low while SCL is high;
// STOP = SDA low->high while SCL high; data bit sampled on each SCL rising edge
// (MSB first, 8 data bits then the ACK/NACK bit). Needs SCL + SDA channels.
// ---------------------------------------------------------------------------

/// One decoded I2C bus event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum I2cEvent {
    Start,
    /// A data byte (the first after START is address+R/W) and its 9th-bit ACK
    /// (true = ACK, SDA pulled low; false = NACK).
    Byte {
        value: u8,
        ack: bool,
    },
    Stop,
}

/// Decode I2C events from parallel SCL + SDA level sequences.
pub fn decode_i2c(scl: &[bool], sda: &[bool]) -> Vec<I2cEvent> {
    let n = scl.len().min(sda.len());
    let mut out = Vec::new();
    let mut value: u8 = 0;
    let mut nbits: u8 = 0;
    let mut active = false; // between START and STOP
    for i in 1..n {
        // START/STOP are SDA transitions while SCL is stably high.
        if scl[i - 1] && scl[i] {
            if sda[i - 1] && !sda[i] {
                out.push(I2cEvent::Start);
                active = true;
                value = 0;
                nbits = 0;
                continue;
            }
            if !sda[i - 1] && sda[i] {
                out.push(I2cEvent::Stop);
                active = false;
                value = 0;
                nbits = 0;
                continue;
            }
        }
        // Data/ACK bits are latched on the SCL rising edge.
        if active && !scl[i - 1] && scl[i] {
            if nbits < 8 {
                value = (value << 1) | u8::from(sda[i]);
                nbits += 1;
            } else {
                out.push(I2cEvent::Byte {
                    value,
                    ack: !sda[i], // ACK = SDA low on the 9th clock
                });
                value = 0;
                nbits = 0;
            }
        }
    }
    out
}

/// Synthesize I2C SCL + SDA waveforms for a START, the given (byte, ack) pairs,
/// then STOP (golden tests / stimulus).
pub fn encode_i2c(bytes: &[(u8, bool)]) -> (Vec<bool>, Vec<bool>) {
    let mut scl = Vec::new();
    let mut sda = Vec::new();
    scl.push(true);
    sda.push(true); // idle: both high
    scl.push(true);
    sda.push(true);
    scl.push(true);
    sda.push(false); // START: SDA high->low while SCL high
    for &(byte, ack) in bytes {
        for b in (0..8).rev() {
            let bit = ((byte >> b) & 1) == 1;
            scl.push(false);
            sda.push(bit); // set data while SCL low
            scl.push(true);
            sda.push(bit); // SCL rising latches the bit
        }
        // 9th clock: ACK (SDA low) / NACK (SDA high).
        let ackbit = !ack;
        scl.push(false);
        sda.push(ackbit);
        scl.push(true);
        sda.push(ackbit);
    }
    scl.push(false);
    sda.push(false); // SDA low while SCL low, ready for STOP
    scl.push(true);
    sda.push(false);
    scl.push(true);
    sda.push(true); // STOP: SDA low->high while SCL high
    scl.push(true);
    sda.push(true); // idle
    (scl, sda)
}

// ---------------------------------------------------------------------------
// Parallel bus interpreter: latch N data channels on each active clock
// edge into an N-bit word (bit i = data[i]).
// ---------------------------------------------------------------------------

/// Decode parallel-bus words. `data[i]` is channel i's level sequence; each is
/// sampled on the selected clock edge.
pub fn decode_parallel(clock: &[bool], data: &[&[bool]], sample_on_rising: bool) -> Vec<u32> {
    let mut out = Vec::new();
    for i in 1..clock.len() {
        let edge = if sample_on_rising {
            !clock[i - 1] && clock[i]
        } else {
            clock[i - 1] && !clock[i]
        };
        if !edge {
            continue;
        }
        let mut word = 0u32;
        for (bit, ch) in data.iter().enumerate() {
            if ch.get(i).copied().unwrap_or(false) {
                word |= 1u32 << bit;
            }
        }
        out.push(word);
    }
    out
}

// ---------------------------------------------------------------------------
// ISO 7816-3 (T=0 smart card) interpreter. Character framing is async
// serial at the etu rate with even parity and 2 guard-time stop bits. The TS
// initial character selects convention: direct (0x3B) = LSB-first, high=1;
// inverse (0x3F) = same idle-high framing but MSB-first with low=1 (data and
// parity complemented). Verified against real vendor SIM (direct) and
// smart-card (inverse) ATR captures.
// ---------------------------------------------------------------------------

/// ISO 7816-3 transmission convention (from the ATR's TS byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Iso7816Convention {
    Direct,
    Inverse,
}

/// Decode ISO 7816-3 T=0 characters from a single I/O line.
pub fn decode_iso7816(
    levels: &[bool],
    sample_rate_hz: u64,
    etu_baud: u32,
    convention: Iso7816Convention,
) -> Vec<SerialByte> {
    let cfg = match convention {
        Iso7816Convention::Direct => AsyncSerialConfig {
            sample_rate_hz,
            baud: etu_baud,
            data_bits: 8,
            parity: Parity::Even,
            stop_bits: 2,
            idle_high: true,
            lsb_first: true,
        },
        // Inverse convention shares the direct framing (the line still idles
        // high with a low start bit); only the bit encoding changes: logic 1 =
        // low level (so the sampled high=1 bits are complemented below) and MSB
        // first. Complementing all nine bits (8 data + parity) flips the parity
        // sense, so even parity over the logical bits is odd parity over the
        // sampled (high=1) bits: check Odd here so a valid frame is error-free.
        Iso7816Convention::Inverse => AsyncSerialConfig {
            sample_rate_hz,
            baud: etu_baud,
            data_bits: 8,
            parity: Parity::Odd,
            stop_bits: 2,
            idle_high: true,
            lsb_first: false,
        },
    };
    let mut bytes = decode_async_serial(levels, &cfg);
    if convention == Iso7816Convention::Inverse {
        // Inverse convention also complements the logical bit values.
        for byte in &mut bytes {
            byte.value = (!byte.value) & 0xFF;
        }
    }
    bytes
}

// ---------------------------------------------------------------------------
// 1-Wire interpreter. Timing-based on a single line: RESET = master
// low >= ~480us (followed by a slave presence pulse, skipped); a data bit is a
// falling edge whose value is the line level ~15us later (short low = 1, long
// low = 0), assembled LSB-first into bytes.
// ---------------------------------------------------------------------------

/// One decoded 1-Wire event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OneWireEvent {
    Reset,
    Byte(u8),
}

fn micros(sample_rate_hz: u64, us: f64) -> usize {
    ((sample_rate_hz as f64 * us / 1e6).round() as usize).max(1)
}

/// Decode 1-Wire events from a single line captured at `sample_rate_hz`.
pub fn decode_onewire(levels: &[bool], sample_rate_hz: u64) -> Vec<OneWireEvent> {
    let reset_low = micros(sample_rate_hz, 410.0); // >= ~480us low = reset (410 margin)
    let sample_off = micros(sample_rate_hz, 15.0); // sample bit ~15us after falling edge
    let mut out = Vec::new();
    let mut acc: u8 = 0;
    let mut nbits: u8 = 0;
    let mut skip_presence = false;
    let mut i = 1usize;
    while i < levels.len() {
        if !(levels[i - 1] && !levels[i]) {
            i += 1;
            continue;
        }
        // Falling edge: measure the low-pulse length.
        let start = i;
        let mut j = i;
        while j < levels.len() && !levels[j] {
            j += 1;
        }
        let low_len = j - start;
        if low_len >= reset_low {
            out.push(OneWireEvent::Reset);
            acc = 0;
            nbits = 0;
            skip_presence = true; // the slave presence pulse follows; ignore it
            i = j;
            continue;
        }
        if skip_presence {
            skip_presence = false;
            i = j;
            continue;
        }
        let bit = levels.get(start + sample_off).copied().unwrap_or(false);
        acc |= u8::from(bit) << nbits; // LSB-first
        nbits += 1;
        if nbits == 8 {
            out.push(OneWireEvent::Byte(acc));
            acc = 0;
            nbits = 0;
        }
        i = j;
    }
    out
}

fn push_level(w: &mut Vec<bool>, level: bool, n: usize) {
    w.resize(w.len() + n, level);
}

/// Synthesize a 1-Wire waveform: reset + presence, then the given bytes.
pub fn encode_onewire(bytes: &[u8], sample_rate_hz: u64) -> Vec<bool> {
    let us = |n: f64| micros(sample_rate_hz, n);
    let mut w = Vec::new();
    push_level(&mut w, true, us(50.0)); // idle high
    push_level(&mut w, false, us(480.0)); // reset low
    push_level(&mut w, true, us(60.0)); // release
    push_level(&mut w, false, us(120.0)); // slave presence low
    push_level(&mut w, true, us(60.0));
    for &byte in bytes {
        for b in 0..8 {
            let bit = (byte >> b) & 1 == 1; // LSB-first
            if bit {
                push_level(&mut w, false, us(6.0)); // write-1: short low
                push_level(&mut w, true, us(64.0));
            } else {
                push_level(&mut w, false, us(60.0)); // write-0: long low
                push_level(&mut w, true, us(10.0));
            }
        }
    }
    push_level(&mut w, true, us(50.0));
    w
}

// ---------------------------------------------------------------------------
// CAN 2.0A interpreter. Bit-timed on a single logic line (dominant=0,
// recessive=1). Removes bit-stuffing (a complementary bit after 5 identical
// bits, SOF..CRC), parses the standard 11-bit frame, and verifies CRC-15.
// ---------------------------------------------------------------------------

/// A decoded standard (2.0A) CAN data/remote frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanFrame {
    pub id: u16,
    pub rtr: bool,
    pub dlc: u8,
    pub data: Vec<u8>,
    pub crc: u16,
    pub crc_ok: bool,
}

/// CAN CRC-15 (polynomial 0x4599) over the frame bits from SOF through data.
fn can_crc15(bits: &[bool]) -> u16 {
    let mut crc: u16 = 0;
    for &b in bits {
        let invert = (((crc >> 14) & 1) != 0) ^ b;
        crc = (crc << 1) & 0x7fff;
        if invert {
            crc ^= 0x4599;
        }
    }
    crc
}

/// A reader over raw (still-stuffed) CAN bits that removes stuff bits on the fly
/// while `stuffing` is enabled (SOF..CRC).
struct CanBitReader<'a> {
    raw: &'a [bool],
    pos: usize,
    last: Option<bool>,
    run: u8,
    stuffing: bool,
    logical: Vec<bool>, // destuffed bits, for CRC over SOF..data
}

impl<'a> CanBitReader<'a> {
    fn new(raw: &'a [bool]) -> Self {
        Self {
            raw,
            pos: 0,
            last: None,
            run: 0,
            stuffing: true,
            logical: Vec::new(),
        }
    }
    fn bit(&mut self) -> Option<bool> {
        if self.stuffing && self.run == 5 {
            let stuff = *self.raw.get(self.pos)?; // discard the stuff bit
            self.pos += 1;
            self.last = Some(stuff);
            self.run = 1;
        }
        let b = *self.raw.get(self.pos)?;
        self.pos += 1;
        if self.last == Some(b) {
            self.run += 1;
        } else {
            self.run = 1;
        }
        self.last = Some(b);
        self.logical.push(b);
        Some(b)
    }
    fn bits(&mut self, n: usize) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | u32::from(self.bit()?);
        }
        Some(v)
    }
}

/// Decode standard CAN 2.0A frames from a single logic line captured at
/// `sample_rate_hz` running at `bitrate`.
pub fn decode_can(levels: &[bool], sample_rate_hz: u64, bitrate: u32) -> Vec<CanFrame> {
    let spb = (sample_rate_hz as f64 / f64::from(bitrate)).max(1.0);
    let mut out = Vec::new();
    let mut i = 1usize;
    while i < levels.len() {
        // SOF: idle recessive (high) -> dominant (low).
        if !(levels[i - 1] && !levels[i]) {
            i += 1;
            continue;
        }
        let sof = i;
        // Sample a generous run of raw bits from SOF (max standard frame with
        // stuffing is ~130 bits) at each bit centre.
        let mut raw = Vec::new();
        for b in 0..140 {
            let idx = sof + (((b as f64) + 0.5) * spb).round() as usize;
            match levels.get(idx) {
                Some(&l) => raw.push(l),
                None => break,
            }
        }
        let mut r = CanBitReader::new(&raw);
        let decoded = (|| {
            let _sof = r.bit()?; // 0
            let id = r.bits(11)? as u16;
            let rtr = r.bit()?;
            let _ide = r.bit()?;
            let _r0 = r.bit()?;
            let dlc = r.bits(4)? as u8;
            let nbytes = usize::from(dlc.min(8));
            let mut data = Vec::with_capacity(nbytes);
            for _ in 0..nbytes {
                data.push(r.bits(8)? as u8);
            }
            // CRC-15 covers SOF..data (the logical bits gathered so far).
            let computed = can_crc15(&r.logical);
            let crc = r.bits(15)? as u16;
            Some(CanFrame {
                id,
                rtr,
                dlc,
                data,
                crc,
                crc_ok: crc == computed,
            })
        })();
        if let Some(frame) = decoded {
            out.push(frame);
        }
        // Advance well past this frame (raw bits consumed) to find the next SOF.
        i = sof + ((r.pos as f64 + 10.0) * spb) as usize;
    }
    out
}

/// Shortest run of consecutive equal samples, ignoring single-sample glitches.
/// For a CAN line this approximates one unstuffed bit time in samples.
fn shortest_run(levels: &[bool]) -> Option<usize> {
    let mut min_run: Option<usize> = None;
    let mut run = 1usize;
    for i in 1..levels.len() {
        if levels[i] == levels[i - 1] {
            run += 1;
        } else {
            if run >= 2 {
                min_run = Some(min_run.map_or(run, |m| m.min(run)));
            }
            run = 1;
        }
    }
    min_run
}

/// Decode CAN without being told the bit-rate. The controller's actual bit-rate
/// can sit several percent off its nominal setting (the timing registers rarely
/// divide the kernel clock exactly), so estimate one bit time from the shortest
/// line run, then sweep bit-rates around that estimate and keep the frames from
/// the rate that yields the most CRC-valid frames. Returns the chosen bit-rate
/// (0 if nothing locked) and its frames.
pub fn decode_can_auto(levels: &[bool], sample_rate_hz: u64) -> (u32, Vec<CanFrame>) {
    let Some(bit_samples) = shortest_run(levels) else {
        return (0, Vec::new());
    };
    let base = sample_rate_hz as f64 / bit_samples as f64;
    let mut best_rate = 0u32;
    let mut best_frames = Vec::new();
    let mut best_score = 0usize;
    let mut permil: i32 = -60;
    while permil <= 60 {
        let rate = (base * (1.0 + f64::from(permil) / 1000.0)).round();
        if rate >= 1.0 {
            let rate = rate as u32;
            let frames = decode_can(levels, sample_rate_hz, rate);
            let score = frames.iter().filter(|frame| frame.crc_ok).count();
            if score > best_score {
                best_score = score;
                best_rate = rate;
                best_frames = frames;
            }
        }
        permil += 3;
    }
    (best_rate, best_frames)
}

/// Synthesize a standard CAN 2.0A data-frame waveform (SOF..EOF) for a golden
/// test: builds the logical field bits, computes CRC-15, applies bit-stuffing,
/// then renders each bit as `spb` samples. Dominant = low, recessive = high.
pub fn encode_can(id: u16, data: &[u8], sample_rate_hz: u64, bitrate: u32) -> Vec<bool> {
    let spb = (sample_rate_hz as f64 / f64::from(bitrate)).round() as usize;
    let dlc = data.len().min(8) as u8;
    // Logical field bits SOF..data (CRC input).
    let mut fields: Vec<bool> = Vec::new();
    fields.push(false); // SOF (dominant)
    for b in (0..11).rev() {
        fields.push((id >> b) & 1 == 1);
    }
    fields.push(false); // RTR (data frame = dominant)
    fields.push(false); // IDE (standard)
    fields.push(false); // r0
    for b in (0..4).rev() {
        fields.push((u16::from(dlc) >> b) & 1 == 1);
    }
    for &byte in data.iter().take(8) {
        for b in (0..8).rev() {
            fields.push((byte >> b) & 1 == 1);
        }
    }
    let crc = can_crc15(&fields);
    let mut stuffable = fields.clone();
    for b in (0..15).rev() {
        stuffable.push((crc >> b) & 1 == 1);
    }
    // Apply bit-stuffing across SOF..CRC.
    let mut stuffed: Vec<bool> = Vec::new();
    let mut last: Option<bool> = None;
    let mut run = 0u8;
    for &b in &stuffable {
        if run == 5 {
            let stuff = !last.unwrap_or(b);
            stuffed.push(stuff);
            last = Some(stuff);
            run = 1;
        }
        if last == Some(b) {
            run += 1;
        } else {
            run = 1;
        }
        last = Some(b);
        stuffed.push(b);
    }
    // Non-stuffed trailer: CRC delimiter, ACK slot (dominant, "acked"), ACK
    // delimiter, EOF (7 recessive), then interframe idle.
    stuffed.push(true); // CRC delimiter
    stuffed.push(false); // ACK slot
    stuffed.push(true); // ACK delimiter
    stuffed.resize(stuffed.len() + 7, true); // EOF: 7 recessive bits
    // Render: idle-high lead-in, the frame bits, idle-high lead-out.
    let mut wave = Vec::new();
    push_level(&mut wave, true, spb * 4);
    for &b in &stuffed {
        push_level(&mut wave, b, spb);
    }
    push_level(&mut wave, true, spb * 8);
    wave
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(decoded: &[SerialByte]) -> Vec<u16> {
        decoded.iter().map(|b| b.value).collect()
    }

    #[test]
    fn golden_can_standard_frame_round_trips_with_crc() {
        // A frame whose ID + data force >5 identical bits, exercising stuffing.
        let id = 0x123u16;
        let data = [0xFFu8, 0x00, 0xFF, 0x00];
        let wave = encode_can(id, &data, 8_000_000, 500_000);
        let frames = decode_can(&wave, 8_000_000, 500_000);
        assert_eq!(frames.len(), 1, "exactly one frame");
        let f = &frames[0];
        assert_eq!(f.id, id);
        assert_eq!(f.dlc, 4);
        assert_eq!(f.data, data.to_vec());
        assert!(f.crc_ok, "CRC-15 must verify");
        assert!(!f.rtr);
    }

    // Regression: live CAN decode must not depend on being told the exact
    // bit-rate, since the controller's real rate drifts a few percent off
    // nominal. Encode at a rate the caller never passes and let the auto
    // decoder recover both the rate and the frame.
    #[test]
    fn can_auto_bitrate_recovers_a_frame_without_the_nominal_rate() {
        let id = 0x123u16;
        let data = [0x01u8, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        // 470 kbit/s stands in for a nominal-500k controller running ~6% slow.
        let wave = encode_can(id, &data, 10_000_000, 470_000);
        let (rate, frames) = decode_can_auto(&wave, 10_000_000);
        assert!(!frames.is_empty(), "auto decode should find the frame");
        assert_eq!(frames[0].id, id);
        assert_eq!(frames[0].data, data.to_vec());
        assert!(frames[0].crc_ok, "recovered frame must pass CRC");
        assert!(
            (i64::from(rate) - 470_000).abs() < 20_000,
            "detected rate should be near the true 470k, got {rate}"
        );
    }

    #[test]
    fn golden_can_stuffing_heavy_all_zero_id_and_data() {
        // All-dominant ID/data maximally exercises stuff-bit insertion+removal.
        let wave = encode_can(0x000, &[0x00, 0x00], 8_000_000, 1_000_000);
        let frames = decode_can(&wave, 8_000_000, 1_000_000);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].id, 0x000);
        assert_eq!(frames[0].data, vec![0x00, 0x00]);
        assert!(frames[0].crc_ok);
    }

    #[test]
    fn golden_onewire_reset_and_bytes_round_trip() {
        let bytes = [0x33u8, 0xA5, 0x00, 0xFF]; // READ ROM + data
        let wave = encode_onewire(&bytes, 1_000_000); // 1 MHz -> 1 us/sample
        let decoded = decode_onewire(&wave, 1_000_000);
        assert_eq!(decoded.first(), Some(&OneWireEvent::Reset));
        let vals: Vec<u8> = decoded
            .iter()
            .filter_map(|e| match e {
                OneWireEvent::Byte(v) => Some(*v),
                OneWireEvent::Reset => None,
            })
            .collect();
        assert_eq!(vals, bytes.to_vec());
    }

    #[test]
    fn golden_parallel_8bit_latches_on_rising() {
        // 8 data channels; drive a couple of words and clock them in.
        let words: [u8; 3] = [0xA5, 0x00, 0xFF];
        // Build clock + 8 channels: for each word, [clk low, clk high] holding bits.
        let mut clk = vec![false, false];
        let mut ch: Vec<Vec<bool>> = (0..8).map(|_| vec![false, false]).collect();
        for &w in &words {
            clk.push(false);
            clk.push(true); // rising edge latches
            for (b, c) in ch.iter_mut().enumerate() {
                let bit = (w >> b) & 1 == 1;
                c.push(bit);
                c.push(bit);
            }
        }
        let refs: Vec<&[bool]> = ch.iter().map(Vec::as_slice).collect();
        assert_eq!(
            decode_parallel(&clk, &refs, true),
            words.iter().map(|&w| u32::from(w)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn golden_iso7816_direct_convention_round_trips() {
        // Direct convention = async serial 8E2, LSB-first, idle-high.
        let enc_cfg = AsyncSerialConfig {
            sample_rate_hz: 4_000_000,
            baud: 9_600,
            data_bits: 8,
            parity: Parity::Even,
            stop_bits: 2,
            idle_high: true,
            lsb_first: true,
        };
        let atr = [0x3Bu8, 0x90, 0x00]; // TS(direct) + a couple of ATR bytes
        let wave = encode_async_serial(&atr, &enc_cfg);
        let decoded = decode_iso7816(&wave, 4_000_000, 9_600, Iso7816Convention::Direct);
        assert_eq!(
            values(&decoded),
            atr.iter().map(|&b| u16::from(b)).collect::<Vec<_>>()
        );
        assert!(decoded.iter().all(|b| b.error.is_none()));
    }

    #[test]
    fn golden_i2c_write_transaction_round_trips() {
        // Address 0x50 write (0xA0), two data bytes, all ACKed; then STOP.
        let frames = [(0xA0u8, true), (0x12, true), (0x34, true)];
        let (scl, sda) = encode_i2c(&frames);
        let decoded = decode_i2c(&scl, &sda);
        assert_eq!(
            decoded,
            vec![
                I2cEvent::Start,
                I2cEvent::Byte {
                    value: 0xA0,
                    ack: true
                },
                I2cEvent::Byte {
                    value: 0x12,
                    ack: true
                },
                I2cEvent::Byte {
                    value: 0x34,
                    ack: true
                },
                I2cEvent::Stop,
            ]
        );
    }

    #[test]
    fn golden_i2c_nack_is_decoded() {
        // A read that ends with a NACK (master not-acknowledging the last byte).
        let (scl, sda) = encode_i2c(&[(0xA1, true), (0x7E, false)]);
        let decoded = decode_i2c(&scl, &sda);
        assert_eq!(
            decoded[2],
            I2cEvent::Byte {
                value: 0x7E,
                ack: false
            }
        );
        assert_eq!(decoded.last(), Some(&I2cEvent::Stop));
    }

    #[test]
    fn golden_spi_mode0_msb_first_round_trips() {
        let cfg = SpiConfig::mode0_8bit();
        let words = [0xA5u16, 0x3C, 0xFF, 0x00, 0x81];
        let (clk, dat) = encode_spi(&words, &cfg);
        assert_eq!(decode_spi(&clk, &dat, &cfg), words.to_vec());
    }

    #[test]
    fn golden_spi_all_modes_and_lsb_first() {
        let words = [0x12u16, 0xED, 0x55];
        for (cpol, cpha) in [(false, false), (false, true), (true, false), (true, true)] {
            for msb_first in [true, false] {
                let cfg = SpiConfig {
                    cpol,
                    cpha,
                    bits: 8,
                    msb_first,
                };
                let (clk, dat) = encode_spi(&words, &cfg);
                assert_eq!(
                    decode_spi(&clk, &dat, &cfg),
                    words.to_vec(),
                    "mode cpol={cpol} cpha={cpha} msb_first={msb_first}"
                );
            }
        }
    }

    // Regression: a free-running capture usually starts partway through the SPI
    // stream. Without CS the decoder groups clock edges from the first one, so
    // leading partial-bit noise shifts every byte; an active-low CS lets it
    // reset at the transfer boundary and recover the true bytes.
    #[test]
    fn spi_cs_reframes_when_capture_starts_mid_stream() {
        let cfg = SpiConfig::mode0_8bit();
        let (body_clk, body_dat) = encode_spi(&[0xA5u16, 0x3C], &cfg);
        let mut clk = Vec::new();
        let mut dat = Vec::new();
        let mut cs = Vec::new();
        // Three sampling edges of junk while DESELECTED (a previous transfer's
        // tail the capture window happened to start on).
        for &b in &[true, false, true] {
            clk.push(false);
            dat.push(b);
            cs.push(true);
            clk.push(true);
            dat.push(b);
            cs.push(true);
        }
        // The real transfer, selected (CS low) for its whole duration.
        for (&c, &d) in body_clk.iter().zip(&body_dat) {
            clk.push(c);
            dat.push(d);
            cs.push(false);
        }
        // Deselect again.
        clk.push(false);
        dat.push(false);
        cs.push(true);

        assert_eq!(
            decode_spi_cs(&clk, &dat, Some(&cs), &cfg),
            vec![0xA5, 0x3C],
            "CS framing must recover the true bytes"
        );
        assert_ne!(
            decode_spi(&clk, &dat, &cfg),
            vec![0xA5, 0x3C],
            "without CS the leading junk must misalign the bytes"
        );
    }

    #[test]
    fn golden_uart_8n1_round_trips_a_known_message() {
        // 115200 8N1 sampled at 10 MHz (~86.8 samples/bit), a typical UART
        // stimulus configuration.
        let cfg = AsyncSerialConfig::uart_8n1(10_000_000, 115_200);
        let msg = b"LP-LA1034\r\n";
        let wave = encode_async_serial(msg, &cfg);
        let decoded = decode_async_serial(&wave, &cfg);
        assert_eq!(
            values(&decoded),
            msg.iter().map(|&b| u16::from(b)).collect::<Vec<_>>()
        );
        assert!(
            decoded.iter().all(|b| b.error.is_none()),
            "no framing/parity errors on a clean frame"
        );
    }

    #[test]
    fn golden_uart_various_rates_and_9600_7e1() {
        // A slower, parity-bearing config decodes with 0 errors too.
        let cfg = AsyncSerialConfig {
            sample_rate_hz: 1_000_000,
            baud: 9_600,
            data_bits: 7,
            parity: Parity::Even,
            stop_bits: 1,
            idle_high: true,
            lsb_first: true,
        };
        let msg = b"Hi!42";
        let decoded = decode_async_serial(&encode_async_serial(msg, &cfg), &cfg);
        assert_eq!(
            values(&decoded),
            msg.iter().map(|&b| u16::from(b)).collect::<Vec<_>>()
        );
        assert!(decoded.iter().all(|b| b.error.is_none()));
    }

    #[test]
    fn framing_error_is_flagged_when_stop_bit_is_low() {
        // Corrupt the stop bit of a single byte -> framing error, value still read.
        let cfg = AsyncSerialConfig::uart_8n1(10_000_000, 115_200);
        let mut wave = encode_async_serial(&[0x55], &cfg);
        let spb = cfg.samples_per_bit().round() as usize;
        // Stop bit is the 10th cell: 2-bit idle lead-in + start + 8 data. Drive
        // the whole stop-bit cell low -> framing violation.
        let stop_start = spb * 2 + spb * 9;
        for s in wave.iter_mut().skip(stop_start).take(spb) {
            *s = false;
        }
        let decoded = decode_async_serial(&wave, &cfg);
        assert_eq!(decoded.first().map(|b| b.value), Some(0x55));
        assert_eq!(decoded[0].error, Some(FrameError::Framing));
    }

    #[test]
    fn empty_and_all_idle_lines_decode_to_nothing() {
        let cfg = AsyncSerialConfig::uart_8n1(10_000_000, 115_200);
        assert!(decode_async_serial(&[], &cfg).is_empty());
        assert!(decode_async_serial(&vec![true; 5000], &cfg).is_empty());
    }
}
