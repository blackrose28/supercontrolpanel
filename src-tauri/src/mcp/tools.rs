use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::{
    mcp::{McpToolCallEnvelope, McpToolDescriptor},
    server_mgmt::load_servers,
    services::ConfigPaths,
    stores::{BoundaryAutomationStore, FunctionStore, OpenVpnAdminStore, RemoteBrowserStore, SessionStore},
    start_boundary_auth_action, start_caddy_action, start_openvpn_action, start_server_ssh_action,
    start_server_tunnel_action, stop_boundary_auth_action, stop_caddy_action,
    stop_openvpn_action, stop_server_ssh_action, stop_server_tunnel_action,
};

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
            let username = args.get("username").and_then(Value::as_str).unwrap_or("").to_string();
            let password = args.get("password").and_then(Value::as_str).unwrap_or("").to_string();
            let result = start_openvpn_action(
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
            let server_id = args.get("serverId").and_then(Value::as_str).ok_or_else(|| "serverId is required".to_string())?.to_string();
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
            let server_id = args.get("serverId").and_then(Value::as_str).ok_or_else(|| "serverId is required".to_string())?.to_string();
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
            let server_id = args.get("serverId").and_then(Value::as_str).ok_or_else(|| "serverId is required".to_string())?.to_string();
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
            let server_id = args.get("serverId").and_then(Value::as_str).ok_or_else(|| "serverId is required".to_string())?.to_string();
            stop_server_ssh_action(
                server_id,
                app.clone(),
                app.state::<SessionStore>(),
                app.state::<FunctionStore>(),
            )?;
            json!({})
        }
        "caddy_start" => {
            let folder = args.get("folder").and_then(Value::as_str).ok_or_else(|| "folder is required".to_string())?.to_string();
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
