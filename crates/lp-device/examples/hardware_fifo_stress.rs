//! Hardware FIFO stress and vendor-recovery harness (diagnostic only).
//!
//! Measures the per-transaction failure rate of the FT245 FIFO command path on
//! the real analyzer and tests whether the vendor's `Flush` recovery (reopen,
//! purge, zero tail + `0x01`, purge, packet counter := 0) restores the parser
//! without a power cycle. Every line is stamped with microseconds since start
//! so it can be cross-referenced with a usbmon capture.
use lp_device::{
    clock::WallClock,
    link::{DevError, Link, LinkConfig},
    real::RealTransport,
};
use lp_proto::{
    addr::Addr,
    encode::{
        Provenance,
        trigger::{TriggerLayout, TriggerSpec},
    },
    regs,
    setup_seq::{Dirty, Setup, setup_sequence},
};
use std::time::{Duration, Instant};

struct Options {
    reads: usize,
    setups: usize,
    flush_zeros: Vec<usize>,
    tail_zeros: usize,
    recover: bool,
    max_failures: usize,
    image: u8,
    flush_first: bool,
    read_gap: Duration,
    op_gap: Duration,
    flush_attempts: usize,
    read_reg: ReadReg,
    recover_mode: RecoverMode,
    keepalive: bool,
    force_c3: bool,
    idle_loop: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ReadReg {
    ImageId,
    Status,
    Version,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RecoverMode {
    Flush,
    Reconfigure,
    FlushThenReconfigure,
}

fn parse() -> Result<Options, String> {
    let mut opts = Options {
        reads: 200,
        setups: 10,
        flush_zeros: vec![0xffff, 30_000],
        tail_zeros: 8_712,
        recover: true,
        max_failures: 10,
        image: 7,
        flush_first: false,
        read_gap: Duration::ZERO,
        op_gap: Duration::ZERO,
        flush_attempts: 3,
        read_reg: ReadReg::ImageId,
        recover_mode: RecoverMode::Flush,
        keepalive: false,
        force_c3: false,
        idle_loop: false,
    };
    let mut acknowledged = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| -> Result<String, String> {
            args.next().ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--i-understand-this-configures-hardware" => acknowledged = true,
            "--reads" => opts.reads = value("--reads")?.parse().map_err(|e| format!("{e}"))?,
            "--setups" => opts.setups = value("--setups")?.parse().map_err(|e| format!("{e}"))?,
            "--image" => opts.image = value("--image")?.parse().map_err(|e| format!("{e}"))?,
            "--tail-zeros" => {
                opts.tail_zeros = value("--tail-zeros")?.parse().map_err(|e| format!("{e}"))?;
            }
            "--flush-zeros" => {
                opts.flush_zeros = value("--flush-zeros")?
                    .split(',')
                    .map(|part| part.trim().parse::<usize>().map_err(|e| format!("{e}")))
                    .collect::<Result<Vec<_>, _>>()?;
            }
            "--max-failures" => {
                opts.max_failures = value("--max-failures")?
                    .parse()
                    .map_err(|e| format!("{e}"))?;
            }
            "--flush-attempts" => {
                opts.flush_attempts = value("--flush-attempts")?
                    .parse()
                    .map_err(|e| format!("{e}"))?;
            }
            "--read-gap-ms" => {
                let ms: u64 = value("--read-gap-ms")?
                    .parse()
                    .map_err(|e| format!("{e}"))?;
                opts.read_gap = Duration::from_millis(ms);
            }
            "--op-gap-ms" => {
                let ms: u64 = value("--op-gap-ms")?.parse().map_err(|e| format!("{e}"))?;
                opts.op_gap = Duration::from_millis(ms);
            }
            "--read-reg" => {
                opts.read_reg = match value("--read-reg")?.as_str() {
                    "image_id" => ReadReg::ImageId,
                    "status" => ReadReg::Status,
                    "version" => ReadReg::Version,
                    other => return Err(format!("unknown --read-reg {other}")),
                };
            }
            "--recover-mode" => {
                opts.recover_mode = match value("--recover-mode")?.as_str() {
                    "flush" => RecoverMode::Flush,
                    "reconfigure" => RecoverMode::Reconfigure,
                    "flush-then-reconfigure" => RecoverMode::FlushThenReconfigure,
                    other => return Err(format!("unknown --recover-mode {other}")),
                };
            }
            "--no-recover" => opts.recover = false,
            "--flush-first" => opts.flush_first = true,
            "--keepalive" => opts.keepalive = true,
            "--force-c3" => opts.force_c3 = true,
            "--idle-loop" => opts.idle_loop = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if !acknowledged {
        return Err(
            "refusing hardware mutation without --i-understand-this-configures-hardware".into(),
        );
    }
    Ok(opts)
}

struct Log {
    start: Instant,
}
impl Log {
    fn line(&self, msg: &str) {
        println!("t=+{:>9}us {msg}", self.start.elapsed().as_micros());
    }
}

type HwLink = Link<RealTransport, WallClock>;

fn read_image_id(link: &mut HwLink, expected: u8) -> Result<Duration, String> {
    let started = Instant::now();
    match link.read(regs::ctrl::IMAGE_ID, 1) {
        Ok(value) if value == [expected] => Ok(started.elapsed()),
        Ok(value) => Err(format!(
            "IMAGE_ID mismatch {value:02x?} (expected {expected:#04x})"
        )),
        Err(error) => Err(describe(&error)),
    }
}

/// One framed read of the selected register. STATUS is volatile, so only
/// framing/packet number are verified for it; VERSION must be stable.
fn read_selected(
    link: &mut HwLink,
    reg: ReadReg,
    expected_id: u8,
    version: &mut Option<u8>,
) -> Result<(Duration, u8), String> {
    match reg {
        ReadReg::ImageId => read_image_id(link, expected_id).map(|d| (d, expected_id)),
        ReadReg::Status => {
            let started = Instant::now();
            match link.read(regs::ctrl::STATUS, 1) {
                Ok(value) if value.len() == 1 => Ok((started.elapsed(), value[0])),
                Ok(value) => Err(format!("STATUS length {}", value.len())),
                Err(error) => Err(describe(&error)),
            }
        }
        ReadReg::Version => {
            let started = Instant::now();
            match link.read(regs::ctrl::VERSION, 1) {
                Ok(value) if value.len() == 1 => match *version {
                    Some(seen) if seen != value[0] => {
                        Err(format!("VERSION changed {seen:#04x} -> {:#04x}", value[0]))
                    }
                    _ => {
                        *version = Some(value[0]);
                        Ok((started.elapsed(), value[0]))
                    }
                },
                Ok(value) => Err(format!("VERSION length {}", value.len())),
                Err(error) => Err(describe(&error)),
            }
        }
    }
}

/// Forced cold reconfiguration through the FT245 bit-bang path. Diagnostic
/// only: production policy never reconfigures a silent-but-DONE FPGA. Here
/// the question is whether a wedged parser can be cleared without cutting
/// power.
fn reconfigure(link: &mut HwLink, image: &[u8], idx: u8, log: &Log) -> Result<(), String> {
    let started = Instant::now();
    match link.configure_fpga(image, idx, true) {
        Ok(outcome) => {
            log.line(&format!(
                "RECONFIGURE ok warm={} id={:#04x} in {}ms {}",
                outcome.warm,
                outcome.id,
                started.elapsed().as_millis(),
                pins_note(link)
            ));
            Ok(())
        }
        Err(error) => {
            let note = pins_note(link);
            log.line(&format!(
                "RECONFIGURE failed after {}ms: {error} {note}",
                started.elapsed().as_millis()
            ));
            Err(format!("{error}"))
        }
    }
}

fn describe(error: &DevError) -> String {
    format!("{error}")
}

/// The vendor recovery: Flush, then prove the parser with IMAGE_ID. If the
/// flushed session does not answer, try the one-time `0x61` enable once and
/// prove again. Returns which variant restored the session.
fn recover(
    link: &mut HwLink,
    opts: &Options,
    log: &Log,
    expected: u8,
    image: &[u8],
) -> Result<String, String> {
    if opts.recover_mode == RecoverMode::Reconfigure {
        reconfigure(link, image, opts.image, log)?;
        return read_image_id(link, expected)
            .map(|_| "reconfigure".to_string())
            .map_err(|e| format!("reconfigure verify: {e}"));
    }
    let flushed = recover_flush(link, opts, log, expected);
    if flushed.is_ok() || opts.recover_mode == RecoverMode::Flush {
        return flushed;
    }
    log.line("RECOVER flush exhausted; trying forced reconfigure");
    reconfigure(link, image, opts.image, log)?;
    read_image_id(link, expected)
        .map(|_| "flush-then-reconfigure".to_string())
        .map_err(|e| format!("reconfigure verify: {e}"))
}

fn recover_flush(
    link: &mut HwLink,
    opts: &Options,
    log: &Log,
    expected: u8,
) -> Result<String, String> {
    for attempt in 1..=opts.flush_attempts {
        log.line(&format!(
            "RECOVER attempt={attempt} vendor_flush zeros={:?}",
            opts.flush_zeros
        ));
        let flushed = Instant::now();
        match link.vendor_flush(&opts.flush_zeros) {
            Ok(()) => log.line(&format!(
                "RECOVER flush ok in {}us",
                flushed.elapsed().as_micros()
            )),
            Err(error) => {
                log.line(&format!("RECOVER flush failed: {error}"));
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        }
        match read_image_id(link, expected) {
            Ok(latency) => {
                log.line(&format!(
                    "RECOVERED variant=flush-only IMAGE_ID ok {}us",
                    latency.as_micros()
                ));
                return Ok("flush-only".into());
            }
            Err(error) => log.line(&format!("RECOVER flush-only IMAGE_ID failed: {error}")),
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!(
        "session not recovered after {} flush attempts",
        opts.flush_attempts
    ))
}

fn pins_note(link: &mut HwLink) -> String {
    match link.pins() {
        Ok(pins) => format!("pins={pins:#04x}"),
        Err(error) => format!("pins=? ({error})"),
    }
}

fn stats(label: &str, samples: &mut [Duration], log: &Log) {
    if samples.is_empty() {
        log.line(&format!("STATS {label}: no samples"));
        return;
    }
    samples.sort();
    let n = samples.len();
    let total: Duration = samples.iter().sum();
    let pct = |p: f64| samples[((n as f64 - 1.0) * p).round() as usize].as_micros();
    log.line(&format!(
        "STATS {label}: n={n} min={}us p50={}us p90={}us p99={}us max={}us mean={}us",
        samples[0].as_micros(),
        pct(0.5),
        pct(0.9),
        pct(0.99),
        samples[n - 1].as_micros(),
        (total / n as u32).as_micros()
    ));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opts = parse()?;
    let log = Log {
        start: Instant::now(),
    };
    log.line(&format!(
        "BEGIN reads={} setups={} tail_zeros={} flush_zeros={:?} recover={} mode={:?} reg={:?} flush_first={} read_gap={:?} op_gap={:?}",
        opts.reads, opts.setups, opts.tail_zeros, opts.flush_zeros, opts.recover, opts.recover_mode, opts.read_reg, opts.flush_first, opts.read_gap, opts.op_gap
    ));
    let transport = RealTransport::open()?;
    let mut link = Link::new(transport, WallClock::default(), LinkConfig::default());
    link.set_fifo_tail_zeros(opts.tail_zeros);
    let ccf = lp_ccf::Ccf::load("fixtures/vendor/LogicPort.ccf", true)?;
    let upload = ccf.image_for_upload(opts.image)?;
    let expected = opts.image | 0x10;
    let configured = match link.configure_fpga(&upload, opts.image, false) {
        Ok(outcome) => outcome,
        Err(error) => {
            log.line(&format!(
                "CONFIGURE failed: {error} {}",
                pins_note(&mut link)
            ));
            return Err(Box::new(error));
        }
    };
    log.line(&format!(
        "CONFIGURED warm={} id={:#04x} elapsed={}ms drained={} {}",
        configured.warm,
        configured.id,
        configured.elapsed.as_millis(),
        configured.drained_bytes,
        pins_note(&mut link)
    ));
    if configured.id != expected {
        return Err(format!(
            "expected IMAGE_ID {expected:#04x}, got {:#04x}",
            configured.id
        )
        .into());
    }

    let mut failures = 0usize;
    let mut recoveries: Vec<String> = Vec::new();
    let mut aborted = false;

    if opts.idle_loop {
        // Replay the idle poll loop: freq-counter reads/writes in segment
        // 0x4000 and wire-status in 0x0010, constantly switching banks.
        // `reads` = number of iterations.
        use lp_proto::addr::Addr;
        let mut ok = 0usize;
        let mut i = 0usize;
        while i < opts.reads {
            i += 1;
            let src = if i.is_multiple_of(2) { 0x20u8 } else { 0x21u8 };
            let step: Result<(), String> = (|| {
                link.read(Addr(0x4000_0005), 1).map_err(|e| describe(&e))?;
                link.read(Addr(0x4000_0000), 1).map_err(|e| describe(&e))?;
                link.write_checked(Addr(0x4000_0000), &[src], None)
                    .map_err(|e| describe(&e))?;
                link.read(Addr(0x4000_0001), 4).map_err(|e| describe(&e))?;
                link.read(Addr(0x1000_0001), 9).map_err(|e| describe(&e))?;
                Ok(())
            })();
            match step {
                Ok(()) => {
                    ok += 1;
                    if i.is_multiple_of(50) || i <= 3 {
                        log.line(&format!(
                            "WIN-LOOP iter {i}/{} ok ({} cmds)",
                            opts.reads,
                            i * 6
                        ));
                    }
                }
                Err(error) => {
                    log.line(&format!(
                        "FAIL phase=idle-loop iter={i} (~{} cmds) err={error} {}",
                        i * 6,
                        pins_note(&mut link)
                    ));
                    aborted = true;
                    break;
                }
            }
        }
        log.line(&format!(
            "WIN-LOOP done ok={ok}/{} (~{} commands)",
            opts.reads,
            ok * 6
        ));
        log.line(&format!(
            "SUMMARY idle_loop_ok={ok}/{} aborted={aborted}",
            opts.reads
        ));
        return if aborted {
            Err("idle-loop wedged".into())
        } else {
            Ok(())
        };
    }

    if opts.flush_first {
        log.line("FLUSH-FIRST: exercising vendor Flush on a healthy session");
        match recover(&mut link, &opts, &log, expected, &upload) {
            Ok(variant) => recoveries.push(format!("flush-first:{variant}")),
            Err(error) => {
                log.line(&format!("FLUSH-FIRST failed: {error}"));
                return Err(error.into());
            }
        }
    }

    // Phase A: IMAGE_ID read loop.
    let mut read_latency = Vec::with_capacity(opts.reads);
    let mut reads_ok = 0usize;
    let mut i = 0usize;
    let mut version_seen: Option<u8> = None;
    while i < opts.reads && !aborted {
        i += 1;
        if opts.force_c3 {
            link.invalidate_bank();
        }
        let op_result = if opts.keepalive {
            let started = Instant::now();
            match link.write_checked(regs::ctrl::ARM, &[0x00], None) {
                Ok(()) => Ok((started.elapsed(), 0u8)),
                Err(error) => Err(describe(&error)),
            }
        } else {
            read_selected(&mut link, opts.read_reg, expected, &mut version_seen)
        };
        match op_result {
            Ok((latency, value)) => {
                reads_ok += 1;
                read_latency.push(latency);
                if i.is_multiple_of(50) || i <= 3 {
                    log.line(&format!(
                        "OP {i}/{} {}={value:#04x} ok ({}us)",
                        opts.reads,
                        if opts.keepalive {
                            "keepalive(ARM=0)"
                        } else {
                            "read"
                        },
                        latency.as_micros()
                    ));
                }
            }
            Err(error) => {
                failures += 1;
                log.line(&format!(
                    "FAIL phase=read iter={i} failures={failures} err={error} {}",
                    pins_note(&mut link)
                ));
                if !opts.recover || failures >= opts.max_failures {
                    aborted = true;
                    break;
                }
                match recover(&mut link, &opts, &log, expected, &upload) {
                    Ok(variant) => recoveries.push(format!("read#{i}:{variant}")),
                    Err(error) => {
                        log.line(&format!("UNRECOVERABLE after read iter={i}: {error}"));
                        aborted = true;
                    }
                }
            }
        }
        if !opts.read_gap.is_zero() {
            std::thread::sleep(opts.read_gap);
        }
    }
    stats("image_id_read", &mut read_latency, &log);
    log.line(&format!(
        "PHASE-A reads_ok={reads_ok} attempted={i} failures_so_far={failures}"
    ));

    // Phase B: full vendor settings passes.
    let setup = Setup {
        rate: [0x21, 0x00],
        mode: 0x14,
        enable_mask: (1_u64 << 34) - 1,
        channel_mask_active: true,
        mask2: 0,
        mode_flag: false,
        trigger: TriggerSpec::default(),
        trigger_layout: TriggerLayout::default(),
        threshold_code: 0x024c,
        pre_count: 1032,
        post_count: 1016,
        arm: false,
        provenance: Provenance::Provisional,
    };
    let mut passes_ok = 0usize;
    let mut writes_ok = 0usize;
    let mut pass_latency = Vec::new();
    let mut just_flushed = !recoveries.is_empty();
    let mut pass = 0usize;
    while pass < opts.setups && !aborted {
        pass += 1;
        let dirty = Dirty {
            rate: pass == 1 || just_flushed,
            mode: true,
            trigger: true,
            threshold: true,
            position: true,
        };
        just_flushed = false;
        let writes: Vec<(Addr, Vec<u8>)> = setup_sequence(&setup, dirty)
            .into_iter()
            .map(|op| (op.addr, op.data))
            .collect();
        let started = Instant::now();
        let mut failed_at: Option<(usize, String)> = None;
        for (index, (addr, data)) in writes.iter().enumerate() {
            let next = writes.get(index + 1).map(|(addr, _)| *addr);
            if !opts.op_gap.is_zero() {
                std::thread::sleep(opts.op_gap);
            }
            match link.write_checked(*addr, data, next) {
                Ok(()) => writes_ok += 1,
                Err(error) => {
                    failed_at = Some((index, format!("{error}")));
                    break;
                }
            }
        }
        match failed_at {
            None => match read_image_id(&mut link, expected) {
                Ok(_) => {
                    passes_ok += 1;
                    pass_latency.push(started.elapsed());
                    log.line(&format!(
                        "SETUP pass={pass}/{} ok writes={} rate_dirty={} in {}ms",
                        opts.setups,
                        writes.len(),
                        dirty.rate,
                        started.elapsed().as_millis()
                    ));
                }
                Err(error) => {
                    failures += 1;
                    log.line(&format!(
                        "FAIL phase=setup-verify pass={pass} failures={failures} err={error} {}",
                        pins_note(&mut link)
                    ));
                    failed_at = Some((writes.len(), error));
                }
            },
            Some((index, ref error)) => {
                failures += 1;
                let (addr, data) = &writes[index];
                log.line(&format!(
                    "FAIL phase=setup pass={pass} op={index}/{} addr={:#010x} data={data:02x?} failures={failures} err={error} {}",
                    writes.len(),
                    addr.0,
                    pins_note(&mut link)
                ));
            }
        }
        if failed_at.is_some() {
            if !opts.recover || failures >= opts.max_failures {
                aborted = true;
                break;
            }
            match recover(&mut link, &opts, &log, expected, &upload) {
                Ok(variant) => {
                    recoveries.push(format!("setup#{pass}:{variant}"));
                    just_flushed = true;
                }
                Err(error) => {
                    log.line(&format!("UNRECOVERABLE after setup pass={pass}: {error}"));
                    aborted = true;
                }
            }
        }
    }
    stats("setup_pass", &mut pass_latency, &log);
    log.line(&format!(
        "SUMMARY reads_ok={reads_ok}/{} setup_passes_ok={passes_ok}/{} writes_ok={writes_ok} failures={failures} recoveries={} aborted={aborted} {}",
        opts.reads,
        opts.setups,
        recoveries.len(),
        pins_note(&mut link)
    ));
    for recovery in &recoveries {
        log.line(&format!("RECOVERY {recovery}"));
    }
    if aborted {
        Err("run aborted".into())
    } else {
        Ok(())
    }
}
