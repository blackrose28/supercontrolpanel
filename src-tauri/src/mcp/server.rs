use std::{
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde_json::{json, Value};
use tauri::{AppHandle, Listener, Manager};

use crate::{
    mcp::{
        allowed_mcp_resources, allowed_mcp_tools, call_mcp_tool, read_mcp_resource,
        McpRuntimeStatus,
    },
    services::{config_file_path, ConfigPaths, SyncSettings},
};

struct McpListenerHandle {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

#[derive(Clone)]
pub struct McpRuntimeSnapshot {
    pub enabled: bool,
    pub active: bool,
    pub bind_address: String,
    pub port: u16,
    pub transport: String,
    pub implementation_stage: String,
    pub last_error: Option<String>,
    pub last_started_at_ms: Option<u64>,
}

impl Default for McpRuntimeSnapshot {
    fn default() -> Self {
        Self {
            enabled: false,
            active: false,
            bind_address: "127.0.0.1".to_string(),
            port: 46739,
            transport: "http".to_string(),
            implementation_stage: "skeleton".to_string(),
            last_error: None,
            last_started_at_ms: None,
        }
    }
}

pub struct McpRuntimeState {
    pub snapshot: Mutex<McpRuntimeSnapshot>,
    listener: Mutex<Option<McpListenerHandle>>,
}

impl Default for McpRuntimeState {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(McpRuntimeSnapshot::default()),
            listener: Mutex::new(None),
        }
    }
}

fn is_localhost_bind(bind_address: &str) -> bool {
    matches!(bind_address, "127.0.0.1" | "localhost" | "::1")
}

pub fn mcp_runtime_status(state: &McpRuntimeState) -> Result<McpRuntimeStatus, String> {
    let snapshot = state
        .snapshot
        .lock()
        .map_err(|_| "mcp runtime lock poisoned".to_string())?
        .clone();
    Ok(McpRuntimeStatus {
        enabled: snapshot.enabled,
        active: snapshot.active,
        transport: snapshot.transport,
        bind_address: snapshot.bind_address,
        port: snapshot.port,
        implementation_stage: snapshot.implementation_stage,
        last_error: snapshot.last_error,
        last_started_at_ms: snapshot.last_started_at_ms,
    })
}

