use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::commands::{
    close_remote_browser_action, config_git_pull_action, config_git_push_action,
    create_remote_browser_directory_action, delete_remote_browser_path_action,
    delete_server_action, get_app_config_action, list_remote_browser_children_action,
    open_remote_browser_action, save_app_config_action, save_sync_settings_action,
    scp_download_action, scp_upload_action, start_boundary_auth_action, start_caddy_action,
    start_openvpn_manual_action, start_scp_download_action, start_scp_upload_action,
    start_server_ssh_action, start_server_tunnel_action, stat_remote_browser_path_action,
    stop_boundary_auth_action, stop_caddy_action, stop_openvpn_action, stop_server_ssh_action,
    stop_server_tunnel_action, sync_config_pull_action, sync_config_push_action,
    upsert_server_action,
};
use crate::{
    mcp::{
        McpAppOverviewResult, McpBoundaryGetStatusResult, McpFunctionStatusGetArgs,
        McpFunctionStatusGetResult, McpFunctionStatusListResult, McpOpenVpnGetStatusResult,
        McpServerGetArgs, McpServerGetResult, McpServersListResult, McpToolCallEnvelope,
    },
    server_mgmt::{load_servers, Server},
    services::{
        get_scp_transfer_entry, list_scp_transfer_entries, load_sync_settings_from_disk,
        ConfigPaths, SyncSettingsPatch,
    },
    stores::{
        BoundaryAutomationStore, FunctionStore, OpenVpnAdminStore, RemoteBrowserStore,
        ScpTransferStore, SessionStore,
    },
};

fn redact_app_config_value(value: Value) -> Value {
    let mut value = value;
    if let Some(openvpn) = value.get_mut("openvpn").and_then(Value::as_object_mut) {
        openvpn.insert(
            "password".to_string(),
            Value::String("[redacted]".to_string()),
        );
        openvpn.insert(
            "otpSecret".to_string(),
            Value::String("[redacted]".to_string()),
        );
    }
    if let Some(boundary) = value.get_mut("boundary").and_then(Value::as_object_mut) {
        boundary.insert(
            "password".to_string(),
            Value::String("[redacted]".to_string()),
        );
        boundary.insert(
            "oidcPassword".to_string(),
            Value::String("[redacted]".to_string()),
        );
        boundary.insert(
            "oidcOtpSecret".to_string(),
            Value::String("[redacted]".to_string()),
        );
    }
    value
}

fn redact_sync_settings_value(value: Value) -> Value {
    let mut value = value;
    if let Some(settings) = value.as_object_mut() {
        settings.insert(
            "mcpAuthToken".to_string(),
            Value::String("[redacted]".to_string()),
        );
    }
    value
}

fn merge_json_value(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target_map), Value::Object(patch_map)) => {
            for (key, patch_value) in patch_map {
                match target_map.get_mut(key) {
                    Some(target_value) => merge_json_value(target_value, patch_value),
                    None => {
                        target_map.insert(key.clone(), patch_value.clone());
                    }
                }
            }
        }
        (target_slot, patch_value) => {
            *target_slot = patch_value.clone();
        }
    }
}

fn save_mcp_sync_settings(
    args: &Value,
    save: impl FnOnce(SyncSettingsPatch) -> Result<crate::services::SyncSettings, String>,
) -> Result<Value, String> {
    let patch_value = args
        .get("patch")
        .cloned()
        .ok_or_else(|| "patch is required".to_string())?;
    // Remote callers must never change their own authorization or CC identity.
    // An explicit allowlist also keeps future local controls local by default.
    let fields = patch_value
        .as_object()
        .ok_or_else(|| "invalid sync settings patch: expected object".to_string())?;
    if fields.keys().any(|field| {
        !matches!(
            field.as_str(),
            "repoRoot"
                | "openvpnExecutablePath"
                | "openvpnOvpnPath"
                | "boundaryCliPath"
                | "caddyFolder"
        )
    }) {
        return Err("MCP settings patch permits only path/folder fields; use desktop settings for other controls".to_string());
    }
    let patch: SyncSettingsPatch = serde_json::from_value(patch_value)
        .map_err(|_| "invalid sync settings patch".to_string())?;
    let saved = save(patch)?;
    Ok(redact_sync_settings_value(json!(saved)))
}

