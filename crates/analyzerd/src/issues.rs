//! Agent-facing issue reports about the service itself.
//!
//! An agent that hits a bug in the daemon can file it from inside the tool,
//! with the daemon's own context attached, instead of the report dying in a
//! session log. A report is a claim for a human to triage; filing one never
//! changes what any other operation answers.
//!
//! Reports are deduplicated by title and tool so a retry loop adds weight to
//! one report rather than burying the queue, and a report that matches an
//! already-resolved issue reopens it as a regression, which is the signal
//! worth surfacing loudest.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "logicport-issues/1";
pub const OPEN: &str = "open";
pub const ACKNOWLEDGED: &str = "acknowledged";
pub const RESOLVED: &str = "resolved";
const STATUSES: [&str; 3] = [OPEN, ACKNOWLEDGED, RESOLVED];

/// Whether a filing created a report, added weight to an existing one, or
/// reopened one that had been marked resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filed {
    New,
    Duplicate,
    Regression,
}
impl Filed {
    pub fn as_str(self) -> &'static str {
        match self {
            Filed::New => "new",
            Filed::Duplicate => "duplicate",
            Filed::Regression => "regression",
        }
    }
    pub fn note(self) -> &'static str {
        match self {
            Filed::New => "filed",
            Filed::Duplicate => {
                "this matches an open report; counted against it rather than filed twice"
            }
            Filed::Regression => {
                "this was already marked resolved; reopened as a regression, which is worth \
                 saying out loud to whoever fixed it"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub id: u64,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<String>,
    /// The operation whose answer was wrong, when it was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The arguments it was called with, so the call can be repeated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Distinct reporters. Ten sightings from one retry loop is not ten agents,
    /// so priority follows this rather than the raw count.
    #[serde(default)]
    pub reporters: Vec<String>,
    pub sightings: u32,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// Daemon and device state captured at filing time, which the agent would
    /// otherwise have to reconstruct by hand.
    #[serde(default)]
    pub context: Value,
    #[serde(default)]
    pub evidence: Vec<Value>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NewIssue {
    pub title: String,
    pub expected: Option<String>,
    pub observed: Option<String>,
    pub tool: Option<String>,
    pub args: Option<Value>,
    pub reporter: Option<String>,
    pub context: Value,
}

/// Match key for deduplication: the title reduced to its words plus the tool.
/// Case, padding, and punctuation differences between two sightings of the same
/// bug must not file it twice.
fn signature(title: &str, tool: Option<&str>) -> String {
    let words: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!("{}\u{1f}{}", words, tool.unwrap_or_default().to_lowercase())
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Persisted {
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    issues: Vec<Issue>,
}

#[derive(Debug, Default)]
pub struct IssueStore {
    issues: Vec<Issue>,
    next_id: u64,
    path: Option<PathBuf>,
}

impl IssueStore {
    /// Load the store from `path`, if one is configured and already exists. A
    /// missing or unreadable file starts empty rather than failing the daemon:
    /// losing the issue log must never stop the service it reports on.
    pub fn new(path: Option<PathBuf>) -> Self {
        let mut store = Self {
            issues: Vec::new(),
            next_id: 1,
            path,
        };
        if let Some(path) = store.path.clone()
            && let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(loaded) = serde_json::from_str::<Persisted>(&text)
        {
            store.next_id = loaded.next_id.max(1);
            store.issues = loaded.issues;
            let highest = store.issues.iter().map(|issue| issue.id).max().unwrap_or(0);
            store.next_id = store.next_id.max(highest + 1);
        }
        store
    }

    pub fn len(&self) -> usize {
        self.issues.len()
    }
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn file(&mut self, new: NewIssue, now_ms: u64) -> (Issue, Filed) {
        let key = signature(&new.title, new.tool.as_deref());
        let existing = self
            .issues
            .iter_mut()
            .find(|issue| signature(&issue.title, issue.tool.as_deref()) == key);
        if let Some(issue) = existing {
            let filed = if issue.status == RESOLVED {
                issue.status = OPEN.to_owned();
                Filed::Regression
            } else {
                Filed::Duplicate
            };
            issue.sightings = issue.sightings.saturating_add(1);
            issue.updated_ms = now_ms;
            if let Some(reporter) = new.reporter
                && !issue.reporters.iter().any(|seen| seen == &reporter)
            {
                issue.reporters.push(reporter);
            }
            // A later sighting can carry detail the first one lacked.
            if issue.expected.is_none() {
                issue.expected = new.expected;
            }
            if issue.observed.is_none() {
                issue.observed = new.observed;
            }
            let issue = issue.clone();
            self.persist();
            return (issue, filed);
        }
        let issue = Issue {
            id: self.next_id,
            title: new.title,
            status: OPEN.to_owned(),
            expected: new.expected,
            observed: new.observed,
            tool: new.tool,
            args: new.args,
            reporters: new.reporter.into_iter().collect(),
            sightings: 1,
            created_ms: now_ms,
            updated_ms: now_ms,
            context: new.context,
            evidence: Vec::new(),
            notes: Vec::new(),
        };
        self.next_id = self.next_id.saturating_add(1);
        self.issues.push(issue.clone());
        self.persist();
        (issue, Filed::New)
    }

    pub fn list(&self, status: Option<&str>, limit: usize) -> Vec<Issue> {
        self.issues
            .iter()
            .rev()
            .filter(|issue| status.is_none_or(|want| issue.status == want))
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn get(&self, id: u64) -> Option<Issue> {
        self.issues.iter().find(|issue| issue.id == id).cloned()
    }

    pub fn update(
        &mut self,
        id: u64,
        status: Option<&str>,
        note: Option<&str>,
        now_ms: u64,
    ) -> Result<Issue, String> {
        if let Some(status) = status
            && !STATUSES.contains(&status)
        {
            return Err(format!(
                "unknown status {status}, expected one of {}",
                STATUSES.join(", ")
            ));
        }
        let issue = self
            .issues
            .iter_mut()
            .find(|issue| issue.id == id)
            .ok_or_else(|| format!("unknown issue {id}"))?;
        if let Some(status) = status {
            issue.status = status.to_owned();
        }
        if let Some(note) = note {
            issue.notes.push(note.to_owned());
        }
        issue.updated_ms = now_ms;
        let issue = issue.clone();
        self.persist();
        Ok(issue)
    }

    pub fn attach(&mut self, id: u64, evidence: Value, now_ms: u64) -> Result<Issue, String> {
        let issue = self
            .issues
            .iter_mut()
            .find(|issue| issue.id == id)
            .ok_or_else(|| format!("unknown issue {id}"))?;
        issue.evidence.push(evidence);
        issue.updated_ms = now_ms;
        let issue = issue.clone();
        self.persist();
        Ok(issue)
    }

    /// The flat feed served at `/api/issues.json` and returned by `issue.export`.
    pub fn export(&self, now_ms: u64) -> Value {
        json!({
            "schema": SCHEMA,
            "generated_ms": now_ms,
            "count": self.issues.len(),
            "open": self.issues.iter().filter(|i| i.status == OPEN).count(),
            "issues": self.issues,
        })
    }

    /// Write the log beside the daemon's data. Best effort: a read-only or full
    /// disk must not turn filing a bug report into a failed operation.
    fn persist(&self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let payload = Persisted {
            next_id: self.next_id,
            issues: self.issues.clone(),
        };
        let Ok(text) = serde_json::to_string_pretty(&payload) else {
            return;
        };
        write_atomic(path, &text);
    }
}

fn write_atomic(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let temporary = path.with_extension("json.tmp");
    if std::fs::write(&temporary, text).is_ok() {
        let _ = std::fs::rename(&temporary, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(title: &str, reporter: &str) -> NewIssue {
        NewIssue {
            title: title.to_owned(),
            reporter: Some(reporter.to_owned()),
            ..NewIssue::default()
        }
    }

    fn temp_path(tag: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!("lp-issues-{tag}-{unique}/issues.json"))
    }

    // A retry loop must add weight to one report rather than bury the queue in
    // near-duplicates, and priority follows distinct reporters, not raw count.
    #[test]
    fn a_repeated_sighting_adds_weight_instead_of_filing_twice() {
        let mut store = IssueStore::new(None);
        let (first, filed) = store.file(report("acquire times out at 200MHz", "agent-a"), 10);
        assert_eq!(filed, Filed::New);
        assert_eq!(first.sightings, 1);
        let (again, filed) = store.file(report("acquire times out at 200MHz", "agent-b"), 20);
        assert_eq!(filed, Filed::Duplicate);
        assert_eq!(store.len(), 1, "one bug is one report");
        assert_eq!(again.sightings, 2);
        assert_eq!(
            again.reporters,
            ["agent-a", "agent-b"],
            "distinct reporters"
        );
        // The same reporter sighting it again is weight, not a new reporter.
        let (third, _) = store.file(report("acquire times out at 200MHz", "agent-b"), 30);
        assert_eq!(third.sightings, 3);
        assert_eq!(third.reporters.len(), 2);
    }

    // The loudest signal in the set: something marked fixed is happening again.
    #[test]
    fn a_sighting_after_resolution_reopens_as_a_regression() {
        let mut store = IssueStore::new(None);
        let (issue, _) = store.file(report("decode returns no frames", "agent-a"), 10);
        store
            .update(issue.id, Some(RESOLVED), None, 20)
            .unwrap_or_else(|error| panic!("{error}"));
        let (reopened, filed) = store.file(report("decode returns no frames", "agent-c"), 30);
        assert_eq!(filed, Filed::Regression);
        assert_eq!(reopened.status, OPEN, "a regression reopens the report");
        assert_eq!(
            reopened.id, issue.id,
            "it is the same report, not a new one"
        );
    }

    #[test]
    fn matching_ignores_case_and_punctuation_but_separates_by_tool() {
        let mut store = IssueStore::new(None);
        store.file(report("Acquire  TIMES-out!", "a"), 1);
        let (_, filed) = store.file(report("acquire times out", "b"), 2);
        assert_eq!(filed, Filed::Duplicate, "wording noise is the same bug");
        let mut tooled = NewIssue {
            title: "acquire times out".to_owned(),
            tool: Some("acq.single".to_owned()),
            ..NewIssue::default()
        };
        let (_, filed) = store.file(tooled.clone(), 3);
        assert_eq!(filed, Filed::New, "a different tool is a different report");
        tooled.tool = Some("ACQ.SINGLE".to_owned());
        let (_, filed) = store.file(tooled, 4);
        assert_eq!(filed, Filed::Duplicate, "tool match is case-insensitive");
    }

    // Reports outlive the process they describe; a restart must not lose them.
    #[test]
    fn the_log_round_trips_through_its_file() {
        let path = temp_path("roundtrip");
        {
            let mut store = IssueStore::new(Some(path.clone()));
            store.file(report("first", "a"), 10);
            store.file(report("second", "b"), 20);
        }
        let reloaded = IssueStore::new(Some(path.clone()));
        assert_eq!(reloaded.len(), 2, "issues survive a restart");
        let titles: Vec<String> = reloaded
            .list(None, 10)
            .into_iter()
            .map(|i| i.title)
            .collect();
        assert_eq!(titles, ["second", "first"], "newest first");
        // Ids continue rather than colliding with a reloaded report.
        let mut reloaded = reloaded;
        let (third, _) = reloaded.file(report("third", "c"), 30);
        assert_eq!(third.id, 3);
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new("/nonexistent")));
    }

    #[test]
    fn a_missing_or_unreadable_log_starts_empty_rather_than_failing() {
        let store = IssueStore::new(Some(temp_path("absent")));
        assert!(store.is_empty(), "a missing log is an empty log");
    }

    #[test]
    fn update_rejects_an_unknown_status_and_an_unknown_id() {
        let mut store = IssueStore::new(None);
        let (issue, _) = store.file(report("a bug", "a"), 1);
        assert!(store.update(issue.id, Some("wontfix"), None, 2).is_err());
        assert!(store.update(9999, Some(RESOLVED), None, 2).is_err());
        let updated = store
            .update(issue.id, Some(ACKNOWLEDGED), Some("triaged"), 3)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(updated.status, ACKNOWLEDGED);
        assert_eq!(updated.notes, ["triaged"]);
        assert_eq!(updated.updated_ms, 3);
    }

    #[test]
    fn list_filters_by_status_and_honours_the_limit() {
        let mut store = IssueStore::new(None);
        let (open_one, _) = store.file(report("one", "a"), 1);
        store.file(report("two", "a"), 2);
        store.file(report("three", "a"), 3);
        store
            .update(open_one.id, Some(RESOLVED), None, 4)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(store.list(Some(OPEN), 10).len(), 2);
        assert_eq!(store.list(Some(RESOLVED), 10).len(), 1);
        assert_eq!(store.list(None, 2).len(), 2, "limit caps the page");
    }

    #[test]
    fn evidence_attaches_to_the_named_issue_only() {
        let mut store = IssueStore::new(None);
        let (issue, _) = store.file(report("a bug", "a"), 1);
        store.file(report("another bug", "a"), 2);
        let updated = store
            .attach(issue.id, json!({"capture_id": 7}), 5)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(updated.evidence, [json!({"capture_id": 7})]);
        assert!(store.attach(9999, json!({}), 6).is_err());
        let other = store.get(issue.id + 1).unwrap_or_else(|| panic!("missing"));
        assert!(
            other.evidence.is_empty(),
            "evidence does not leak across reports"
        );
    }

    #[test]
    fn the_feed_carries_the_schema_and_open_count() {
        let mut store = IssueStore::new(None);
        let (one, _) = store.file(report("one", "a"), 1);
        store.file(report("two", "a"), 2);
        store
            .update(one.id, Some(RESOLVED), None, 3)
            .unwrap_or_else(|error| panic!("{error}"));
        let feed = store.export(99);
        assert_eq!(feed["schema"], SCHEMA);
        assert_eq!(feed["count"], 2);
        assert_eq!(feed["open"], 1);
        assert_eq!(feed["generated_ms"], 99);
        assert_eq!(feed["issues"].as_array().map(Vec::len), Some(2));
    }
}
