use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::commands::{
    close_remote_browser_action, create_remote_browser_directory_action,
    delete_remote_browser_path_action, delete_server_action, get_app_config_action,
    list_remote_browser_children_action, open_remote_browser_action, save_app_config_action,
    scp_download_action, scp_upload_action, start_boundary_auth_action, start_caddy_action,
    start_openvpn_manual_action, start_scp_download_action, start_scp_upload_action,
    start_server_ssh_action, start_server_tunnel_action, stat_remote_browser_path_action,
    stop_boundary_auth_action, stop_caddy_action, stop_openvpn_action, stop_server_ssh_action,
    stop_server_tunnel_action, sync_config_push_action, upsert_server_action,
};
use crate::{
    mcp::{McpToolCallEnvelope, McpToolDescriptor},
    server_mgmt::{load_servers, Server},
    services::{
        get_scp_transfer_entry, list_scp_transfer_entries, load_sync_settings_from_disk,
        save_sync_settings_to_disk, ConfigPaths,
    },
    stores::{
        BoundaryAutomationStore, FunctionStore, OpenVpnAdminStore, RemoteBrowserStore,
        ScpTransferStore, SessionStore,
    },
};

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

pub fn list_mcp_tools() -> Vec<McpToolDescriptor> {
    super::contracts::OPERATIONS
        .iter()
        .map(|op| McpToolDescriptor {
            name: op.name.into(),
            title: op.title.into(),
            category: op.category.into(),
            read_only: op.read_only,
        })
        .collect()
}

pub fn allowed_mcp_tools(app: &AppHandle) -> Result<Vec<McpToolDescriptor>, String> {
    let settings = load_sync_settings_from_disk(app, "sync-local.json").map_err(|_| {
        super::contracts::OperationError::new(super::contracts::ErrorCode::OperationFailed)
            .legacy_message()
    })?;
    let categories: Vec<_> = settings
        .mcp_allowed_tool_categories
        .iter()
        .map(String::as_str)
        .collect();
    Ok(list_mcp_tools()
        .into_iter()
        .filter(|op| {
            super::contracts::authorize(
                &op.name,
                &settings.mcp_sensitive_action_policy,
                &categories,
            )
            .is_ok()
        })
        .collect())
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
    // Preserve the existing backup/recovery lease through validation and mutation.
    let _guard = crate::settings_backup::gate::enter().map_err(|_| {
        super::contracts::OperationError::new(super::contracts::ErrorCode::Conflict)
            .legacy_message()
    })?;
    let settings = load_sync_settings_from_disk(app, "sync-local.json").map_err(|_| {
        super::contracts::OperationError::new(super::contracts::ErrorCode::OperationFailed)
            .legacy_message()
    })?;
    let categories: Vec<_> = settings
        .mcp_allowed_tool_categories
        .iter()
        .map(String::as_str)
        .collect();
    super::contracts::execute(
        tool_name,
        &settings.mcp_sensitive_action_policy,
        &categories,
        &args,
        |op| {
            dispatch_mcp_tool(
                app,
                op,
                args.clone(),
                now_ms,
                openvpn_function_id,
                boundary_auth_function_id,
                config_paths,
            )
        },
    )
    .map_err(|err| err.legacy_message())
}

