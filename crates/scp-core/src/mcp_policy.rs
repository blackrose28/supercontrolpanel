//! Pure MCP execution policy, separate from permissive settings persistence.
//!
//! Hosts supply trusted local policy settings, authenticate callers separately,
//! and enforce capabilities/session ownership before executing actions. Success
//! neither implements a handler nor enables an operation in the CLI. Discovery
//! metadata is deliberately not an execution allowlist or sensitivity source.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpPolicyError {
    UnknownOperation,
    UnknownPolicy,
    UnknownCategory,
    CategoryDisabled,
    ApprovalRequired,
}

impl McpPolicyError {
    const fn message(self) -> &'static str {
        match self {
            Self::UnknownOperation => "Unknown MCP operation",
            Self::UnknownPolicy => "Unknown MCP sensitive-action policy; use desktop settings to select a supported policy",
            Self::UnknownCategory => "Unknown MCP tool category in settings; use desktop settings to select supported categories",
            Self::CategoryDisabled => "MCP tool category is disabled by settings",
            Self::ApprovalRequired => "Sensitive MCP tool requires UI approval; no approval workflow is available",
        }
    }
}

impl fmt::Display for McpPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for McpPolicyError {}

/// Validate exact operation/policy/category names without normalizing settings.
///
/// Empty categories deny every operation. Unknown category entries invalidate the
/// whole list. Duplicate known entries are harmless. Approval is unavailable and
/// always blocks the explicitly sensitive operations under require-ui-approval.
/// Errors contain only fixed text, never caller-supplied names or settings values.
pub fn validate_mcp_tool_policy(
    tool_name: &str,
    allowed_categories: &[String],
    sensitive_action_policy: &str,
) -> Result<(), McpPolicyError> {
    let category = execution_category(tool_name).ok_or(McpPolicyError::UnknownOperation)?;
    if !matches!(
        sensitive_action_policy,
        "require-ui-approval" | "allow-with-token"
    ) {
        return Err(McpPolicyError::UnknownPolicy);
    }
    if allowed_categories.iter().any(|category| {
        !matches!(
            category.as_str(),
            "read-only" | "runtime-control" | "config-mutation" | "file-transfer"
        )
    }) {
        return Err(McpPolicyError::UnknownCategory);
    }
    if !allowed_categories.iter().any(|allowed| allowed == category) {
        return Err(McpPolicyError::CategoryDisabled);
    }
    if sensitive_action_policy == "require-ui-approval" && is_sensitive_tool(tool_name) {
        return Err(McpPolicyError::ApprovalRequired);
    }
    Ok(())
}

// Explicit execution mapping: a new discovery entry cannot authorize execution.
fn execution_category(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "app_get_overview"
        | "function_status_list"
        | "function_status_get"
        | "openvpn_get_status"
        | "boundary_get_status"
        | "servers_list"
        | "server_get"
        | "config_get_paths"
        | "sync_settings_get" => Some("read-only"),
        "openvpn_start"
        | "openvpn_stop"
        | "boundary_auth_start"
        | "boundary_auth_stop"
        | "server_tunnel_start"
        | "server_tunnel_stop"
        | "server_ssh_start"
        | "server_ssh_stop"
        | "caddy_start"
        | "caddy_stop" => Some("runtime-control"),
        "server_upsert"
        | "server_delete"
        | "config_save"
        | "config_save_partial"
        | "sync_settings_save_partial"
        | "config_sync_pull"
        | "config_sync_push"
        | "config_git_pull"
        | "config_git_push" => Some("config-mutation"),
        "remote_browser_open"
        | "remote_browser_close"
        | "remote_browser_list_children"
        | "remote_browser_stat_path"
        | "remote_browser_create_directory"
        | "remote_browser_delete_path"
        | "scp_upload"
        | "scp_download"
        | "scp_start_upload"
        | "scp_start_download"
        | "scp_transfer_list"
        | "scp_transfer_get" => Some("file-transfer"),
        _ => None,
    }
}

fn is_sensitive_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "openvpn_start"
            | "openvpn_stop"
            | "boundary_auth_start"
            | "server_delete"
            | "remote_browser_delete_path"
            | "scp_upload"
            | "scp_download"
            | "scp_start_upload"
            | "scp_start_download"
            | "config_sync_push"
            | "config_git_push"
    )
}