fn enforce_tool_policy(
    tool_name: &str,
    load_settings: impl FnOnce() -> Result<crate::services::SyncSettings, String>,
) -> Result<(), String> {
    let settings = load_settings()?;
    scp_core::mcp_policy::validate_mcp_tool_policy(
        tool_name,
        &settings.mcp_allowed_tool_categories,
        &settings.mcp_sensitive_action_policy,
    )
    .map_err(|error| error.to_string())
}

pub use scp_core::mcp_catalog::list_mcp_tools;

pub fn call_mcp_tool(
    app: &AppHandle,
    tool_name: &str,
    args: Value,
    now_ms: fn() -> u64,
    openvpn_function_id: &str,
    boundary_auth_function_id: &str,
    config_paths: fn(&AppHandle) -> Result<ConfigPaths, String>,
) -> Result<McpToolCallEnvelope, String> {
    enforce_tool_policy(tool_name, || {
        load_sync_settings_from_disk(app, "sync-local.json")
    })?;
    let data = match tool_name {
        "app_get_overview" => {
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            let active_functions = statuses
                .values()
                .filter(|status| status.session_id.is_some())
                .count();
            json!(McpAppOverviewResult {
                app_version: app.package_info().version.to_string(),
                active_functions,
                known_function_count: statuses.len(),
            })
        }
        "function_status_list" => {
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            let result: McpFunctionStatusListResult = statuses.values().cloned().collect();
            json!(result)
        }
        "function_status_get" => {
            let args = McpFunctionStatusGetArgs::from_args(&args)?;
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            let status: McpFunctionStatusGetResult = statuses
                .get(args.function_id)
                .cloned()
                .ok_or_else(|| "function status not found".to_string())?;
            json!(status)
        }
        "openvpn_get_status" => {
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            let result: McpOpenVpnGetStatusResult = statuses.get(openvpn_function_id).cloned();
            json!(result)
        }
        "boundary_get_status" => {
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            let result: McpBoundaryGetStatusResult =
                statuses.get(boundary_auth_function_id).cloned();
            json!(result)
        }
        "servers_list" => {
            let result: McpServersListResult = load_servers(app)?;
            json!(result)
        }
        "server_get" => {
            let args = McpServerGetArgs::from_args(&args)?;
            let server: McpServerGetResult = load_servers(app)?
                .into_iter()
                .find(|server| server.id == args.server_id)
                .ok_or_else(|| "server not found".to_string())?;
            json!(server)
        }
        "config_get_paths" => json!(config_paths(app)?),
        "openvpn_start" => {
            let username = args
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let password = args
                .get("password")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let result = start_openvpn_manual_action(
                "".to_string(),
                username,
                password,
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
                app.state::<OpenVpnAdminStore>(),
            )?;
            json!(result)
        }
        "openvpn_stop" => {
            stop_openvpn_action(
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
                app.state::<OpenVpnAdminStore>(),
            )?;
            json!({})
        }
        "boundary_auth_start" => {
            let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(120) as u16;
            let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(36) as u16;
            let result = start_boundary_auth_action(
                cols,
                rows,
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
                app.state::<BoundaryAutomationStore>(),
            )?;
            json!(result)
        }
        "boundary_auth_stop" => {
            stop_boundary_auth_action(
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
                app.state::<BoundaryAutomationStore>(),
            )?;
            json!({})
        }
        "server_tunnel_start" => {
            let server_id = args
                .get("serverId")
                .and_then(Value::as_str)
                .ok_or_else(|| "serverId is required".to_string())?
                .to_string();
            let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(120) as u16;
            let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(36) as u16;
            let result = start_server_tunnel_action(
                server_id,
                cols,
                rows,
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
                app.state::<RemoteBrowserStore>(),
            )?;
            json!(result)
        }
        "server_tunnel_stop" => {
            let server_id = args
                .get("serverId")
                .and_then(Value::as_str)
                .ok_or_else(|| "serverId is required".to_string())?
                .to_string();
            stop_server_tunnel_action(
                server_id,
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
                app.state::<RemoteBrowserStore>(),
            )?;
            json!({})
        }
        "server_ssh_start" => {
            let server_id = args
                .get("serverId")
                .and_then(Value::as_str)
                .ok_or_else(|| "serverId is required".to_string())?
                .to_string();
            let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(120) as u16;
            let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(36) as u16;
            let result = start_server_ssh_action(
                server_id,
                cols,
                rows,
                None,
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
            )?;
            json!(result)
        }
        "server_ssh_stop" => {
            let server_id = args
                .get("serverId")
                .and_then(Value::as_str)
                .ok_or_else(|| "serverId is required".to_string())?
                .to_string();
            stop_server_ssh_action(
                server_id,
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
            )?;
            json!({})
        }
        "caddy_start" => {
            let folder = args
                .get("folder")
                .and_then(Value::as_str)
                .ok_or_else(|| "folder is required".to_string())?
                .to_string();
            let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(120) as u16;
            let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(36) as u16;
            let result = start_caddy_action(
                cols,
                rows,
                folder,
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
            )?;
            json!(result)
        }
        "caddy_stop" => {
            stop_caddy_action(
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
            )?;
            json!({})
        }
        "server_upsert" => {
            let server_value = args
                .get("server")
                .cloned()
                .ok_or_else(|| "server is required".to_string())?;
            let server: Server =
                serde_json::from_value(server_value).map_err(|e| format!("invalid server: {e}"))?;
            let servers =
                upsert_server_action(app.clone(), server, app.state::<RemoteBrowserStore>())?;
            json!(servers)
        }
        "server_delete" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "id is required".to_string())?
                .to_string();
            let servers = delete_server_action(app.clone(), id, app.state::<RemoteBrowserStore>())?;
            json!(servers)
        }
        "config_save" => {
            let config_value = args
                .get("config")
                .cloned()
                .ok_or_else(|| "config is required".to_string())?;
            let config =
                serde_json::from_value(config_value).map_err(|e| format!("invalid config: {e}"))?;
            let saved = save_app_config_action(app.clone(), config)?;
            redact_app_config_value(json!(saved))
        }
        "config_save_partial" => {
            let patch = args
                .get("patch")
                .cloned()
                .ok_or_else(|| "patch is required".to_string())?;
            let current = get_app_config_action(app.clone())?;
            let mut current_value =
                serde_json::to_value(current).map_err(|e| format!("encode config: {e}"))?;
            merge_json_value(&mut current_value, &patch);
            let merged = serde_json::from_value(current_value)
                .map_err(|e| format!("invalid merged config: {e}"))?;
            let saved = save_app_config_action(app.clone(), merged)?;
            redact_app_config_value(json!(saved))
        }
        "sync_settings_get" => {
            let settings = load_sync_settings_from_disk(app, "sync-local.json")?;
            redact_sync_settings_value(json!(settings))
        }
        "sync_settings_save_partial" => save_mcp_sync_settings(&args, |patch| {
            save_sync_settings_action(
                app.clone(),
                patch,
                app.state::<crate::mcp::McpRuntimeState>(),
            )
        })?,
        "remote_browser_open" => {
            let server_id = args
                .get("serverId")
                .and_then(Value::as_str)
                .ok_or_else(|| "serverId is required".to_string())?
                .to_string();
            let session = open_remote_browser_action(
                app.clone(),
                server_id,
                app.state::<RemoteBrowserStore>(),
                app.state::<FunctionStore>(),
            )?;
            json!(session)
        }
        "remote_browser_close" => {
            let browser_id = args
                .get("browserId")
                .and_then(Value::as_str)
                .ok_or_else(|| "browserId is required".to_string())?
                .to_string();
            close_remote_browser_action(browser_id, app.state::<RemoteBrowserStore>())?;
            json!({})
        }
        "remote_browser_list_children" => {
            let browser_id = args
                .get("browserId")
                .and_then(Value::as_str)
                .ok_or_else(|| "browserId is required".to_string())?
                .to_string();
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let entries = list_remote_browser_children_action(
                browser_id,
                path,
                app.state::<RemoteBrowserStore>(),
            )?;
            json!(entries)
        }
        "remote_browser_stat_path" => {
            let browser_id = args
                .get("browserId")
                .and_then(Value::as_str)
                .ok_or_else(|| "browserId is required".to_string())?
                .to_string();
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let info = stat_remote_browser_path_action(
                browser_id,
                path,
                app.state::<RemoteBrowserStore>(),
            )?;
            json!(info)
        }
        "remote_browser_create_directory" => {
            let browser_id = args
                .get("browserId")
                .and_then(Value::as_str)
                .ok_or_else(|| "browserId is required".to_string())?
                .to_string();
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "path is required".to_string())?
                .to_string();
            create_remote_browser_directory_action(
                browser_id,
                path,
                app.state::<RemoteBrowserStore>(),
            )?;
            json!({})
        }
        "remote_browser_delete_path" => {
            let browser_id = args
                .get("browserId")
                .and_then(Value::as_str)
                .ok_or_else(|| "browserId is required".to_string())?
                .to_string();
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "path is required".to_string())?
                .to_string();
            let recursive = args.get("recursive").and_then(Value::as_bool);
            delete_remote_browser_path_action(
                browser_id,
                path,
                recursive,
                app.state::<RemoteBrowserStore>(),
            )?;
            json!({})
        }
        "scp_upload" => {
            let server_id = args
                .get("serverId")
                .and_then(Value::as_str)
                .ok_or_else(|| "serverId is required".to_string())?
                .to_string();
            let local_path = args
                .get("localPath")
                .and_then(Value::as_str)
                .ok_or_else(|| "localPath is required".to_string())?
                .to_string();
            let remote_path = args
                .get("remotePath")
                .and_then(Value::as_str)
                .ok_or_else(|| "remotePath is required".to_string())?
                .to_string();
            let recursive = args.get("recursive").and_then(Value::as_bool);
            let result = scp_upload_action(
                app.clone(),
                server_id,
                local_path,
                remote_path,
                recursive,
                app.state::<ScpTransferStore>(),
            )?;
            json!({ "result": result })
        }
        "scp_download" => {
            let server_id = args
                .get("serverId")
                .and_then(Value::as_str)
                .ok_or_else(|| "serverId is required".to_string())?
                .to_string();
            let remote_path = args
                .get("remotePath")
                .and_then(Value::as_str)
                .ok_or_else(|| "remotePath is required".to_string())?
                .to_string();
            let local_path = args
                .get("localPath")
                .and_then(Value::as_str)
                .ok_or_else(|| "localPath is required".to_string())?
                .to_string();
            let recursive = args.get("recursive").and_then(Value::as_bool);
            let result = scp_download_action(
                app.clone(),
                server_id,
                remote_path,
                local_path,
                recursive,
                app.state::<ScpTransferStore>(),
            )?;
            json!({ "result": result })
        }
        "scp_start_upload" => {
            let server_id = args
                .get("serverId")
                .and_then(Value::as_str)
                .ok_or_else(|| "serverId is required".to_string())?
                .to_string();
            let local_path = args
                .get("localPath")
                .and_then(Value::as_str)
                .ok_or_else(|| "localPath is required".to_string())?
                .to_string();
            let remote_path = args
                .get("remotePath")
                .and_then(Value::as_str)
                .ok_or_else(|| "remotePath is required".to_string())?
                .to_string();
            let recursive = args.get("recursive").and_then(Value::as_bool);
            let session_id = start_scp_upload_action(
                app.clone(),
                app.state::<SessionStore>(),
                server_id,
                local_path,
                remote_path,
                recursive,
                app.state::<ScpTransferStore>(),
            )?;
            json!({ "sessionId": session_id })
        }
        "scp_start_download" => {
            let server_id = args
                .get("serverId")
                .and_then(Value::as_str)
                .ok_or_else(|| "serverId is required".to_string())?
                .to_string();
            let remote_path = args
                .get("remotePath")
                .and_then(Value::as_str)
                .ok_or_else(|| "remotePath is required".to_string())?
                .to_string();
            let local_path = args
                .get("localPath")
                .and_then(Value::as_str)
                .ok_or_else(|| "localPath is required".to_string())?
                .to_string();
            let recursive = args.get("recursive").and_then(Value::as_bool);
            let session_id = start_scp_download_action(
                app.clone(),
                app.state::<SessionStore>(),
                server_id,
                remote_path,
                local_path,
                recursive,
                app.state::<ScpTransferStore>(),
            )?;
            json!({ "sessionId": session_id })
        }
        "scp_transfer_list" => {
            let entries = list_scp_transfer_entries(&app.state::<ScpTransferStore>())?;
            json!(entries)
        }
        "scp_transfer_get" => {
            let transfer_id = args
                .get("transferId")
                .and_then(Value::as_str)
                .ok_or_else(|| "transferId is required".to_string())?;
            let entry = get_scp_transfer_entry(&app.state::<ScpTransferStore>(), transfer_id)?
                .ok_or_else(|| "transfer not found".to_string())?;
            json!(entry)
        }
        "config_sync_pull" => {
            let repo_root = args
                .get("repoRoot")
                .and_then(Value::as_str)
                .ok_or_else(|| "repoRoot is required".to_string())?
                .to_string();
            sync_config_pull_action(app.clone(), repo_root)?;
            json!({})
        }
        "config_sync_push" => {
            let repo_root = args
                .get("repoRoot")
                .and_then(Value::as_str)
                .ok_or_else(|| "repoRoot is required".to_string())?
                .to_string();
            sync_config_push_action(app.clone(), repo_root)?;
            json!({})
        }
        "config_git_pull" => {
            let repo_root = args
                .get("repoRoot")
                .and_then(Value::as_str)
                .ok_or_else(|| "repoRoot is required".to_string())?
                .to_string();
            let result = config_git_pull_action(repo_root)?;
            json!({ "result": result })
        }
        "config_git_push" => {
            let repo_root = args
                .get("repoRoot")
                .and_then(Value::as_str)
                .ok_or_else(|| "repoRoot is required".to_string())?
                .to_string();
            let result = config_git_push_action(repo_root)?;
            json!({ "result": result })
        }
        _ => return Err(format!("unknown MCP tool: {tool_name}")),
    };

    Ok(McpToolCallEnvelope {
        ok: true,
        message: format!("{tool_name} completed"),
        data,
        warnings: vec!["Minimal localhost HTTP transport is enabled; full MCP spec compatibility is still in progress.".to_string()],
        timestamp: now_ms(),
    })
}

