use crate::ToolError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    sync::OnceLock,
};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Rest {
    pub method: String,
    pub path: String,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct McpBinding {
    pub tool: String,
    pub op: String,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct OpSpec {
    pub id: String,
    pub title: String,
    pub area: String,
    pub origin: String,
    pub params: Value,
    pub result: Value,
    pub mutating: bool,
    pub ui: Vec<String>,
    pub shortcut: Option<String>,
    pub rest: Rest,
    pub mcp: McpBinding,
    pub truth: String,
}
#[derive(Deserialize)]
struct Catalog {
    count: usize,
    aliases: BTreeMap<String, String>,
    operations: Vec<OpSpec>,
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();
fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("catalog.json"))
            .unwrap_or_else(|error| panic!("invalid embedded operation catalog: {error}"))
    })
}
pub fn registry() -> &'static [OpSpec] {
    &catalog().operations
}
pub fn aliases() -> &'static BTreeMap<String, String> {
    &catalog().aliases
}
pub fn find(id: &str) -> Option<&'static OpSpec> {
    let canonical = catalog().aliases.get(id).map_or(id, String::as_str);
    registry().iter().find(|op| op.id == canonical)
}

pub trait Dispatcher {
    fn call(&self, op: &OpSpec, params: Value) -> Result<Value, ToolError>;
}
pub fn dispatch(ctx: &dyn Dispatcher, id: &str, params: Value) -> Result<Value, ToolError> {
    let op =
        find(id).ok_or_else(|| ToolError::new("unknown_op", format!("unknown operation: {id}")))?;
    ctx.call(op, params)
}
pub fn validate() -> Result<(), ToolError> {
    if registry().len() != catalog().count || registry().len() != 459 {
        return Err(ToolError::new(
            "op_count",
            format!("expected 459 operations, got {}", registry().len()),
        ));
    }
    let mut ids = HashSet::new();
    for op in registry() {
        if !ids.insert(&op.id) {
            return Err(ToolError::new("duplicate_op", op.id.clone()));
        }
        if !op.params.is_object() || !op.result.is_object() {
            return Err(ToolError::new("schema", op.id.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_has_exactly_459_unique_schema_bearing_ops() {
        validate().unwrap_or_else(|e| panic!("{e}"));
    }
    #[test]
    fn aliases_resolve_to_canonical_ops() {
        for (alias, target) in aliases() {
            assert_eq!(find(alias).map(|op| op.id.as_str()), Some(target.as_str()));
        }
    }
    #[test]
    fn origin_covers_all_menu_ids() {
        let excluded = [
            "mnuSeparator",
            "mnuDebug",
            "mnuService",
            "mnuTest",
            "mnuUpdates",
            "mnuCheckUpdates",
            "mnuPopup",
        ];
        let ids = include_str!("../../../../fixtures/vendor/ui_identifiers.txt");
        for menu in ids.lines().filter(|line| {
            line.starts_with("mnu") && !excluded.iter().any(|prefix| line.starts_with(prefix))
        }) {
            assert!(
                registry()
                    .iter()
                    .any(|op| op.origin.split(" | ").any(|origin| origin == menu)),
                "uncovered menu id: {menu}"
            );
        }
    }
}
