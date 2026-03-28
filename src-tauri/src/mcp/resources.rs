use serde_json::json;
use tauri::AppHandle;

use crate::{
    mcp::{call_mcp_tool, McpResourceDescriptor, McpToolCallEnvelope},
    services::ConfigPaths,
};

pub fn list_mcp_resources() -> Vec<McpResourceDescriptor> {
    vec![
        McpResourceDescriptor {
            uri: "scp://status/functions".to_string(),
            title: "Function statuses".to_string(),
            category: "status".to_string(),
        },
        McpResourceDescriptor {
            uri: "scp://status/openvpn".to_string(),
            title: "OpenVPN status".to_string(),
            category: "status".to_string(),
        },
        McpResourceDescriptor {
            uri: "scp://status/boundary".to_string(),
            title: "Boundary status".to_string(),
            category: "status".to_string(),
        },
        McpResourceDescriptor {
            uri: "scp://servers/list".to_string(),
            title: "Servers list".to_string(),
            category: "inventory".to_string(),
        },
        McpResourceDescriptor {
            uri: "scp://config/paths".to_string(),
            title: "Config paths".to_string(),
            category: "config".to_string(),
        },
    ]
}

pub fn read_mcp_resource(
    app: &AppHandle,
    uri: &str,
    now_ms: fn() -> u64,
    openvpn_function_id: &str,
    boundary_auth_function_id: &str,
    config_paths: fn(&AppHandle) -> Result<ConfigPaths, String>,
) -> Result<McpToolCallEnvelope, String> {
    match uri {
        "scp://status/functions" => call_mcp_tool(
            app,
            "function_status_list",
            json!({}),
            now_ms,
            openvpn_function_id,
            boundary_auth_function_id,
            config_paths,
        ),
        "scp://status/openvpn" => call_mcp_tool(
            app,
            "openvpn_get_status",
            json!({}),
            now_ms,
            openvpn_function_id,
            boundary_auth_function_id,
            config_paths,
        ),
        "scp://status/boundary" => call_mcp_tool(
            app,
            "boundary_get_status",
            json!({}),
            now_ms,
            openvpn_function_id,
            boundary_auth_function_id,
            config_paths,
        ),
        "scp://servers/list" => call_mcp_tool(
            app,
            "servers_list",
            json!({}),
            now_ms,
            openvpn_function_id,
            boundary_auth_function_id,
            config_paths,
        ),
        "scp://config/paths" => call_mcp_tool(
            app,
            "config_get_paths",
            json!({}),
            now_ms,
            openvpn_function_id,
            boundary_auth_function_id,
            config_paths,
        ),
        _ => Err(format!("unknown MCP resource: {uri}")),
    }
}