pub fn sync_mcp_runtime_from_settings(
    app: &AppHandle,
    state: &McpRuntimeState,
    settings: &SyncSettings,
    now_ms: fn() -> u64,
) -> Result<McpRuntimeStatus, String> {
    let _settings_guard = crate::settings_backup::gate::enter()?;
    stop_listener(state)?;

    {
        let mut snapshot = state
            .snapshot
            .lock()
            .map_err(|_| "mcp runtime lock poisoned".to_string())?;
        snapshot.enabled = settings.mcp_enabled;
        snapshot.bind_address = settings.mcp_bind_address.trim().to_string();
        snapshot.port = settings.mcp_port;
        snapshot.transport = "http".to_string();
        snapshot.last_error = None;
        snapshot.last_started_at_ms = None;
        snapshot.implementation_stage = if settings.mcp_enabled {
            "starting".to_string()
        } else {
            "disabled".to_string()
        };
        snapshot.active = false;
    }

    if !settings.mcp_enabled {
        return mcp_runtime_status(state);
    }

    let bind_address = if settings.mcp_bind_address.trim().is_empty() {
        "127.0.0.1".to_string()
    } else {
        settings.mcp_bind_address.trim().to_string()
    };
    if !is_localhost_bind(&bind_address) {
        let message = format!("MCP bind address must stay localhost-only for now: {bind_address}");
        set_runtime_error(
            state,
            bind_address,
            settings.mcp_port,
            "http-streamable-slice2",
            &message,
        );
        return Err(message);
    }
    let port = if settings.mcp_port == 0 {
        46739
    } else {
        settings.mcp_port
    };
    let token = settings.mcp_auth_token.trim().to_string();
    if token.is_empty() {
        set_runtime_error(
            state,
            bind_address,
            port,
            "http-streamable-slice2",
            "MCP auth token cannot be empty",
        );
        return Err("MCP auth token cannot be empty".to_string());
    }

    let listener = match TcpListener::bind((bind_address.as_str(), port)) {
        Ok(listener) => listener,
        Err(err) => {
            let message = format!("failed to bind MCP listener: {err}");
            set_runtime_error(
                state,
                bind_address,
                port,
                "http-streamable-slice2",
                &message,
            );
            return Err(message);
        }
    };
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("failed to configure MCP listener: {err}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);
    let app_handle = app.clone();
    let thread = thread::spawn(move || {
        run_http_server_loop(app_handle, listener, stop_clone, token);
    });

    {
        let mut slot = state
            .listener
            .lock()
            .map_err(|_| "mcp listener lock poisoned".to_string())?;
        *slot = Some(McpListenerHandle { stop, thread });
    }

    {
        let mut snapshot = state
            .snapshot
            .lock()
            .map_err(|_| "mcp runtime lock poisoned".to_string())?;
        snapshot.enabled = true;
        snapshot.active = true;
        snapshot.bind_address = bind_address;
        snapshot.port = port;
        snapshot.transport = "http".to_string();
        snapshot.implementation_stage = "http-streamable-slice2".to_string();
        snapshot.last_error = None;
        snapshot.last_started_at_ms = Some(now_ms());
    }

    mcp_runtime_status(state)
}

pub fn stop_mcp_runtime(state: &McpRuntimeState) -> Result<McpRuntimeStatus, String> {
    stop_listener(state)?;
    let mut snapshot = state
        .snapshot
        .lock()
        .map_err(|_| "mcp runtime lock poisoned".to_string())?;
    snapshot.active = false;
    snapshot.implementation_stage = if snapshot.enabled {
        "stopped".to_string()
    } else {
        "disabled".to_string()
    };
    Ok(McpRuntimeStatus {
        enabled: snapshot.enabled,
        active: snapshot.active,
        transport: snapshot.transport.clone(),
        bind_address: snapshot.bind_address.clone(),
        port: snapshot.port,
        implementation_stage: snapshot.implementation_stage.clone(),
        last_error: snapshot.last_error.clone(),
        last_started_at_ms: snapshot.last_started_at_ms,
    })
}

fn stop_listener(state: &McpRuntimeState) -> Result<(), String> {
    let handle = {
        let mut slot = state
            .listener
            .lock()
            .map_err(|_| "mcp listener lock poisoned".to_string())?;
        slot.take()
    };
    if let Some(handle) = handle {
        handle.stop.store(true, Ordering::SeqCst);
        let _ = handle.thread.join();
    }
    Ok(())
}

fn set_runtime_error(
    state: &McpRuntimeState,
    bind_address: String,
    port: u16,
    implementation_stage: &str,
    error: &str,
) {
    if let Ok(mut snapshot) = state.snapshot.lock() {
        snapshot.active = false;
        snapshot.bind_address = bind_address;
        snapshot.port = port;
        snapshot.transport = "http".to_string();
        snapshot.implementation_stage = implementation_stage.to_string();
        snapshot.last_error = Some(error.to_string());
    }
}

fn run_http_server_loop(
    app: AppHandle,
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    token: String,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return,
    };
    runtime.block_on(async move {
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(46739);
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(_) => return,
        };
        let host = desktop_protocol_host(app.clone());
        let control = host.control.clone();
        let function_control = control.clone();
        let transfer_control = control.clone();
        let function_listener =
            app.listen("function-status", move |_| function_control.invalidate());
        let transfer_listener = app.listen("scp-transfer-status", move |_| {
            transfer_control.invalidate()
        });
        let router = super::protocol::protocol_router(host, port, token, true);
        // Transport shutdown drops SDK sessions as well; desktop-owned work stays in its stores.
        let signal = control.stop.clone();
        let monitor = tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            signal.cancel();
        });
        super::lifecycle::serve_transport(listener, router, control).await;
        monitor.abort();
        app.unlisten(function_listener);
        app.unlisten(transfer_listener);
    });
    runtime.shutdown_timeout(Duration::from_secs(2));
}

fn desktop_protocol_host(app: AppHandle) -> super::protocol::ProtocolHost {
    let discovery_app = app.clone();
    let execution_app = app.clone();
    super::protocol::ProtocolHost {
        subscriptions: Default::default(),
        control: super::lifecycle::TransportControl::default(),
        permitted: Arc::new(move || {
            allowed_mcp_tools(&discovery_app)
                .map(|tools| tools.into_iter().map(|t| t.name).collect())
                .map_err(super::contracts::classify_host_error)
        }),
        execute: Arc::new(move |name, args| {
            audit_mcp_call("tool.call", name, &args);
            call_mcp_tool(
                &execution_app,
                name,
                args,
                crate::services::now_ms,
                "openvpn-connect",
                "boundary-auth",
                mcp_config_paths,
            )
            .map(|envelope| envelope.data)
            .map_err(super::contracts::classify_host_error)
        }),
        legacy: Arc::new(move |method, path, body| {
            match legacy_request(&app, method, path, body) {
                Ok(payload) => (200, payload),
                Err(error) => (
                    if error == "Not found" { 404 } else { 400 },
                    error_envelope(&error, crate::services::now_ms),
                ),
            }
        }),
    }
}