#[cfg(test)]
mod catalog_contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_catalog.rs"
    ));
}

#[cfg(test)]
mod shared_catalog_identity {
    #[test]
    fn desktop_discovery_uses_shared_types() {
        let tools: Vec<scp_core::mcp_catalog::McpToolDescriptor> = super::list_mcp_tools();
        let _: Vec<crate::mcp::dto::McpToolDescriptor> = tools;
    }

    #[test]
    fn discovery_names_match_desktop_dispatch_arms() {
        // A static boundary check: it does not execute Tauri actions or prove policy.
        let source = include_str!("tools.rs");
        let dispatch = source.split("let data = match tool_name {").nth(1).unwrap();
        let dispatch = dispatch.split("#[cfg(test)]").next().unwrap();
        let actual: std::collections::BTreeSet<_> = dispatch
            .lines()
            .filter_map(|line| line.strip_prefix("        \""))
            .filter_map(|line| line.split_once("\" =>").map(|(name, _)| name))
            .collect();
        let tools = super::list_mcp_tools();
        let advertised: std::collections::BTreeSet<_> =
            tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(actual, advertised);
    }
}

#[cfg(test)]
mod remote_settings_guard {
    use super::*;
    use crate::services::SyncSettings;

    #[test]
    fn remote_control_settings_are_rejected_before_any_write() {
        let controls = json!({
            "mcpEnabled": false, "mcpBindAddress": "127.0.0.1", "mcpPort": 0,
            "mcpAuthToken": "secret-marker", "mcpAllowedToolCategories": [],
            "mcpSensitiveActionPolicy": "allow-with-token", "ccEnabled": true,
            "ccServerUrl": "secret-marker", "ccAuthMode": "token",
            "ccAgentToken": "secret-marker", "ccDeviceId": "device",
            "ccDeviceName": "name", "firstRunCompleted": false,
            "futureControl": "secret-marker"
        });
        for (field, value) in controls.as_object().unwrap() {
            for value in [value.clone(), Value::Null] {
                let mut patch = json!({"repoRoot": "allowed-but-must-not-be-written"});
                patch[field] = value;
                let mut writes = 0;
                let result = save_mcp_sync_settings(&json!({"patch": patch}), |_| {
                    writes += 1;
                    Ok(SyncSettings::default())
                });
                assert!(result.is_err(), "accepted remote control field {field}");
                assert_eq!(writes, 0, "partially saved patch containing {field}");
                let error = result.unwrap_err();
                assert!(error.contains("desktop settings"), "{error}");
                assert!(!error.contains("secret-marker"));
            }
        }
    }

