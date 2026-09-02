//! Exhaustive cross-surface operation coverage checks live in this crate's tests.

#[cfg(test)]
mod tests {
    use lp_core::{
        ToolError,
        ops::{self, Dispatcher},
    };
    use lp_mcp::{LeaseMode, McpServer};
    use serde_json::{Value, json};
    use std::collections::BTreeSet;

    struct Echo;

    impl Dispatcher for Echo {
        fn call(&self, op: &ops::OpSpec, params: Value) -> Result<Value, ToolError> {
            Ok(json!({"op":op.id,"params":params}))
        }
    }

    fn ids_from_inventory() -> BTreeSet<String> {
        include_str!("../../../docs/FEATURE-INVENTORY.md")
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix("| `")?;
                let (id, _) = rest.split_once('`')?;
                Some(id.to_owned())
            })
            .collect()
    }

    #[test]
    fn inventory_and_registry_are_identical() {
        let registry = ops::registry()
            .iter()
            .map(|op| op.id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(registry.len(), 465);
        assert_eq!(ids_from_inventory(), registry);
    }

    #[test]
    fn every_operation_dispatches_through_its_rest_binding() {
        for op in ops::registry() {
            assert_eq!(
                op.rest.method, "POST",
                "{} has a non-call REST method",
                op.id
            );
            assert_eq!(op.rest.path, format!("/api/ops/{}", op.id));
            let result = ops::dispatch(&Echo, &op.id, json!({"surface":"rest"}))
                .unwrap_or_else(|error| panic!("REST dispatch failed for {}: {error}", op.id));
            assert_eq!(result["op"], op.id);
            assert_eq!(result["params"]["surface"], "rest");
        }
    }

    #[test]
    fn every_operation_dispatches_through_its_mcp_binding() {
        let server = McpServer::new(LeaseMode::Auto);
        for op in ops::registry() {
            assert_eq!(op.mcp.tool, "op_call");
            assert_eq!(op.mcp.op, op.id);
            let response = server.handle(
                &Echo,
                json!({
                    "jsonrpc":"2.0",
                    "id":op.id,
                    "method":"tools/call",
                    "params":{
                        "name":"op_call",
                        "arguments":{"op":op.id,"params":{"surface":"mcp"}}
                    }
                }),
            );
            assert_eq!(
                response["result"]["isError"], false,
                "MCP failed for {}",
                op.id
            );
            assert_eq!(response["result"]["structuredContent"]["op"], op.id);
            assert_eq!(
                response["result"]["structuredContent"]["params"]["surface"],
                "mcp"
            );
        }
    }

    #[test]
    fn every_operation_is_in_the_web_palette_catalog() {
        let web: Value = serde_json::from_str(include_str!("../../../web/src/generated/ops.json"))
            .unwrap_or_else(|error| panic!("invalid Web operation catalog: {error}"));
        let web_ops = web["operations"]
            .as_array()
            .unwrap_or_else(|| panic!("Web operation catalog has no operations array"));
        assert_eq!(web_ops.len(), 465);
        let web_ids = web_ops
            .iter()
            .map(|op| {
                assert_eq!(op["ui"], json!(["Palette"]));
                op["id"]
                    .as_str()
                    .unwrap_or_else(|| panic!("Web operation missing id"))
                    .to_owned()
            })
            .collect::<BTreeSet<_>>();
        let registry = ops::registry().iter().map(|op| op.id.clone()).collect();
        assert_eq!(web_ids, registry);

        let palette_source = include_str!("../../../web/src/command-palette.tsx");
        assert!(palette_source.contains("matches.map((operation)"));
        assert!(palette_source.contains("onInvoke(selected.id, parsed)"));
    }
}
