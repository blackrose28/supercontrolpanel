use serde_json::Value;
use tauri::{AppHandle, State};

use crate::mcp::{
    allowed_mcp_resources, allowed_mcp_tools, call_mcp_tool, mcp_runtime_status, read_mcp_resource,
    stop_mcp_runtime, sync_mcp_runtime_from_settings, McpResourceDescriptor, McpRuntimeState,
    McpRuntimeStatus, McpToolCallEnvelope, McpToolDescriptor,
};
use crate::services::{config_file_path, load_sync_settings_from_disk, now_ms, ConfigPaths};
use crate::{
    BOUNDARY_AUTH_FUNCTION_ID, CONFIG_FILE_NAME, OPENVPN_FUNCTION_ID, SYNC_SETTINGS_FILE_NAME,
};

pub(crate) fn get_config_paths_internal(app: &AppHandle) -> Result<ConfigPaths, String> {
    let path = config_file_path(app, CONFIG_FILE_NAME)?;
    Ok(ConfigPaths {
        app_config_path: path.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub fn mcp_get_runtime_status(
    mcp_state: State<'_, McpRuntimeState>,
) -> Result<McpRuntimeStatus, String> {
    mcp_runtime_status(&mcp_state)
}

#[tauri::command]
pub fn mcp_sync_runtime(
    app: AppHandle,
    mcp_state: State<'_, McpRuntimeState>,
) -> Result<McpRuntimeStatus, String> {
    let settings = load_sync_settings_from_disk(&app, SYNC_SETTINGS_FILE_NAME)?;
    sync_mcp_runtime_from_settings(&app, &mcp_state, &settings, now_ms)
}

#[tauri::command]
pub fn mcp_stop_runtime(mcp_state: State<'_, McpRuntimeState>) -> Result<McpRuntimeStatus, String> {
    stop_mcp_runtime(&mcp_state)
}

#[tauri::command]
pub fn mcp_list_tools_command(app: AppHandle) -> Result<Vec<McpToolDescriptor>, String> {
    allowed_mcp_tools(&app)
}

#[tauri::command]
pub fn mcp_list_resources_command(app: AppHandle) -> Result<Vec<McpResourceDescriptor>, String> {
    allowed_mcp_resources(&app)
}

#[tauri::command]
pub fn mcp_call_tool_command(
    app: AppHandle,
    tool_name: String,
    args: Value,
) -> Result<McpToolCallEnvelope, String> {
    call_mcp_tool(
        &app,
        &tool_name,
        args,
        now_ms,
        OPENVPN_FUNCTION_ID,
        BOUNDARY_AUTH_FUNCTION_ID,
        get_config_paths_internal,
    )
}

#[tauri::command]
pub fn mcp_read_resource_command(
    app: AppHandle,
    uri: String,
) -> Result<McpToolCallEnvelope, String> {
    read_mcp_resource(
        &app,
        &uri,
        now_ms,
        OPENVPN_FUNCTION_ID,
        BOUNDARY_AUTH_FUNCTION_ID,
        get_config_paths_internal,
    )
}