    #[test]
    fn remote_path_patch_preserves_values_and_redacts_saved_token() {
        let result = save_mcp_sync_settings(
            &json!({"patch": {
                "repoRoot": "", "openvpnExecutablePath": "vpn", "openvpnOvpnPath": "profile",
                "boundaryCliPath": "boundary", "caddyFolder": "folder"
            }}),
            |patch| {
                assert_eq!(patch.repo_root.as_deref(), Some(""));
                assert_eq!(patch.openvpn_executable_path.as_deref(), Some("vpn"));
                assert_eq!(patch.openvpn_ovpn_path.as_deref(), Some("profile"));
                assert_eq!(patch.boundary_cli_path.as_deref(), Some("boundary"));
                assert_eq!(patch.caddy_folder.as_deref(), Some("folder"));
                assert!(patch.mcp_sensitive_action_policy.is_none());
                Ok(SyncSettings {
                    mcp_auth_token: "secret-marker".into(),
                    ..Default::default()
                })
            },
        )
        .unwrap();
        assert_eq!(result["mcpAuthToken"], "[redacted]");
        assert!(!result.to_string().contains("secret-marker"));
    }

    #[test]
    fn invalid_remote_patches_never_write_or_echo_input() {
        for args in [
            json!({}),
            json!({"patch": null}),
            json!({"patch": []}),
            json!({"patch": "secret-marker"}),
            json!({"patch": {"repoRoot": ["secret-marker"]}}),
        ] {
            let result = save_mcp_sync_settings(&args, |_| panic!("invalid patch reached writer"));
            assert!(result.is_err());
            assert!(!result.unwrap_err().contains("secret-marker"));
        }
    }

