use serde_json::{Value, json};

pub fn list() -> Value {
    json!({"resources":[
        {"uri":"lp://capture/latest","name":"Latest capture","mimeType":"application/json"},
        {"uri":"lp://project/current","name":"Current project","mimeType":"application/json"},
        {"uri":"lp://setup/current","name":"Current setup","mimeType":"application/json"},
        {"uri":"lp://docs/protocol","name":"LogicPort protocol","mimeType":"text/markdown"},
        {"uri":"lp://docs/feature-inventory","name":"Feature inventory","mimeType":"text/markdown"},
        {"uri":"lp://docs/operation-matrix","name":"Operation matrix","mimeType":"text/markdown"}
    ]})
}

pub fn read(
    dispatcher: &dyn lp_core::ops::Dispatcher,
    uri: &str,
) -> Result<Value, lp_core::ToolError> {
    let (mime_type, value) = match uri {
        "lp://capture/latest" => (
            "application/json",
            lp_core::ops::dispatch(dispatcher, "capture.get", json!({"capture_id":"latest"}))?,
        ),
        "lp://project/current" => (
            "application/json",
            lp_core::ops::dispatch(dispatcher, "project.get", json!({}))?,
        ),
        "lp://setup/current" => (
            "application/json",
            lp_core::ops::dispatch(dispatcher, "sample.get", json!({}))?,
        ),
        "lp://docs/protocol" => (
            "text/markdown",
            Value::String(include_str!("../../../docs/PROTOCOL.md").to_owned()),
        ),
        "lp://docs/feature-inventory" => (
            "text/markdown",
            Value::String(include_str!("../../../docs/FEATURE-INVENTORY.md").to_owned()),
        ),
        "lp://docs/operation-matrix" => (
            "text/markdown",
            Value::String(include_str!("../../../docs/OPERATION-MATRIX.md").to_owned()),
        ),
        _ => {
            return Err(lp_core::ToolError::new(
                "UNKNOWN_RESOURCE",
                format!("unknown resource: {uri}"),
            ));
        }
    };
    let text = match value {
        Value::String(text) => text,
        value => serde_json::to_string_pretty(&value)
            .map_err(|error| lp_core::ToolError::new("INTERNAL", error.to_string()))?,
    };
    Ok(json!({"contents":[{"uri":uri,"mimeType":mime_type,"text":text}]}))
}
