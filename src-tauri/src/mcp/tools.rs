use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::{
    close_remote_browser_action, config_git_pull_action, config_git_push_action,
    create_remote_browser_directory_action, delete_remote_browser_path_action,
    delete_server_action, get_app_config_action, list_remote_browser_children_action,
    mcp::{McpToolCallEnvelope, McpToolDescriptor},
    open_remote_browser_action, save_app_config_action, save_sync_settings_action,
    scp_download_action, scp_upload_action,
    server_mgmt::{load_servers, Server},
    services::{
        get_scp_transfer_entry, list_scp_transfer_entries, load_sync_settings_from_disk,
        ConfigPaths, SyncSettingsPatch,
    },
    start_boundary_auth_action, start_caddy_action, start_openvpn_manual_action,
    start_scp_download_action, start_scp_upload_action, start_server_ssh_action,
    start_server_tunnel_action, stat_remote_browser_path_action, stop_boundary_auth_action,
    stop_caddy_action, stop_openvpn_action, stop_server_ssh_action, stop_server_tunnel_action,
    stores::{
        BoundaryAutomationStore, FunctionStore, OpenVpnAdminStore, RemoteBrowserStore,
        ScpTransferStore, SessionStore,
    },
    sync_config_pull_action, sync_config_push_action, upsert_server_action,
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

fn tool_category(tool_name: &str) -> Option<String> {
    list_mcp_tools()
        .into_iter()
        .find(|tool| tool.name == tool_name)
        .map(|tool| tool.category)
}

fn enforce_tool_policy(app: &AppHandle, tool_name: &str) -> Result<(), String> {
    let settings = load_sync_settings_from_disk(app, "sync-local.json")?;
    if let Some(category) = tool_category(tool_name) {
        if !settings
            .mcp_allowed_tool_categories
            .iter()
            .any(|item| item == &category)
        {
            return Err(format!(
                "MCP tool category is disabled by settings: {category}"
            ));
        }
    }
    if settings.mcp_sensitive_action_policy == "require-ui-approval" && is_sensitive_tool(tool_name)
    {
        return Err(format!(
            "Sensitive MCP tool requires UI approval and is currently blocked over HTTP: {tool_name}"
        ));
    }
    Ok(())
}

pub fn list_mcp_tools() -> Vec<McpToolDescriptor> {
    vec![
        McpToolDescriptor {
            name: "app_get_overview".to_string(),
            title: "App overview".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "function_status_list".to_string(),
            title: "List function statuses".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "function_status_get".to_string(),
            title: "Get one function status".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "openvpn_get_status".to_string(),
            title: "OpenVPN status".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "boundary_get_status".to_string(),
            title: "Boundary status".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "servers_list".to_string(),
            title: "List servers".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "server_get".to_string(),
            title: "Get server".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "config_get_paths".to_string(),
            title: "Get config paths".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "openvpn_start".to_string(),
            title: "Start OpenVPN".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "openvpn_stop".to_string(),
            title: "Stop OpenVPN".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "boundary_auth_start".to_string(),
            title: "Start Boundary auth".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "boundary_auth_stop".to_string(),
            title: "Stop Boundary auth".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_tunnel_start".to_string(),
            title: "Start server tunnel".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_tunnel_stop".to_string(),
            title: "Stop server tunnel".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_ssh_start".to_string(),
            title: "Start server SSH".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_ssh_stop".to_string(),
            title: "Stop server SSH".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "caddy_start".to_string(),
            title: "Start Caddy".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "caddy_stop".to_string(),
            title: "Stop Caddy".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_upsert".to_string(),
            title: "Create or update server".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_delete".to_string(),
            title: "Delete server".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "config_save".to_string(),
            title: "Save full app config".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "config_save_partial".to_string(),
            title: "Patch app config".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "sync_settings_get".to_string(),
            title: "Get device-local sync settings".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "sync_settings_save_partial".to_string(),
            title: "Patch device-local sync settings".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_open".to_string(),
            title: "Open remote browser session".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_close".to_string(),
            title: "Close remote browser session".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_list_children".to_string(),
            title: "List remote browser children".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_stat_path".to_string(),
            title: "Stat remote browser path".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_create_directory".to_string(),
            title: "Create remote directory".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_delete_path".to_string(),
            title: "Delete remote path".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "scp_upload".to_string(),
            title: "Run SCP upload".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "scp_download".to_string(),
            title: "Run SCP download".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "scp_start_upload".to_string(),
            title: "Start PTY-backed SCP upload".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "scp_start_download".to_string(),
            title: "Start PTY-backed SCP download".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "scp_transfer_list".to_string(),
            title: "List SCP transfers".to_string(),
            category: "file-transfer".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "scp_transfer_get".to_string(),
            title: "Get one SCP transfer".to_string(),
            category: "file-transfer".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "config_sync_pull".to_string(),
            title: "Pull synced config from repo".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "config_sync_push".to_string(),
            title: "Push synced config to repo".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "config_git_pull".to_string(),
            title: "Run git pull for config repo".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "config_git_push".to_string(),
            title: "Run git add/commit/push for config repo".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
    ]
}

pub fn call_mcp_tool(
    app: &AppHandle,
    tool_name: &str,
    args: Value,
    now_ms: fn() -> u64,
    openvpn_function_id: &str,
    boundary_auth_function_id: &str,
    config_paths: fn(&AppHandle) -> Result<ConfigPaths, String>,
) -> Result<McpToolCallEnvelope, String> {
    enforce_tool_policy(app, tool_name)?;
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
            json!({
                "appVersion": app.package_info().version.to_string(),
                "activeFunctions": active_functions,
                "knownFunctionCount": statuses.len(),
            })
        }
        "function_status_list" => {
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            json!(statuses.values().cloned().collect::<Vec<_>>())
        }
        "function_status_get" => {
            let function_id = args
                .get("functionId")
                .and_then(Value::as_str)
                .ok_or_else(|| "functionId is required".to_string())?;
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            let status = statuses
                .get(function_id)
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
            json!(statuses.get(openvpn_function_id).cloned())
        }
        "boundary_get_status" => {
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            json!(statuses.get(boundary_auth_function_id).cloned())
        }
        "servers_list" => json!(load_servers(app)?),
        "server_get" => {
            let server_id = args
                .get("serverId")
                .and_then(Value::as_str)
                .ok_or_else(|| "serverId is required".to_string())?;
            let server = load_servers(app)?
                .into_iter()
                .find(|server| server.id == server_id)
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
        "sync_settings_save_partial" => {
            let patch_value = args
                .get("patch")
                .cloned()
                .ok_or_else(|| "patch is required".to_string())?;
            let patch: SyncSettingsPatch = serde_json::from_value(patch_value)
                .map_err(|e| format!("invalid sync settings patch: {e}"))?;
            let saved = save_sync_settings_action(
                app.clone(),
                patch,
                app.state::<crate::mcp::McpRuntimeState>(),
            )?;
            redact_sync_settings_value(json!(saved))
        }
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