    #[test]
    fn empty_and_null_path_patches_preserve_omission_and_writer_errors() {
        for patch in [json!({}), json!({"repoRoot": null, "caddyFolder": null})] {
            let error = save_mcp_sync_settings(&json!({"patch": patch}), |patch| {
                assert!(patch.repo_root.is_none());
                assert!(patch.caddy_folder.is_none());
                Err("writer unavailable".into())
            })
            .unwrap_err();
            assert_eq!(error, "writer unavailable");
        }
    }
}

#[cfg(test)]
mod execution_policy_gate {
    use super::*;
    use crate::services::SyncSettings;
    use std::cell::Cell;

    #[test]
    fn dispatcher_gate_rejects_invalid_settings_and_unknown_operations() {
        for (tool, policy, categories) in [
            ("servers_list", "future-policy", vec!["read-only"]),
            (
                "servers_list",
                "allow-with-token",
                vec!["read-only", "future-category"],
            ),
            ("future-operation", "allow-with-token", vec!["read-only"]),
            (
                "openvpn_start",
                "require-ui-approval",
                vec!["runtime-control"],
            ),
            ("servers_list", "allow-with-token", vec!["runtime-control"]),
        ] {
            let settings = SyncSettings {
                mcp_sensitive_action_policy: policy.to_owned(),
                mcp_allowed_tool_categories: categories.into_iter().map(str::to_owned).collect(),
                ..SyncSettings::default()
            };
            assert!(
                enforce_tool_policy(tool, || Ok(settings)).is_err(),
                "{tool}: {policy}"
            );
        }
    }

