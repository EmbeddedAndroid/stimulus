use crate::{
    Capture, Column, Cursor, Group, Interpreter, MeasurementSlot, Row, Settings, Signal, ViewState,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const SCHEMA: &str = "lpj/1";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Native,
    LpfImport,
    Cli,
    Capture,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Source {
    pub kind: SourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub unknown_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    pub schema: String,
    pub version: u32,
    pub generator: String,
    pub created: String,
    pub modified: String,
    pub read_only: bool,
    pub source: Source,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub signals: Vec<Signal>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub interpreters: Vec<Interpreter>,
    #[serde(default)]
    pub rows: Vec<Row>,
    #[serde(default)]
    pub columns: Vec<Column>,
    #[serde(default)]
    pub view: ViewState,
    #[serde(default)]
    pub cursors: Vec<Cursor>,
    #[serde(default)]
    pub measurements: Vec<MeasurementSlot>,
    #[serde(default)]
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<Capture>,
    #[serde(flatten)]
    pub x_unknown: BTreeMap<String, Value>,
}

impl Project {
    pub fn new(timestamp: impl Into<String>) -> Self {
        let timestamp = timestamp.into();
        Self {
            schema: SCHEMA.to_owned(),
            version: VERSION,
            generator: format!("stimulus/{}", env!("CARGO_PKG_VERSION")),
            created: timestamp.clone(),
            modified: timestamp,
            read_only: false,
            source: Source {
                kind: SourceKind::Native,
                path: None,
                unknown_keys: Vec::new(),
            },
            settings: Settings::default(),
            signals: (0..34)
                .map(|wire| Signal {
                    wire,
                    wire_name: if wire < 32 {
                        format!("D{wire}")
                    } else {
                        format!("CLK{}", wire - 31)
                    },
                    name: if wire < 32 {
                        format!("D{wire}")
                    } else {
                        format!("CLK{}", wire - 31)
                    },
                })
                .collect(),
            groups: Vec::new(),
            interpreters: Vec::new(),
            rows: Vec::new(),
            columns: Vec::new(),
            view: ViewState::default(),
            cursors: ('A'..='F')
                .map(|id| Cursor {
                    id,
                    offset_samples: 0,
                    offset_s: 0.0,
                    tracks: None,
                    visible: false,
                })
                .collect(),
            measurements: (0..4)
                .map(|slot| MeasurementSlot {
                    slot,
                    kind: "interval".into(),
                    left: "trigger".into(),
                    right: "reference".into(),
                    source: "D0".into(),
                })
                .collect(),
            notes: String::new(),
            capture: None,
            x_unknown: BTreeMap::new(),
        }
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self, ProjectError> {
        let value: Value = serde_json::from_slice(bytes)?;
        let value = crate::migrate::migrate_value(value)?;
        let project: Self = serde_json::from_value(value)?;
        if project.schema != SCHEMA || project.version != VERSION {
            return Err(ProjectError::Unsupported {
                schema: project.schema,
                version: project.version,
            });
        }
        project.validate()?;
        Ok(project)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        Self::from_slice(&fs::read(path)?)
    }

    pub fn save_atomic(&self, path: impl AsRef<Path>) -> Result<(), ProjectError> {
        if self.read_only {
            return Err(ProjectError::ReadOnly);
        }
        self.validate()?;
        let path = path.as_ref();
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let tmp = temporary_path(path);
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let result = (|| -> Result<(), ProjectError> {
            serde_json::to_writer_pretty(&mut file, self)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            fs::rename(&tmp, path)?;
            OpenOptions::new().read(true).open(parent)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    pub fn validate(&self) -> Result<(), ProjectError> {
        for (field, got, expected) in [
            ("signals", self.signals.len(), 34),
            ("cursors", self.cursors.len(), 6),
            ("measurements", self.measurements.len(), 4),
            (
                "logic_sense.inverted",
                self.settings.logic_sense.inverted.len(),
                34,
            ),
            (
                "controls.selections",
                self.settings.controls.selections.len(),
                5,
            ),
            ("controls.values", self.settings.controls.values.len(), 5),
        ] {
            if got != expected {
                return Err(ProjectError::Cardinality {
                    field,
                    expected,
                    got,
                });
            }
        }
        if self.settings.sample.rate_index > 19
            || !(0.0..=100.0).contains(&self.settings.sample.pretrigger_pct)
        {
            return Err(ProjectError::Domain("sample settings"));
        }
        Ok(())
    }
}

pub fn schema_document() -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&schemars::schema_for!(Project)).map(|mut json| {
        json.push('\n');
        json
    })
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    path.with_file_name(name)
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("unsupported project schema {schema} version {version}")]
    Unsupported { schema: String, version: u32 },
    #[error("project is read-only")]
    ReadOnly,
    #[error("project root must be a JSON object")]
    InvalidRoot,
    #[error("{field} requires {expected} entries, got {got}")]
    Cardinality {
        field: &'static str,
        expected: usize,
        got: usize,
    },
    #[error("value outside project domain: {0}")]
    Domain(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    #[test]
    fn schema_round_trip_preserves_unknown_top_level_fields() {
        let mut project = Project::new("2026-08-25T00:00:00-04:00");
        project
            .x_unknown
            .insert("future".into(), serde_json::json!({"a": 1}));
        let bytes = serde_json::to_vec(&project).unwrap_or_else(|e| panic!("{e}"));
        let decoded = Project::from_slice(&bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(decoded, project);
    }
    #[test]
    fn rejects_cardinality_and_domain_drift() {
        let mut project = Project::new("2026-08-25T00:00:00-04:00");
        project.cursors.pop();
        assert!(matches!(
            project.validate(),
            Err(ProjectError::Cardinality {
                field: "cursors",
                ..
            })
        ));
        project = Project::new("2026-08-25T00:00:00-04:00");
        project.settings.sample.rate_index = 20;
        assert!(matches!(
            project.validate(),
            Err(ProjectError::Domain("sample settings"))
        ));
    }
    #[test]
    fn migrates_v0_recursively_and_preserves_extensions() {
        let legacy = serde_json::json!({
            "schema":"lpj/0", "version":0, "created":"2020-01-01T00:00:00Z",
            "notes":"legacy", "settings":{"threshold_v":2.5}, "vendor_extension":{"answer":42}
        });
        let project =
            Project::from_slice(&serde_json::to_vec(&legacy).unwrap_or_else(|e| panic!("{e}")))
                .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(project.schema, SCHEMA);
        assert_eq!(project.notes, "legacy");
        assert_eq!(project.settings.threshold_v, 2.5);
        assert_eq!(
            project.x_unknown.get("vendor_extension"),
            Some(&serde_json::json!({"answer":42}))
        );
        assert_eq!(project.signals.len(), 34);
    }
    #[test]
    fn atomic_save_load_is_stable() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let path = std::env::temp_dir().join(format!("lp-project-{nonce}.lpj"));
        let project = Project::new("2026-08-25T00:00:00-04:00");
        project.save_atomic(&path).unwrap_or_else(|e| panic!("{e}"));
        let loaded = Project::load(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(loaded, project);
        std::fs::remove_file(path).unwrap_or_else(|e| panic!("{e}"));
    }
}
