//! Discovery and exact URI resolution for the five desktop MCP resources.
//!
//! Resource categories are descriptive labels, not execution-policy categories.
//! Resolution grants no authority: hosts must dispatch the returned tool through
//! policy, authentication and capability checks. This does not enable CLI resources.

use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceDescriptor {
    pub uri: String,
    pub title: String,
    pub category: String,
}

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

/// Resolve an exact resource URI to its existing no-argument tool.
/// Unknown URIs are left to the host for transport-specific error reporting.
pub fn resolve_mcp_resource(uri: &str) -> Option<&'static str> {
    match uri {
        "scp://status/functions" => Some("function_status_list"),
        "scp://status/openvpn" => Some("openvpn_get_status"),
        "scp://status/boundary" => Some("boundary_get_status"),
        "scp://servers/list" => Some("servers_list"),
        "scp://config/paths" => Some("config_get_paths"),
        _ => None,
    }
}