    #[test]
    fn dispatcher_gate_loads_once_and_preserves_load_errors() {
        let loads = Cell::new(0);
        assert_eq!(
            enforce_tool_policy("servers_list", || {
                loads.set(loads.get() + 1);
                Ok(SyncSettings::default())
            }),
            Ok(())
        );
        assert_eq!(loads.get(), 1);
        assert_eq!(
            enforce_tool_policy("servers_list", || Err("settings unavailable".to_owned())),
            Err("settings unavailable".to_owned())
        );
    }

    #[test]
    fn dispatcher_checks_policy_before_matching_any_action() {
        // Static ordering check, complemented by the executed gate tests above.
        // Does not exercise Tauri, HTTP, resources or side effects end to end.
        let source = include_str!("tools.rs");
        let body = source.split("pub fn call_mcp_tool(").nth(1).unwrap();
        let before_dispatch = body.split("let data = match tool_name {").next().unwrap();
        assert!(before_dispatch.contains("enforce_tool_policy(tool_name, ||"));
        assert!(before_dispatch.contains("load_sync_settings_from_disk(app, \"sync-local.json\")"));
        assert!(before_dispatch.contains("})?;"));
    }
}

#[cfg(test)]
mod resource_policy_gate {
    use super::enforce_tool_policy;
    use crate::services::SyncSettings;