fn dispatch_mcp_tool(
    app: &AppHandle,
    operation: &super::contracts::Operation,
    args: Value,
    now_ms: fn() -> u64,
    openvpn_function_id: &str,
    boundary_auth_function_id: &str,
    config_paths: fn(&AppHandle) -> Result<ConfigPaths, String>,
) -> Result<McpToolCallEnvelope, String> {
    let mut data = match operation.handler {
        super::contracts::Handler::BoundarySearchTargets
        | super::contracts::Handler::DetectOpenvpnExecutable
        | super::contracts::Handler::ServerRdpStart
        | super::contracts::Handler::ServerSshPasswordSet
        | super::contracts::Handler::ServerSshPasswordClear => {
            super::parity::dispatch(operation.handler, &args, &super::parity::DesktopParityHost(app))?
        }
        // Exhaustive matching makes adding an advertised operation require a host decision.
        super::contracts::Handler::AppGetOverview => {
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
        super::contracts::Handler::FunctionStatusList => {
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            json!(statuses
                .values()
                .map(super::public_dto::SafeFunctionStatus::from)
                .collect::<Vec<_>>())
        }
        super::contracts::Handler::FunctionStatusGet => {
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
            json!(super::public_dto::SafeFunctionStatus::from(&status))
        }
        super::contracts::Handler::OpenvpnGetStatus => {
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            json!(statuses
                .get(openvpn_function_id)
                .map(super::public_dto::SafeFunctionStatus::from))
        }
        super::contracts::Handler::BoundaryGetStatus => {
            let function_store = app.state::<FunctionStore>();
            let statuses = function_store
                .statuses
                .lock()
                .map_err(|_| "function status lock poisoned".to_string())?;
            json!(statuses
                .get(boundary_auth_function_id)
                .map(super::public_dto::SafeFunctionStatus::from))
        }
        super::contracts::Handler::ServersList => json!(load_servers(app)?),
        super::contracts::Handler::ServerGet => {
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
        super::contracts::Handler::ConfigGetPaths => json!(config_paths(app)?),
        super::contracts::Handler::OpenvpnStart => {
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
        super::contracts::Handler::OpenvpnStop => {
            stop_openvpn_action(
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
                app.state::<OpenVpnAdminStore>(),
            )?;
            json!({})
        }
        super::contracts::Handler::BoundaryAuthStart => {
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
        super::contracts::Handler::BoundaryAuthStop => {
            stop_boundary_auth_action(
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
                app.state::<BoundaryAutomationStore>(),
            )?;
            json!({})
        }
        super::contracts::Handler::ServerTunnelStart => {
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
        super::contracts::Handler::ServerTunnelStop => {
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
        super::contracts::Handler::ServerSshStart => {
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
        super::contracts::Handler::ServerSshStop => {
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
        super::contracts::Handler::CaddyStart => {
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
        super::contracts::Handler::CaddyStop => {
            stop_caddy_action(
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
            )?;
            json!({})
        }
        super::contracts::Handler::ServerUpsert => {
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
        super::contracts::Handler::ServerDelete => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "id is required".to_string())?
                .to_string();
            let servers = delete_server_action(app.clone(), id, app.state::<RemoteBrowserStore>())?;
            json!(servers)
        }
        super::contracts::Handler::ConfigSave => {
            let config_value = args
                .get("config")
                .cloned()
                .ok_or_else(|| "config is required".to_string())?;
            let config =
                serde_json::from_value(config_value).map_err(|e| format!("invalid config: {e}"))?;
            let current = get_app_config_action(app.clone())?;
            super::validation::protect_config_replacement(&json!(current), &json!(config))
                .map_err(|err| err.legacy_message())?;
            let saved = save_app_config_action(app.clone(), config)?;
            json!(super::public_dto::SafeAppConfig::from(&saved))
        }
        super::contracts::Handler::ConfigSavePartial => {
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
            json!(super::public_dto::SafeAppConfig::from(&saved))
        }
        super::contracts::Handler::SyncSettingsGet => {
            let settings = load_sync_settings_from_disk(app, "sync-local.json")?;
            json!(super::public_dto::SafeSyncSettings::from(&settings))
        }
        super::contracts::Handler::SyncSettingsSavePartial => {
            let patch_value = args
                .get("patch")
                .cloned()
                .ok_or_else(|| "patch is required".to_string())?;
            let mut saved = load_sync_settings_from_disk(app, "sync-local.json")?;
            // The public patch cannot reconfigure the listener or activate CC.
            // Do not call the local settings action: it restarts the listener, including this worker.
            if let Some(repo_root) = patch_value.get("repoRoot").and_then(Value::as_str) {
                saved.repo_root = repo_root.into();
            }
            save_sync_settings_to_disk(app, &saved, "sync-local.json")?;
            json!(super::public_dto::SafeSyncSettings::from(&saved))
        }
        super::contracts::Handler::RemoteBrowserOpen => {
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
        super::contracts::Handler::RemoteBrowserClose => {
            let browser_id = args
                .get("browserId")
                .and_then(Value::as_str)
                .ok_or_else(|| "browserId is required".to_string())?
                .to_string();
            close_remote_browser_action(browser_id, app.state::<RemoteBrowserStore>())?;
            json!({})
        }
        super::contracts::Handler::RemoteBrowserListChildren => {
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
        super::contracts::Handler::RemoteBrowserStatPath => {
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
        super::contracts::Handler::RemoteBrowserCreateDirectory => {
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
        super::contracts::Handler::RemoteBrowserDeletePath => {
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
        super::contracts::Handler::ScpUpload => {
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
            let _result = scp_upload_action(
                app.clone(),
                server_id,
                local_path,
                remote_path,
                recursive,
                app.state::<ScpTransferStore>(),
            )?;
            json!({ "result": "Operation completed; inspect the local desktop for details." })
        }
        super::contracts::Handler::ScpDownload => {
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
            let _result = scp_download_action(
                app.clone(),
                server_id,
                remote_path,
                local_path,
                recursive,
                app.state::<ScpTransferStore>(),
            )?;
            json!({ "result": "Operation completed; inspect the local desktop for details." })
        }
        super::contracts::Handler::ScpStartUpload => {
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
        super::contracts::Handler::ScpStartDownload => {
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
        super::contracts::Handler::ScpTransferList => {
            let entries = list_scp_transfer_entries(&app.state::<ScpTransferStore>())?;
            json!(entries
                .iter()
                .map(super::public_dto::SafeTransfer::from)
                .collect::<Vec<_>>())
        }
        super::contracts::Handler::ScpTransferGet => {
            let transfer_id = args
                .get("transferId")
                .and_then(Value::as_str)
                .ok_or_else(|| "transferId is required".to_string())?;
            let entry = get_scp_transfer_entry(&app.state::<ScpTransferStore>(), transfer_id)?
                .ok_or_else(|| "transfer not found".to_string())?;
            json!(super::public_dto::SafeTransfer::from(&entry))
        }
        super::contracts::Handler::ConfigSyncPull => {
            return Err("MCP capability unavailable".into())
        }
        super::contracts::Handler::ConfigSyncPush => {
            let repo_root = args
                .get("repoRoot")
                .and_then(Value::as_str)
                .ok_or_else(|| "repoRoot is required".to_string())?
                .to_string();
            sync_config_push_action(app.clone(), repo_root)?;
            json!({})
        }
        super::contracts::Handler::ConfigGitPull => return Err("MCP capability unavailable".into()),
        super::contracts::Handler::ConfigGitPush => return Err("MCP capability unavailable".into()),
    };

    super::contracts::redact_fields(&mut data);
    if !super::validation::conforms(&super::validation::output_schema(operation), &data) {
        return Err("MCP host returned an invalid public result".into());
    }
    Ok(McpToolCallEnvelope {
        ok: true,
        message: "MCP operation completed".into(),
        data,
        warnings: vec!["Minimal localhost HTTP transport is enabled; full MCP spec compatibility is still in progress.".to_string()],
        timestamp: now_ms(),
    })
}

#[cfg(test)]
mod slice1_tests {
    use super::*;

    #[test]
    fn every_launch_and_mutation_is_sensitive() {
        for name in [
            "openvpn_start",
            "openvpn_stop",
            "boundary_auth_start",
            "boundary_auth_stop",
            "server_tunnel_start",
            "server_tunnel_stop",
            "server_ssh_start",
            "server_ssh_stop",
            "caddy_start",
            "caddy_stop",
            "server_upsert",
            "server_delete",
            "config_save",
            "config_save_partial",
            "sync_settings_save_partial",
            "remote_browser_open",
            "remote_browser_close",
            "remote_browser_create_directory",
            "remote_browser_delete_path",
            "scp_upload",
            "scp_download",
            "scp_start_upload",
            "scp_start_download",
            "config_sync_pull",
            "config_sync_push",
            "config_git_pull",
            "config_git_push",
        ] {
            assert!(
                !super::super::contracts::operation(name).unwrap().read_only,
                "{name}"
            );
        }
    }

    #[test]
    fn public_outputs_are_allowlists_and_drop_future_secret_fields() {
        let settings = json!(
            serde_json::from_value::<super::super::public_dto::SafeSyncSettings>(
                json!({"repoRoot":"repo", "futureSecret":"leak", "ccAgentToken":"cc-secret"})
            )
            .unwrap()
        );
        assert!(settings.get("futureSecret").is_none());
        let config = json!(serde_json::from_value::<super::super::public_dto::SafeAppConfig>(json!({"app":{"defaultTerminalCols":100,"defaultTerminalRows":30},"futureSecret":"leak", "openvpn":{"password":"secret"}})).unwrap());
        assert!(config.get("futureSecret").is_none());
        assert!(!config.to_string().contains("secret"));
    }

    #[test]
    fn desktop_conversions_exclude_credentials_and_remote_diagnostics() {
        use super::super::public_dto::*;
        let mut config = crate::services::default_app_config();
        config.openvpn.password = "sentinel-secret".into();
        config.openvpn.otp_secret = "sentinel-secret".into();
        config.openvpn.extra_args = "sentinel-secret".into();
        config.boundary.password = "sentinel-secret".into();
        config.boundary.oidc_password = "sentinel-secret".into();
        config.boundary.oidc_otp_secret = "sentinel-secret".into();
        assert!(!json!(SafeAppConfig::from(&config))
            .to_string()
            .contains("sentinel-secret"));
        let status = crate::models::FunctionStatus {
            function_id: "boundary-auth".into(),
            state: "running".into(),
            message: "sentinel-secret".into(),
            boundary_token_source: Some("app".into()),
            session_id: Some("session".into()),
            updated_at_ms: 123,
        };
        let mut value = json!(SafeFunctionStatus::from(&status));
        super::super::contracts::redact_fields(&mut value);
        assert!(!value.to_string().contains("sentinel-secret"));
        assert_eq!(value["boundaryTokenSource"], "app");
        assert_eq!(value["sessionId"], "session");
        assert_eq!(value["updatedAtMs"], 123);
        let entry = crate::models::ScpTransferEntry {
            id: "transfer".into(),
            title: "sentinel-secret".into(),
            detail: "sentinel-secret".into(),
            state: crate::models::ScpTransferState::Error,
            created_at_ms: 1,
            updated_at_ms: 2,
            progress_percent: Some(50),
            source: Some("sentinel-secret".into()),
            destination: Some("sentinel-secret".into()),
            session_id: Some("session".into()),
        };
        let value = json!(SafeTransfer::from(&entry));
        assert!(!value.to_string().contains("sentinel-secret"));
        assert_eq!(value["progressPercent"], 50);
        assert_eq!(value["state"], "error");
    }

    #[test]
    fn local_settings_never_expose_cc_credentials() {
        let mut settings = crate::services::SyncSettings::default();
        settings.repo_root = "repo".into();
        settings.cc_agent_token = "cc-secret".into();
        settings.mcp_auth_token = "mcp-secret".into();
        let value = json!(super::super::public_dto::SafeSyncSettings::from(&settings));
        assert!(!value.to_string().contains("cc-secret"));
        assert!(!value.to_string().contains("mcp-secret"));
        assert_eq!(value["repoRoot"], "repo");
    }
}
