use crate::{protocol, resources};
use lp_core::ops::{self, Dispatcher};
use serde_json::{Value, json};
use std::{
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const INSTRUCTIONS: &str = "Call device_status first. Mutating tools require a lease when leasing is enabled. Page results using next_cursor. Read freshness.capture_id before acting on capture data. Use acquire_wait instead of polling. Branch on structured error.code.";

/// Mutation-lease lifetime. Each authorized use renews it, so an active client
/// keeps its lease; a client that acquires one and then vanishes (crash, lost
/// connection -- stateless HTTP gives no close signal) only holds it until this
/// elapses, after which the next `lease_acquire` reclaims it without a steal.
/// Kept short so an abandoned lease self-heals quickly instead of blocking other
/// clients for minutes.
const LEASE_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseMode {
    Auto,
    Required,
}

#[derive(Debug)]
struct LeaseState {
    token: Option<String>,
    expires: Instant,
    next: u64,
}

#[derive(Debug)]
pub struct McpServer {
    mode: LeaseMode,
    lease: Mutex<LeaseState>,
}

impl Default for McpServer {
    fn default() -> Self {
        let mode = match std::env::var("LP_MCP_LEASE").as_deref() {
            Ok("required") => LeaseMode::Required,
            _ => LeaseMode::Auto,
        };
        Self::new(mode)
    }
}

impl McpServer {
    pub fn new(mode: LeaseMode) -> Self {
        Self {
            mode,
            lease: Mutex::new(LeaseState {
                token: None,
                expires: Instant::now(),
                next: 1,
            }),
        }
    }

    pub fn handle(&self, dispatcher: &dyn Dispatcher, request: Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return protocol::failure(id, -32600, "invalid JSON-RPC request", None);
        }
        let method = match request.get("method").and_then(Value::as_str) {
            Some(method) => method,
            None => return protocol::failure(id, -32600, "method is required", None),
        };
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion":protocol::PROTOCOL_VERSION,
                "capabilities":{"tools":{"listChanged":false},"resources":{"subscribe":true,"listChanged":true}},
                "serverInfo":{"name":"logicport","version":env!("CARGO_PKG_VERSION")},
                "instructions":INSTRUCTIONS
            })),
            "ping" => Ok(json!({})),
            "notifications/initialized" | "notifications/cancelled" => return Value::Null,
            "tools/list" => Ok(tools_list()),
            "tools/call" => return protocol::success(id, self.call_tool(dispatcher, &params)),
            "resources/list" => Ok(resources::list()),
            "resources/read" => params
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| lp_core::ToolError::new("INVALID_ARG", "uri is required"))
                .and_then(|uri| resources::read(dispatcher, uri)),
            "resources/subscribe" => Ok(json!({})),
            "prompts/list" => Ok(json!({"prompts":[]})),
            _ => return protocol::failure(id, -32601, format!("method not found: {method}"), None),
        };
        match result {
            Ok(result) => protocol::success(id, result),
            Err(error) => protocol::failure(
                id,
                -32602,
                error.message.clone(),
                Some(json!({"error":error})),
            ),
        }
    }

    fn call_tool(&self, dispatcher: &dyn Dispatcher, params: &Value) -> Value {
        let name = match params.get("name").and_then(Value::as_str) {
            Some(name) => name,
            None => {
                return protocol::tool_error(lp_core::ToolError::new(
                    "INVALID_ARG",
                    "tool name is required",
                ));
            }
        };
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if name == "lease_acquire" {
            return match self.acquire_lease(
                arguments
                    .get("steal")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ) {
                Ok(value) => protocol::tool_result(value),
                Err(error) => protocol::tool_error(error),
            };
        }
        if name == "lease_release" {
            return match self.release_lease(arguments.get("lease").and_then(Value::as_str)) {
                Ok(value) => protocol::tool_result(value),
                Err(error) => protocol::tool_error(error),
            };
        }
        let resolved = resolve_tool(name, &arguments);
        let (op, op_params) = match resolved {
            Ok(value) => value,
            Err(error) => return protocol::tool_error(error),
        };
        let lease = if is_mutating(op) {
            match self.authorize(arguments.get("lease").and_then(Value::as_str)) {
                Ok(token) => Some(token),
                Err(error) => return protocol::tool_error(error),
            }
        } else {
            None
        };
        match dispatcher.call(op, op_params) {
            Ok(mut value) => {
                if let Some(token) = lease {
                    if let Value::Object(object) = &mut value {
                        object.insert("lease".into(), Value::String(token));
                    } else {
                        value = json!({"value":value,"lease":token});
                    }
                } else {
                    attach_freshness(&mut value);
                }
                match cap_response(value) {
                    Ok(value) => protocol::tool_result(value),
                    Err(error) => protocol::tool_error(error),
                }
            }
            Err(error) => protocol::tool_error(error),
        }
    }

    fn authorize(&self, supplied: Option<&str>) -> Result<String, lp_core::ToolError> {
        self.authorize_at(supplied, Instant::now())
    }

    fn authorize_at(
        &self,
        supplied: Option<&str>,
        now: Instant,
    ) -> Result<String, lp_core::ToolError> {
        let mut lease = self
            .lease
            .lock()
            .map_err(|_| lp_core::ToolError::new("INTERNAL", "lease lock poisoned"))?;
        // Reclaim an expired lease before matching, so an abandoned lease never
        // blocks a new owner past its TTL.
        if lease.token.is_some() && now >= lease.expires {
            lease.token = None;
        }
        match (&lease.token, supplied) {
            (Some(current), Some(value)) if current == value => {
                let token = current.clone();
                lease.expires = now + LEASE_TTL;
                Ok(token)
            }
            (Some(_), Some(_)) => Err(lp_core::ToolError::new(
                "LEASE_HELD",
                "lease belongs to another client",
            )),
            (Some(current), None) if self.mode == LeaseMode::Auto => {
                let token = current.clone();
                lease.expires = now + LEASE_TTL;
                Ok(token)
            }
            (Some(_), None) | (None, None) if self.mode == LeaseMode::Required => Err(
                lp_core::ToolError::new("LEASE_REQUIRED", "mutating MCP tools require a lease")
                    .with_hint("call lease_acquire, then pass its lease token"),
            ),
            (Some(_), None) => Err(lp_core::ToolError::new(
                "LEASE_REQUIRED",
                "mutating MCP tools require the active lease token",
            )),
            (None, Some(_)) => Err(lp_core::ToolError::new(
                "LEASE_HELD",
                "lease token is invalid or expired",
            )),
            (None, None) => self.allocate(&mut lease, now),
        }
    }

    fn acquire_lease(&self, steal: bool) -> Result<Value, lp_core::ToolError> {
        self.acquire_lease_at(steal, Instant::now())
    }

    fn acquire_lease_at(&self, steal: bool, now: Instant) -> Result<Value, lp_core::ToolError> {
        let mut lease = self
            .lease
            .lock()
            .map_err(|_| lp_core::ToolError::new("INTERNAL", "lease lock poisoned"))?;
        if lease.token.is_some() && now < lease.expires && !steal {
            return Err(lp_core::ToolError::new(
                "LEASE_HELD",
                "an active lease already exists",
            ));
        }
        self.allocate(&mut lease, now)
            .map(|token| json!({"lease":token,"ttl_s":LEASE_TTL.as_secs()}))
    }

    fn allocate(&self, lease: &mut LeaseState, now: Instant) -> Result<String, lp_core::ToolError> {
        let token = format!("lp-mcp-{}", lease.next);
        lease.next = lease
            .next
            .checked_add(1)
            .ok_or_else(|| lp_core::ToolError::new("INTERNAL", "lease sequence exhausted"))?;
        lease.token = Some(token.clone());
        lease.expires = now + LEASE_TTL;
        Ok(token)
    }

    fn release_lease(&self, supplied: Option<&str>) -> Result<Value, lp_core::ToolError> {
        let mut lease = self
            .lease
            .lock()
            .map_err(|_| lp_core::ToolError::new("INTERNAL", "lease lock poisoned"))?;
        match (lease.token.as_deref(), supplied) {
            (Some(current), Some(value)) if current == value => {
                lease.token = None;
                Ok(json!({"released":true}))
            }
            _ => Err(lp_core::ToolError::new(
                "LEASE_HELD",
                "lease token is invalid or expired",
            )),
        }
    }
}