    #[test]
    fn resolved_resources_still_reject_invalid_execution_settings() {
        for resource in scp_core::mcp_resources::list_mcp_resources() {
            let tool = scp_core::mcp_resources::resolve_mcp_resource(&resource.uri).unwrap();
            for policy in ["require-ui-approval", "allow-with-token"] {
                let settings = SyncSettings {
                    mcp_sensitive_action_policy: policy.to_owned(),
                    mcp_allowed_tool_categories: vec!["read-only".to_owned()],
                    ..SyncSettings::default()
                };
                assert_eq!(enforce_tool_policy(tool, || Ok(settings.clone())), Ok(()));
                let invalid_policy = SyncSettings {
                    mcp_sensitive_action_policy: "future-policy".to_owned(),
                    ..settings.clone()
                };
                assert!(enforce_tool_policy(tool, || Ok(invalid_policy)).is_err());
                for category in [resource.category.as_str(), "future-category"] {
                    let invalid_category = SyncSettings {
                        mcp_allowed_tool_categories: vec![
                            "read-only".to_owned(),
                            category.to_owned(),
                        ],
                        ..settings.clone()
                    };
                    assert!(enforce_tool_policy(tool, || Ok(invalid_category)).is_err());
                }
                let disabled = SyncSettings {
                    mcp_allowed_tool_categories: vec!["runtime-control".to_owned()],
                    ..settings
                };
                assert!(enforce_tool_policy(tool, || Ok(disabled)).is_err());
            }
        }
    }
}
