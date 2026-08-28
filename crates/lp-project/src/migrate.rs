use crate::project::{Project, ProjectError, SCHEMA, VERSION};
use serde_json::{Map, Value};

pub fn migrate_value(mut source: Value) -> Result<Value, ProjectError> {
    let object = source.as_object_mut().ok_or(ProjectError::InvalidRoot)?;
    let schema = object.get("schema").and_then(Value::as_str);
    let version = object.get("version").and_then(Value::as_u64);
    match (schema, version) {
        (Some(SCHEMA), Some(value)) if value == u64::from(VERSION) => return Ok(source),
        (None | Some("lpj/0"), None | Some(0)) => {}
        (Some(other), value) => {
            return Err(ProjectError::Unsupported {
                schema: other.to_owned(),
                version: value.unwrap_or(0) as u32,
            });
        }
        (None, Some(value)) => {
            return Err(ProjectError::Unsupported {
                schema: "missing".into(),
                version: value as u32,
            });
        }
    }
    let timestamp = object
        .get("created")
        .and_then(Value::as_str)
        .unwrap_or("1970-01-01T00:00:00Z");
    let mut target = serde_json::to_value(Project::new(timestamp))?;
    merge(&mut target, source);
    let target = target.as_object_mut().ok_or(ProjectError::InvalidRoot)?;
    target.insert("schema".into(), Value::String(SCHEMA.into()));
    target.insert("version".into(), Value::from(VERSION));
    Ok(Value::Object(std::mem::take(target)))
}

fn merge(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Object(target), Value::Object(source)) => merge_objects(target, source),
        (target, source) => *target = source,
    }
}
fn merge_objects(target: &mut Map<String, Value>, source: Map<String, Value>) {
    for (key, value) in source {
        if matches!(key.as_str(), "schema" | "version") {
            continue;
        }
        match target.get_mut(&key) {
            Some(existing) => merge(existing, value),
            None => {
                target.insert(key, value);
            }
        }
    }
}
