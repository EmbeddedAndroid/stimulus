use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Verdict {
    test_id: String,
    gate: String,
    #[serde(default)]
    op_ids: Vec<String>,
    pass: bool,
    #[serde(default)]
    measured: serde_json::Value,
    #[serde(default)]
    expected: serde_json::Value,
    #[serde(default)]
    tolerance: serde_json::Value,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    transcript: Option<String>,
}

#[derive(Debug, Serialize)]
struct GateResult {
    status: &'static str,
    detail: String,
}

#[derive(Debug, Serialize)]
struct Report {
    schema: u8,
    run_ts: String,
    git_sha: String,
    duration_s: f64,
    device: serde_json::Value,
    stimulus: serde_json::Value,
    gates: BTreeMap<String, GateResult>,
    layers: BTreeMap<String, serde_json::Value>,
    tests: Vec<Verdict>,
    unverified_ops: usize,
    hw_suspect: Vec<serde_json::Value>,
    known_gaps: Vec<String>,
}

fn main() -> ExitCode {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let Some(root) = root else {
        eprintln!("cannot resolve repository root");
        return ExitCode::FAILURE;
    };
    match env::args().nth(1).as_deref() {
        Some("report") => match generate(&root) {
            Ok(report) => {
                println!(
                    "verification report generated: {} tests, {} unverified operations",
                    report.tests.len(),
                    report.unverified_ops
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("report generation failed: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: lp-verify report");
            ExitCode::from(2)
        }
    }
}

fn generate(root: &Path) -> Result<Report, Box<dyn std::error::Error>> {
    let run_dir = root.join("verification/runs/latest");
    let tests = read_verdicts(&run_dir.join("verdicts.jsonl"))?;
    let inventory = inventory_ops(&root.join("docs/FEATURE-INVENTORY.md"))?;
    let verified = tests
        .iter()
        .filter(|verdict| verdict.pass)
        .flat_map(|verdict| verdict.op_ids.iter().cloned())
        .collect::<BTreeSet<_>>();
    let unverified_ops = inventory.difference(&verified).count();
    let mut gates = BTreeMap::new();
    for number in 1..=12 {
        let gate = format!("D{number}");
        let relevant = tests
            .iter()
            .filter(|verdict| verdict.gate == gate)
            .collect::<Vec<_>>();
        let (status, detail) = if relevant.is_empty() {
            ("fail", "no verdicts recorded".to_owned())
        } else if relevant.iter().all(|verdict| verdict.pass) {
            ("pass", format!("{} verdicts passed", relevant.len()))
        } else {
            let failed = relevant.iter().filter(|verdict| !verdict.pass).count();
            (
                "fail",
                format!("{failed}/{} verdicts failed", relevant.len()),
            )
        };
        gates.insert(gate, GateResult { status, detail });
    }
    if let Some(d1) = gates.get_mut("D1")
        && unverified_ops != 0
    {
        d1.status = "fail";
        d1.detail = format!("{unverified_ops} operations lack passing hardware evidence");
    }
    let report = Report {
        schema: 1,
        run_ts: run_dir
            .canonicalize()
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "latest".to_owned()),
        git_sha: git_sha(root),
        duration_s: tests.iter().map(|verdict| verdict.duration_ms).sum::<u64>() as f64 / 1000.0,
        device: serde_json::json!({}),
        stimulus: serde_json::json!({}),
        gates,
        layers: BTreeMap::new(),
        tests,
        unverified_ops,
        hw_suspect: Vec::new(),
        known_gaps: Vec::new(),
    };
    let verification = root.join("verification");
    fs::create_dir_all(&verification)?;
    fs::write(
        verification.join("report.json"),
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    fs::write(
        root.join("docs/VERIFICATION-REPORT.md"),
        render_markdown(&report),
    )?;
    Ok(report)
}

fn read_verdicts(path: &Path) -> Result<Vec<Verdict>, Box<dyn std::error::Error>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let mut verdicts = Vec::new();
    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        serde_json::from_str(&line).map_or_else(
            |error| Err(format!("{}:{}: {error}", path.display(), index + 1)),
            |verdict| {
                verdicts.push(verdict);
                Ok(())
            },
        )?;
    }
    Ok(verdicts)
}

fn inventory_ops(path: &Path) -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
    let text = fs::read_to_string(path)?;
    Ok(text
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|line| line.split('`').next())
        .map(str::to_owned)
        .collect())
}

fn git_sha(root: &Path) -> String {
    std::process::Command::new("git")
        .arg("-c")
        .arg(format!("safe.directory={}", root.display()))
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn render_markdown(report: &Report) -> String {
    let mut out = format!(
        "<!-- generated by lp-verify; do not edit -->\n# Verification report\n\nRun: `{}`\n\nCommit: `{}`\n\nRecorded tests: {}\n\nUnverified operations: {}\n\n| Gate | Status | Detail |\n|---|---|---|\n",
        report.run_ts,
        report.git_sha,
        report.tests.len(),
        report.unverified_ops
    );
    for number in 1..=12 {
        let gate = format!("D{number}");
        let Some(result) = report.gates.get(&gate) else {
            continue;
        };
        out.push_str(&format!(
            "| {gate} | {} | {} |\n",
            result.status, result.detail
        ));
    }
    out.push_str(
        "\nThis file reflects archived verdicts only. Missing evidence is reported as failure.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_parser_extracts_only_operation_rows() -> Result<(), Box<dyn std::error::Error>> {
        let path = env::temp_dir().join(format!("lp-verify-inventory-{}", std::process::id()));
        fs::write(
            &path,
            "# Inventory\n| Operation | Notes |\n|---|---|\n| `device.status` | x |\n| prose | y |\n| `acq.single` | z |\n",
        )?;
        let parsed = inventory_ops(&path)?;
        fs::remove_file(path)?;
        assert_eq!(
            parsed,
            BTreeSet::from(["acq.single".to_owned(), "device.status".to_owned()])
        );
        Ok(())
    }
}
