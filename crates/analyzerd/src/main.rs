use analyzerd::{
    acquisitions::reset_real_usb,
    api::{AppState, REPLUG_HINT, is_command_wedge, router, run_mcp_stdio},
};
use std::{net::SocketAddr, process::ExitCode, time::Duration};

async fn open_real_with_retry() -> AppState {
    let mut attempt = 1_u64;
    let mut reset_attempted = false;
    loop {
        match AppState::real() {
            Ok(state) => return state,
            Err(error) => {
                eprintln!("open real device attempt {attempt}: {error}");
                if !reset_attempted && is_command_wedge(&error) {
                    reset_attempted = true;
                    eprintln!("escalating once to a software USB device reset");
                    if let Err(reset_error) = reset_real_usb() {
                        eprintln!("USB device reset failed: {reset_error}");
                    }
                }
                attempt = attempt.saturating_add(1);
                let backoff_ms = attempt.saturating_mul(500).min(5_000);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }
}

fn spawn_real_recovery(state: AppState) {
    tokio::spawn(async move {
        let mut attempt = 1_u64;
        let mut reset_attempted = false;
        loop {
            let worker = state.clone();
            let result = tokio::task::spawn_blocking(move || worker.reconnect_real()).await;
            match result {
                Ok(Ok(())) => {
                    eprintln!("real device connected after {attempt} attempt(s)");
                    loop {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        let checker = state.clone();
                        let check =
                            tokio::task::spawn_blocking(move || checker.check_real_connection())
                                .await;
                        match check {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                eprintln!("real device connection lost: {error}");
                                if let Err(disconnect_error) = state.disconnect_real(error) {
                                    eprintln!(
                                        "failed to transition device offline: {disconnect_error}"
                                    );
                                }
                                break;
                            }
                            Err(error) => {
                                let message = format!("real-device health worker failed: {error}");
                                eprintln!("{message}");
                                if let Err(disconnect_error) = state.disconnect_real(message) {
                                    eprintln!(
                                        "failed to transition device offline: {disconnect_error}"
                                    );
                                }
                                break;
                            }
                        }
                    }
                    // A successful connection followed by a loss starts a new
                    // recovery episode with its own single reset escalation.
                    attempt = 1;
                    reset_attempted = false;
                    continue;
                }
                Ok(Err(error)) => {
                    eprintln!("open real device attempt {attempt}: {error}");
                    state.record_connection_error(error.to_string());
                    if !reset_attempted {
                        // First failure this episode: escalate to a software USB
                        // device reset (re-enumerates the FT245, clearing a
                        // wedged command FSM), then retry the reopen. This is the
                        // whole software-only recovery path -- no power control.
                        reset_attempted = true;
                        eprintln!("escalating once to a software USB device reset");
                        if let Err(reset_error) = reset_real_usb() {
                            eprintln!("USB device reset failed: {reset_error}");
                        }
                    } else {
                        // Reopen plus one software reset did not restore the
                        // command channel and the device is still enumerated:
                        // only a manual replug is left. Surface it clearly, then
                        // keep retrying slowly (a fresh reset each round) so a
                        // replug -- or a reset that finally takes -- recovers
                        // with no operator action beyond the cable.
                        eprintln!("software recovery exhausted; requesting USB replug");
                        state.require_replug(REPLUG_HINT);
                        reset_attempted = false;
                    }
                }
                Err(error) => {
                    let message = format!("real-device initialization worker failed: {error}");
                    eprintln!("{message}");
                    state.record_connection_error(message);
                }
            }
            attempt = attempt.saturating_add(1);
            let backoff_ms = attempt.saturating_mul(500).min(5_000);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        }
    });
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !matches!(args.first().map(String::as_str), Some("serve" | "mcp")) {
        eprintln!("usage: analyzerd {{serve [--bind ADDR]|mcp --stdio}} [--device real|sim]");
        return ExitCode::from(2);
    }
    let bind = args
        .windows(2)
        .find(|pair| pair[0] == "--bind")
        .map_or("127.0.0.1:8471", |pair| pair[1].as_str());
    let device = args
        .windows(2)
        .find(|pair| pair[0] == "--device")
        .map_or("real", |pair| pair[1].as_str());
    let is_stdio = args.first().map(String::as_str) == Some("mcp");
    let state = match device {
        "sim" => AppState::new(),
        // Stdio has no independent health surface, so retain its synchronous
        // connection contract. HTTP binds immediately and initializes in the
        // background so a missing or recovering USB device never crash-loops
        // the service or makes the web UI disappear.
        "real" if is_stdio => open_real_with_retry().await,
        "real" => AppState::real_pending("hardware initialization is pending"),
        other => {
            eprintln!("invalid --device: {other} (expected real or sim)");
            return ExitCode::from(2);
        }
    };
    if is_stdio {
        if !args.iter().any(|arg| arg == "--stdio") {
            eprintln!("analyzerd mcp requires --stdio");
            return ExitCode::from(2);
        }
        return match run_mcp_stdio(&state) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("MCP stdio error: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let address: SocketAddr = match bind.parse() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("invalid --bind: {error}");
            return ExitCode::from(2);
        }
    };
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(value) => value,
        Err(error) => {
            eprintln!("bind {address}: {error}");
            return ExitCode::FAILURE;
        }
    };
    if device == "real" {
        spawn_real_recovery(state.clone());
    }
    if let Err(error) = axum::serve(listener, router(state)).await {
        eprintln!("server error: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use analyzerd::api::is_command_wedge;

    // The recovery worker escalates to a software USB reset only for a genuinely
    // dead command channel; benign framing errors the link self-heals must not
    // trigger a disruptive reset. Canonical classifier tests live in api.rs;
    // this guards the binary's use of it against a semantics drift.
    #[test]
    fn recovery_escalation_targets_dead_channel_not_benign_framing() {
        assert!(is_command_wedge(
            "timed out waiting for 3 protocol bytes; got 0"
        ));
        assert!(is_command_wedge("bulk I/O failed: transfer was cancelled"));
        assert!(!is_command_wedge("packet number mismatch"));
        assert!(!is_command_wedge("LogicPort 0403:dc48 is not attached"));
    }
}
