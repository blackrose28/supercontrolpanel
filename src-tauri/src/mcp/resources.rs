use serde_json::{json, Value};
use tauri::AppHandle;

use crate::{
    mcp::{call_mcp_tool, McpToolCallEnvelope},
    services::ConfigPaths,
};
use scp_core::mcp_resources::resolve_mcp_resource;

pub use scp_core::mcp_resources::list_mcp_resources;

pub fn read_mcp_resource(
    app: &AppHandle,
    uri: &str,
    now_ms: fn() -> u64,
    openvpn_function_id: &str,
    boundary_auth_function_id: &str,
    config_paths: fn(&AppHandle) -> Result<ConfigPaths, String>,
) -> Result<McpToolCallEnvelope, String> {
    dispatch_mcp_resource(uri, |tool_name, args| {
        call_mcp_tool(
            app,
            tool_name,
            args,
            now_ms,
            openvpn_function_id,
            boundary_auth_function_id,
            config_paths,
        )
    })
}

fn dispatch_mcp_resource(
    uri: &str,
    dispatch: impl FnOnce(&str, Value) -> Result<McpToolCallEnvelope, String>,
) -> Result<McpToolCallEnvelope, String> {
    let tool_name =
        resolve_mcp_resource(uri).ok_or_else(|| format!("unknown MCP resource: {uri}"))?;
    dispatch(tool_name, json!({}))
}

#[cfg(test)]
mod contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_resources.rs"
    ));
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn resource_adapter_dispatches_once_with_empty_arguments_and_preserves_result() {
        let resources: Vec<scp_core::mcp_resources::McpResourceDescriptor> = list_mcp_resources();
        for resource in resources {
            let calls = Cell::new(0);
            let result = dispatch_mcp_resource(&resource.uri, |tool, args| {
                calls.set(calls.get() + 1);
                assert_eq!(
                    Some(tool),
                    scp_core::mcp_resources::resolve_mcp_resource(&resource.uri)
                );
                assert_eq!(args, json!({}));
                Ok(McpToolCallEnvelope {
                    ok: true,
                    message: "unchanged".to_owned(),
                    data: json!({"value": 7}),
                    warnings: vec!["warning".to_owned()],
                    timestamp: 42,
                })
            })
            .unwrap();
            assert_eq!(calls.get(), 1);
            assert_eq!(
                serde_json::to_value(result).unwrap(),
                json!({"ok": true, "message": "unchanged", "data": {"value": 7}, "warnings": ["warning"], "timestamp": 42})
            );
            assert!(
                matches!(dispatch_mcp_resource(&resource.uri, |_, _| Err("dispatcher denied".to_owned())), Err(error) if error == "dispatcher denied")
            );
        }
    }

    #[test]
    fn unknown_resource_preserves_error_and_never_dispatches() {
        for uri in ["", "scp://unknown", "scp://status/functions\n"] {
            let result = dispatch_mcp_resource(uri, |_, _| panic!("unknown resource dispatched"));
            assert!(
                matches!(result, Err(error) if error == format!("unknown MCP resource: {uri}"))
            );
        }
    }

    #[test]
    fn production_resource_reader_uses_guarded_tool_dispatcher() {
        // Source wiring only, not an HTTP/Tauri or handler execution test.
        assert!(include_str!("resources.rs").contains(concat!(
            "dispatch_mcp_resource(uri, |tool_name, args| {\n",
            "        call_mcp_tool(\n",
            "            app,\n",
            "            tool_name,\n",
            "            args,"
        )));
    }
}
