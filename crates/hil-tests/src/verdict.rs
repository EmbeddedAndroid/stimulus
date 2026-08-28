use serde::Serialize;
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

#[derive(Debug, Serialize)]
pub struct Verdict<'a> {
    pub test_id: &'a str,
    pub gate: &'a str,
    pub op_ids: &'a [&'a str],
    pub pass: bool,
    pub measured: serde_json::Value,
    pub expected: serde_json::Value,
    pub tolerance: serde_json::Value,
    pub duration_ms: u64,
    pub transcript: Option<&'a str>,
}

pub fn append(verdict: &Verdict<'_>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let run_ts = env::var("LP_RUN_TS").unwrap_or_else(|_| "manual".to_owned());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or("cannot resolve repository root")?
        .join("verification/runs")
        .join(run_ts);
    fs::create_dir_all(&root)?;
    let path = root.join("verdicts.jsonl");
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    serde_json::to_writer(&mut file, verdict)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(path)
}