pub fn handle(dispatcher: &dyn Dispatcher, request: Value) -> Value {
    McpServer::default().handle(dispatcher, request)
}

fn tools_list() -> Value {
    let mut tools = vec![
        json!({
            "name":"op_call",
            "description":"Call any registered LogicPort operation by canonical operation ID.",
            "inputSchema":{"type":"object","required":["op"],"properties":{"op":{"type":"string"},"params":{"type":"object"},"lease":{"type":"string"}}}
        }),
        json!({
            "name":"lease_acquire",
            "description":"Acquire the mutation lease. It renews on each use and an abandoned lease is reclaimed after its idle TTL.",
            "inputSchema":{"type":"object","properties":{"steal":{"type":"boolean","default":false}}}
        }),
        json!({
            "name":"lease_release",
            "description":"Release the mutation lease.",
            "inputSchema":{"type":"object","required":["lease"],"properties":{"lease":{"type":"string"}}}
        }),
    ];
    tools.extend(ops::registry().iter().map(|op| {
        json!({
            "name":op.id.replace('.', "_"),
            "description":format!("{} ({})", op.title, op.id),
            "inputSchema":op.params
        })
    }));
    tools.extend(tool_aliases().iter().filter_map(|(name, op_id)| {
        ops::find(op_id).map(|op| {
            json!({
                "name":name,
                "description":format!("{} ({})", op.title, op.id),
                "inputSchema":op.params
            })
        })
    }));
    json!({"tools":tools})
}

