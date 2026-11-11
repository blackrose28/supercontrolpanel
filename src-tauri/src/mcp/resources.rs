use serde_json::json;
use tauri::AppHandle;

use crate::{
    mcp::{call_mcp_tool, McpResourceDescriptor, McpToolCallEnvelope},
    services::ConfigPaths,
};

pub fn list_mcp_resources() -> Vec<McpResourceDescriptor> {
    super::contracts::OPERATIONS
        .iter()
        .filter_map(|op| {
            op.resource
                .map(|(uri, title, category)| McpResourceDescriptor {
                    uri: uri.into(),
                    title: title.into(),
                    category: category.into(),
                })
        })
        .collect()
}

pub fn allowed_mcp_resources(app: &AppHandle) -> Result<Vec<McpResourceDescriptor>, String> {
    let allowed = super::tools::allowed_mcp_tools(app)?;
    Ok(list_mcp_resources()
        .into_iter()
        .filter(|resource| {
            super::contracts::resource_operation(&resource.uri)
                .is_some_and(|op| allowed.iter().any(|tool| tool.name == op.name))
        })
        .collect())
}

pub fn read_mcp_resource(
    app: &AppHandle,
    uri: &str,
    now_ms: fn() -> u64,
    openvpn_function_id: &str,
    boundary_auth_function_id: &str,
    config_paths: fn(&AppHandle) -> Result<ConfigPaths, String>,
) -> Result<McpToolCallEnvelope, String> {
    let operation = super::contracts::resource_operation(uri).ok_or_else(|| {
        super::contracts::OperationError::new(super::contracts::ErrorCode::NotFound)
            .legacy_message()
    })?;
    call_mcp_tool(
        app,
        operation.name,
        json!({}),
        now_ms,
        openvpn_function_id,
        boundary_auth_function_id,
        config_paths,
    )
}
