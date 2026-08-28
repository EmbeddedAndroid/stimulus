#![cfg(feature = "hardware")]
//! D11 robustness -- sustained-load soak driving the LIVE analyzerd over HTTP
//! (the deployed reliability path: same daemon, device setup, readback and
//! recovery a user hits). Hammers `acq.single` back to back and proves zero
//! failures and zero USB errors -- with NO power-cycle. A power-cycle recovery
//! counts as a failed run.
//!
//! Requires `analyzerd` running and connected (D11 measures the daemon, per the
//! plan). Base URL from `LP_BASE_URL` (default the compose service name).
//! Opt-in via `LP_SOAK=1` (`./lp hil --soak`); sizing via `LP_SOAK_ITERS`
//! (default 100) and `LP_SOAK_SECS` (default 3600).

use hil_tests::verdict;
use std::{
    env,
    error::Error,
    time::{Duration, Instant},
};

fn base_url() -> String {
    env::var("LP_BASE_URL").unwrap_or_else(|_| "http://analyzerd:8471".to_owned())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build()
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn soak_enabled() -> bool {
    env_u64("LP_SOAK", 0) != 0
}

fn as_ms(elapsed: Duration) -> u64 {
    elapsed.as_millis().min(u128::from(u64::MAX)) as u64
}

/// One acquisition. Returns Ok(sample_count) or Err(reason) -- an HTTP 502
/// (USB_ERROR) or any transport failure is a soak failure, not a test panic.
fn acquire(agent: &ureq::Agent, base: &str) -> Result<u64, String> {
    let url = format!("{base}/api/ops/acq.single");
    match agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string("{}")
    {
        Ok(resp) => {
            let body: serde_json::Value = resp
                .into_json()
                .map_err(|e| format!("decode acq.single response: {e}"))?;
            let runs = body
                .get("capture")
                .and_then(|c| c.get("runs"))
                .and_then(|r| r.as_array())
                .ok_or_else(|| format!("no capture.runs in response: {body}"))?;
            let samples: u64 = runs
                .iter()
                .filter_map(|run| run.get("count").and_then(serde_json::Value::as_u64))
                .sum();
            if samples == 0 {
                return Err("empty readback".to_owned());
            }
            Ok(samples)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {}", body.trim()))
        }
        Err(e) => Err(format!("transport: {e}")),
    }
}

/// The daemon's USB error count (0 == a clean, self-heal-free run).
fn usb_error_count(agent: &ureq::Agent, base: &str) -> Result<u64, String> {
    let url = format!("{base}/api/ops/device.status");
    let body: serde_json::Value = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string("{}")
        .map_err(|e| format!("device.status: {e}"))?
        .into_json()
        .map_err(|e| format!("device.status decode: {e}"))?;
    if body.get("state").and_then(serde_json::Value::as_str) != Some("connected") {
        return Err(format!("device not connected: {body}"));
    }
    body.get("usb_error_count")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("no usb_error_count: {body}"))
}

// D11: N consecutive single acquisitions with 0 failures and 0 USB errors.
#[test]
fn hundred_single_acquisitions() -> Result<(), Box<dyn Error>> {
    if !soak_enabled() {
        eprintln!("hil::soak::hundred_single_acquisitions skipped (set LP_SOAK=1)");
        return Ok(());
    }
    let base = base_url();
    let agent = agent();
    let started = Instant::now();
    let iters = env_u64("LP_SOAK_ITERS", 100);
    let mut min_samples = u64::MAX;
    let mut failure: Option<String> = None;
    for i in 0..iters {
        match acquire(&agent, &base) {
            Ok(samples) => min_samples = min_samples.min(samples),
            Err(reason) => {
                failure = Some(format!("capture {i}/{iters}: {reason}"));
                break;
            }
        }
    }
    let errors = usb_error_count(&agent, &base).unwrap_or(u64::MAX);
    let pass = failure.is_none() && errors == 0;
    verdict::append(&verdict::Verdict {
        test_id: "hil::soak::hundred_single_acquisitions",
        gate: "D11",
        op_ids: &["acq.single"],
        pass,
        measured: serde_json::json!({
            "iterations": iters, "usb_error_count": errors,
            "min_samples": min_samples, "failure": failure,
        }),
        expected: serde_json::json!({"iterations": iters, "usb_error_count": 0, "failures": 0}),
        tolerance: serde_json::Value::Null,
        duration_ms: as_ms(started.elapsed()),
        transcript: None,
    })?;
    if let Some(reason) = failure {
        return Err(reason.into());
    }
    if errors != 0 {
        return Err(format!("{errors} USB errors over {iters} acquisitions").into());
    }
    Ok(())
}

// D11: sustained recurring soak (default 1 hour) with 0 failures / 0 USB errors.
#[test]
fn recurring_one_hour() -> Result<(), Box<dyn Error>> {
    if !soak_enabled() {
        eprintln!("hil::soak::recurring_one_hour skipped (set LP_SOAK=1)");
        return Ok(());
    }
    let base = base_url();
    let agent = agent();
    let started = Instant::now();
    let secs = env_u64("LP_SOAK_SECS", 3600);
    let deadline = started + Duration::from_secs(secs);
    let mut captures = 0u64;
    let mut failure: Option<String> = None;
    while Instant::now() < deadline {
        match acquire(&agent, &base) {
            Ok(_) => captures += 1,
            Err(reason) => {
                failure = Some(format!("soak capture {captures}: {reason}"));
                break;
            }
        }
    }
    let errors = usb_error_count(&agent, &base).unwrap_or(u64::MAX);
    let pass = failure.is_none() && errors == 0;
    verdict::append(&verdict::Verdict {
        test_id: "hil::soak::recurring_one_hour",
        gate: "D11",
        op_ids: &["acq.single"],
        pass,
        measured: serde_json::json!({
            "seconds": secs, "captures": captures,
            "usb_error_count": errors, "failure": failure,
        }),
        expected: serde_json::json!({"usb_error_count": 0, "failures": 0}),
        tolerance: serde_json::Value::Null,
        duration_ms: as_ms(started.elapsed()),
        transcript: None,
    })?;
    if let Some(reason) = failure {
        return Err(reason.into());
    }
    if errors != 0 {
        return Err(format!("soak: {errors} USB errors over {captures} captures").into());
    }
    Ok(())
}