fn resolve_tool(
    name: &str,
    arguments: &Value,
) -> Result<(&'static ops::OpSpec, Value), lp_core::ToolError> {
    if name == "op_call" {
        let op = match arguments.get("op").and_then(Value::as_str) {
            Some(op) => op,
            None => return Err(lp_core::ToolError::new("INVALID_ARG", "op is required")),
        };
        let op_params = arguments
            .get("params")
            .cloned()
            .unwrap_or_else(|| json!({}));
        ops::find(op).map(|spec| (spec, op_params)).ok_or_else(|| {
            lp_core::ToolError::new("UNKNOWN_OP", format!("unknown operation: {op}"))
        })
    } else {
        let canonical = tool_aliases()
            .iter()
            .find_map(|(alias, op)| (*alias == name).then_some(*op));
        ops::registry()
            .iter()
            .find(|op| canonical == Some(op.id.as_str()) || op.id.replace('.', "_") == name)
            .ok_or_else(|| lp_core::ToolError::new("UNKNOWN_TOOL", format!("unknown tool: {name}")))
            .map(|op| (op, arguments.clone()))
    }
}

fn is_mutating(op: &ops::OpSpec) -> bool {
    op.mutating
        || matches!(
            op.id.as_str(),
            "project.put"
                | "project.import_lpf"
                | "project.export"
                | "project.from_capture"
                | "capture.export"
                | "capture.delete"
                | "capture.pin"
                | "stimulus.program"
                | "verify.run"
        )
}

const RESPONSE_BUDGET: usize = 512 * 1024;
const VALUE_BUDGET: usize = RESPONSE_BUDGET - 1024;

fn attach_freshness(value: &mut Value) {
    let capture_id = value
        .get("capture_id")
        .and_then(Value::as_u64)
        .or_else(|| value.pointer("/capture/id").and_then(Value::as_u64));
    let observed_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let freshness = json!({
        "observed_at_unix_ms":observed_at_unix_ms,
        "device_epoch":value.get("device_epoch").and_then(Value::as_u64).unwrap_or(1),
        "capture_id":capture_id
    });
    if let Value::Object(object) = value {
        object.insert("freshness".into(), freshness);
    } else {
        *value = json!({"value":value.take(),"freshness":freshness});
    }
}

fn cap_response(mut value: Value) -> Result<Value, lp_core::ToolError> {
    if encoded_len(&value)? <= VALUE_BUDGET {
        return Ok(value);
    }
    let mut removed = 0_usize;
    loop {
        let removed_now = ["captures", "rows", "matches", "events", "frames", "runs"]
            .iter()
            .find_map(|field| truncate_rows(&mut value, field))
            .unwrap_or(0);
        if removed_now == 0 {
            return Err(lp_core::ToolError::new(
                "RESULT_CAPPED",
                "result exceeds 512 KiB and has no pageable row collection",
            )
            .with_hint("request a smaller limit or narrower capture range"));
        }
        removed = removed.saturating_add(removed_now);
        if encoded_len(&value)? <= VALUE_BUDGET {
            if let Value::Object(object) = &mut value {
                object.insert("capped".into(), Value::Bool(true));
                object.insert("rows_omitted".into(), json!(removed));
            }
            if encoded_len(&value)? <= VALUE_BUDGET {
                return Ok(value);
            }
        }
    }
}