fn legacy_request(app: &AppHandle, method: &str, path: &str, body: &[u8]) -> Result<Value, String> {
    match (method, path) {
        ("GET", "/health") | ("GET", "/mcp/status") => {
            let state = app.state::<McpRuntimeState>();
            let mut status = mcp_runtime_status(&state)?;
            status.last_error = status
                .last_error
                .map(|_| "MCP listener failed; inspect local desktop diagnostics.".into());
            let payload = serde_json::to_value(status).map_err(|e| e.to_string())?;
            Ok(payload)
        }
        ("GET", "/mcp/tools") => {
            let payload =
                serde_json::to_value(allowed_mcp_tools(app)?).map_err(|e| e.to_string())?;
            Ok(payload)
        }
        ("GET", "/mcp/resources") => {
            let payload =
                serde_json::to_value(allowed_mcp_resources(app)?).map_err(|e| e.to_string())?;
            Ok(payload)
        }
        ("POST", "/mcp/tools/call") => {
            let body: Value = match serde_json::from_slice(body) {
                Ok(body) => body,
                Err(err) => return Err(format!("invalid JSON body: {err}").to_string()),
            };
            let tool_name = match body.get("toolName").and_then(Value::as_str) {
                Some(tool_name) => tool_name,
                None => return Err("toolName is required".to_string()),
            };
            let args = body.get("args").cloned().unwrap_or_else(|| json!({}));
            audit_mcp_call("tool.call", tool_name, &args);
            let envelope = match call_mcp_tool(
                app,
                tool_name,
                args,
                crate::services::now_ms,
                "openvpn-connect",
                "boundary-auth",
                mcp_config_paths,
            ) {
                Ok(envelope) => envelope,
                Err(err) => return Err(err.to_string()),
            };
            let payload = serde_json::to_value(envelope).map_err(|e| e.to_string())?;
            Ok(payload)
        }
        ("POST", "/mcp/resources/read") => {
            let body: Value = match serde_json::from_slice(body) {
                Ok(body) => body,
                Err(err) => return Err(format!("invalid JSON body: {err}").to_string()),
            };
            let uri = match body.get("uri").and_then(Value::as_str) {
                Some(uri) => uri,
                None => return Err("uri is required".to_string()),
            };
            audit_mcp_call("resource.read", uri, &json!({}));
            let envelope = match read_mcp_resource(
                app,
                uri,
                crate::services::now_ms,
                "openvpn-connect",
                "boundary-auth",
                mcp_config_paths,
            ) {
                Ok(envelope) => envelope,
                Err(err) => return Err(err.to_string()),
            };
            let payload = serde_json::to_value(envelope).map_err(|e| e.to_string())?;
            Ok(payload)
        }
        _ => Err("Not found".into()),
    }
}

fn mcp_config_paths(app: &AppHandle) -> Result<ConfigPaths, String> {
    let path = config_file_path(app, "config.json")?;
    Ok(ConfigPaths {
        app_config_path: path.to_string_lossy().into_owned(),
    })
}

fn audit_mcp_call(kind: &str, name: &str, args: &Value) {
    eprintln!("{}", super::contracts::audit_summary(kind, name, args));
}

fn error_envelope(message: &str, now_ms: fn() -> u64) -> Value {
    json!({"ok":false,"message":safe_http_error(message),"data":{},"warnings":[],"timestamp":now_ms()})
}

fn safe_http_error(message: &str) -> String {
    match message {
        "Unauthorized" | "Not found" | "toolName is required" | "uri is required" => message.into(),
        _ => super::contracts::classify_host_error(message.into()).legacy_message(),
    }
}

#[cfg(test)]
mod slice1_tests {
    use super::*;
    #[test]
    fn legacy_http_errors_do_not_echo_runtime_or_parser_secrets() {
        let envelope = error_envelope("keyring error: unlabelled-secret", || 123);
        assert!(!envelope.to_string().contains("unlabelled-secret"));
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["timestamp"], 123);
        assert!(envelope["message"]
            .as_str()
            .unwrap()
            .contains("OPERATION_FAILED"));
    }
}
