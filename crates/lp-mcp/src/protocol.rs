use serde::Serialize;
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

pub fn failure(id: Value, code: i32, message: impl Into<String>, data: Option<Value>) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":RpcError { code, message:message.into(), data }})
}

pub fn tool_result(value: Value) -> Value {
    let summary = match &value {
        Value::Object(object) => format!("Operation completed ({} fields).", object.len()),
        Value::Array(array) => format!("Operation completed ({} rows).", array.len()),
        _ => "Operation completed.".to_owned(),
    };
    json!({
        "content":[{"type":"text","text":summary}],
        "structuredContent":value,
        "isError":false
    })
}

pub fn tool_error(error: lp_core::ToolError) -> Value {
    json!({
        "content":[{"type":"text","text":error.message}],
        "structuredContent":{"error":error},
        "isError":true
    })
}