fn truncate_rows(value: &mut Value, field: &str) -> Option<usize> {
    if let Some(array) = value.get_mut(field).and_then(Value::as_array_mut) {
        let removed = (array.len() / 4).max(1).min(array.len());
        array.truncate(array.len() - removed);
        return Some(removed);
    }
    if let Some(array) = value
        .get_mut("capture")
        .and_then(|capture| capture.get_mut(field))
        .and_then(Value::as_array_mut)
    {
        let removed = (array.len() / 4).max(1).min(array.len());
        array.truncate(array.len() - removed);
        return Some(removed);
    }
    None
}

fn encoded_len(value: &Value) -> Result<usize, lp_core::ToolError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| lp_core::ToolError::new("INTERNAL", error.to_string()))
}

fn tool_aliases() -> &'static [(&'static str, &'static str)] {
    &[
        ("device_list", "device.enumerate"),
        ("acquire_single", "acq.single"),
        ("acquire_recurring_start", "acq.recurring.start"),
        ("acquire_recurring_stop", "acq.recurring.stop"),
        ("acquire_halt", "acq.halt"),
        ("acquire_status", "acq.status"),
        ("acquire_wait", "acq.wait"),
        ("acquire_trigger_immediate", "acq.trigger_immediate"),
        ("capture_list", "capture.list"),
        ("capture_get", "capture.get"),
        ("capture_summary", "capture.summary"),
        ("capture_export", "capture.export"),
        ("capture_search", "capture.search"),
        ("capture_diff", "capture.diff"),
        ("capture_measure", "capture.measure"),
        ("capture_state_list", "capture.state_list"),
        ("project_get", "project.get"),
        ("project_put", "project.put"),
        ("project_notes", "project.notes"),
        ("ops_list", "meta.ops_list"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Echo;
    impl Dispatcher for Echo {
        fn call(&self, op: &ops::OpSpec, params: Value) -> Result<Value, lp_core::ToolError> {
            Ok(json!({"op":op.id,"params":params}))
        }
    }
    #[test]
    fn initialize_and_tools_conform_to_protocol() {
        let initialized = handle(
            &Echo,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        );
        assert_eq!(
            initialized["result"]["protocolVersion"],
            protocol::PROTOCOL_VERSION
        );
        let listed = handle(&Echo, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
        assert!(
            listed["result"]["tools"]
                .as_array()
                .is_some_and(|tools| tools.len() > 460)
        );
        let called = handle(
            &Echo,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"capture_summary","arguments":{"capture_id":4}}}),
        );
        assert_eq!(
            called["result"]["structuredContent"]["op"],
            "capture.summary"
        );
        assert!(called["result"]["structuredContent"]["freshness"].is_object());
        let called = handle(
            &Echo,
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"acquire_single","arguments":{}}}),
        );
        assert_eq!(called["result"]["structuredContent"]["op"], "acq.single");
        assert!(called["result"]["structuredContent"]["lease"].is_string());
    }

    // Regression: a client that acquires a lease and then vanishes (crash / lost
    // connection -- stateless HTTP has no close signal) must not block other
    // clients forever. The abandoned lease is reclaimed by the next acquire once
    // its TTL has elapsed, without needing a steal.
    #[test]
    fn expired_lease_is_reclaimed_by_the_next_acquire_without_steal() {
        let server = McpServer::new(LeaseMode::Required);
        let t0 = Instant::now();
        let first = server.acquire_lease_at(false, t0);
        let token_a = match first {
            Ok(ref value) => value["lease"].as_str().map(str::to_owned),
            Err(_) => None,
        };
        assert!(token_a.is_some(), "first acquire should succeed");
        // While the TTL is unexpired, another client cannot take it without steal.
        assert!(
            server
                .acquire_lease_at(false, t0 + Duration::from_secs(1))
                .is_err(),
            "an unexpired lease must not be reclaimable without a steal"
        );
        // Once the TTL elapses the abandoned lease is reclaimed automatically.
        match server.acquire_lease_at(false, t0 + LEASE_TTL + Duration::from_secs(1)) {
            Ok(value) => assert_ne!(value["lease"].as_str().map(str::to_owned), token_a),
            Err(error) => panic!("expired lease must be reclaimable: {}", error.message),
        }
    }

    // Regression: an actively used lease is renewed on every authorized call, so
    // a client that keeps working never loses its lease to the shorter TTL.
    #[test]
    fn using_a_lease_renews_it_so_active_clients_do_not_expire() {
        let server = McpServer::new(LeaseMode::Required);
        let t0 = Instant::now();
        let token = match server.acquire_lease_at(false, t0) {
            Ok(value) => match value["lease"].as_str() {
                Some(token) => token.to_owned(),
                None => panic!("acquire returned no lease token"),
            },
            Err(error) => panic!("acquire failed: {}", error.message),
        };
        // Use it just before the original expiry; that renews it for another TTL.
        assert!(
            server
                .authorize_at(Some(&token), t0 + LEASE_TTL - Duration::from_secs(1))
                .is_ok()
        );
        // Now past the ORIGINAL expiry but within the renewed window: still valid.
        assert!(
            server
                .authorize_at(Some(&token), t0 + LEASE_TTL + Duration::from_secs(1))
                .is_ok(),
            "a lease used within its window must stay valid"
        );
    }

    #[test]
    fn every_registered_operation_is_callable_through_its_mcp_tool() {
        let server = McpServer::new(LeaseMode::Auto);
        let failures = ops::registry()
            .iter()
            .filter_map(|op| {
                let name = op.id.replace('.', "_");
                let response = server.handle(
                    &Echo,
                    json!({
                        "jsonrpc":"2.0",
                        "id":op.id,
                        "method":"tools/call",
                        "params":{"name":name,"arguments":{}}
                    }),
                );
                (response["result"]["isError"] != false
                    || response["result"]["structuredContent"]["op"] != op.id)
                    .then(|| op.id.clone())
            })
            .collect::<Vec<_>>();
        assert!(
            failures.is_empty(),
            "operations not callable through MCP: {failures:#?}"
        );
    }

    #[test]
    fn required_lease_is_acquired_renewed_checked_and_released() {
        let server = McpServer::new(LeaseMode::Required);
        let call = |id, name: &str, arguments: Value| {
            server.handle(
                &Echo,
                json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}}),
            )
        };
        let denied = call(1, "acquire_single", json!({}));
        assert_eq!(
            denied["result"]["structuredContent"]["error"]["code"],
            "LEASE_REQUIRED"
        );
        let acquired = call(2, "lease_acquire", json!({}));
        let token = acquired["result"]["structuredContent"]["lease"]
            .as_str()
            .unwrap_or_else(|| panic!("missing lease token"));
        let allowed = call(3, "acquire_single", json!({"lease":token}));
        assert_eq!(allowed["result"]["isError"], false);
        assert_eq!(allowed["result"]["structuredContent"]["lease"], token);
        let wrong = call(4, "acquire_single", json!({"lease":"wrong"}));
        assert_eq!(
            wrong["result"]["structuredContent"]["error"]["code"],
            "LEASE_HELD"
        );
        let released = call(5, "lease_release", json!({"lease":token}));
        assert_eq!(released["result"]["structuredContent"]["released"], true);
    }

    #[test]
    fn read_results_have_freshness_and_are_capped_below_512_kib() {
        struct Big;
        impl Dispatcher for Big {
            fn call(&self, _op: &ops::OpSpec, _params: Value) -> Result<Value, lp_core::ToolError> {
                Ok(json!({
                    "rows":(0..10_000).map(|index| json!({"index":index,"payload":"x".repeat(100)})).collect::<Vec<_>>()
                }))
            }
        }
        let response = McpServer::new(LeaseMode::Auto).handle(
            &Big,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"capture_state_list","arguments":{}}}),
        );
        let content = &response["result"]["structuredContent"];
        assert_eq!(content["capped"], true);
        assert!(content["freshness"].is_object());
        assert!(serde_json::to_vec(&response).is_ok_and(|bytes| bytes.len() <= RESPONSE_BUDGET));
        assert!(
            content["rows"]
                .as_array()
                .is_some_and(|rows| rows.len() < 10_000)
        );
    }
}
