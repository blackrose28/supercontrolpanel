use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::{
    mcp::{call_mcp_tool, list_mcp_resources, list_mcp_tools, read_mcp_resource, McpRuntimeStatus},
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

struct SimpleHttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
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
            "http-minimal",
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
            "http-minimal",
            "MCP auth token cannot be empty",
        );
        return Err("MCP auth token cannot be empty".to_string());
    }

    let listener = match TcpListener::bind((bind_address.as_str(), port)) {
        Ok(listener) => listener,
        Err(err) => {
            let message = format!("failed to bind MCP listener: {err}");
            set_runtime_error(state, bind_address, port, "http-minimal", &message);
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
        snapshot.implementation_stage = "http-minimal".to_string();
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
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let _ = handle_http_connection(&app, &mut stream, &token);
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle_http_connection(
    app: &AppHandle,
    stream: &mut TcpStream,
    token: &str,
) -> Result<(), String> {
    let request = match read_http_request(stream) {
        Ok(request) => request,
        Err(err) => return write_error_envelope(stream, 400, &err, crate::services::now_ms),
    };
    if !is_authorized(&request, token) {
        return write_error_envelope(stream, 401, "Unauthorized", crate::services::now_ms);
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/health") | ("GET", "/mcp/status") => {
            let state = app.state::<McpRuntimeState>();
            let payload =
                serde_json::to_value(mcp_runtime_status(&state)?).map_err(|e| e.to_string())?;
            write_json_response(stream, 200, payload)
        }
        ("GET", "/mcp/tools") => {
            let payload = serde_json::to_value(list_mcp_tools()).map_err(|e| e.to_string())?;
            write_json_response(stream, 200, payload)
        }
        ("GET", "/mcp/resources") => {
            let payload = serde_json::to_value(list_mcp_resources()).map_err(|e| e.to_string())?;
            write_json_response(stream, 200, payload)
        }
        ("POST", "/mcp/tools/call") => {
            let body: Value = match serde_json::from_slice(&request.body) {
                Ok(body) => body,
                Err(err) => {
                    return write_error_envelope(
                        stream,
                        400,
                        &format!("invalid JSON body: {err}"),
                        crate::services::now_ms,
                    )
                }
            };
            let tool_name = match body.get("toolName").and_then(Value::as_str) {
                Some(tool_name) => tool_name,
                None => {
                    return write_error_envelope(
                        stream,
                        400,
                        "toolName is required",
                        crate::services::now_ms,
                    )
                }
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
                Err(err) => {
                    return write_error_envelope(stream, 400, &err, crate::services::now_ms)
                }
            };
            let payload = serde_json::to_value(envelope).map_err(|e| e.to_string())?;
            write_json_response(stream, 200, payload)
        }
        ("POST", "/mcp/resources/read") => {
            let body: Value = match serde_json::from_slice(&request.body) {
                Ok(body) => body,
                Err(err) => {
                    return write_error_envelope(
                        stream,
                        400,
                        &format!("invalid JSON body: {err}"),
                        crate::services::now_ms,
                    )
                }
            };
            let uri = match body.get("uri").and_then(Value::as_str) {
                Some(uri) => uri,
                None => {
                    return write_error_envelope(
                        stream,
                        400,
                        "uri is required",
                        crate::services::now_ms,
                    )
                }
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
                Err(err) => {
                    return write_error_envelope(stream, 400, &err, crate::services::now_ms)
                }
            };
            let payload = serde_json::to_value(envelope).map_err(|e| e.to_string())?;
            write_json_response(stream, 200, payload)
        }
        _ => write_error_envelope(stream, 404, "Not found", crate::services::now_ms),
    }
}

fn mcp_config_paths(app: &AppHandle) -> Result<ConfigPaths, String> {
    let path = config_file_path(app, "config.json")?;
    Ok(ConfigPaths {
        app_config_path: path.to_string_lossy().into_owned(),
    })
}

fn is_authorized(request: &SimpleHttpRequest, token: &str) -> bool {
    request
        .headers
        .get("authorization")
        .map(|value| value.trim() == format!("Bearer {token}"))
        .unwrap_or(false)
}

fn audit_mcp_call(kind: &str, name: &str, args: &Value) {
    eprintln!(
        "[MCP audit] kind={kind} name={name} args={}",
        summarize_args(args)
    );
}

fn summarize_args(args: &Value) -> String {
    let mut value = args.clone();
    redact_sensitive_fields(&mut value);
    serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
}

fn redact_sensitive_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if matches!(
                    key.as_str(),
                    "password" | "otpSecret" | "oidcPassword" | "oidcOtpSecret" | "mcpAuthToken"
                ) {
                    *entry = Value::String("[redacted]".to_string());
                } else {
                    redact_sensitive_fields(entry);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_sensitive_fields(item);
            }
        }
        _ => {}
    }
}

fn read_http_request(stream: &mut TcpStream) -> Result<SimpleHttpRequest, String> {
    let mut buf = Vec::new();
    let mut temp = [0u8; 4096];
    let mut header_end = None;
    let mut content_length = 0usize;

    loop {
        let n = stream
            .read(&mut temp)
            .map_err(|e| format!("failed to read request: {e}"))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&temp[..n]);
        if header_end.is_none() {
            header_end = find_header_end(&buf);
            if let Some(end) = header_end {
                content_length = parse_content_length(&buf[..end]);
                if buf.len() >= end + content_length {
                    break;
                }
            }
        } else if let Some(end) = header_end {
            if buf.len() >= end + content_length {
                break;
            }
        }
        if buf.len() > 1024 * 1024 {
            return Err("request too large".to_string());
        }
    }

    let header_end = header_end.ok_or_else(|| "invalid HTTP request".to_string())?;
    let headers_raw = &buf[..header_end];
    let body = buf[header_end..].to_vec();
    let header_text = String::from_utf8(headers_raw.to_vec())
        .map_err(|e| format!("request headers were not valid UTF-8: {e}"))?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "missing request line".to_string())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| "missing HTTP method".to_string())?
        .to_string();
    let path = request_parts
        .next()
        .ok_or_else(|| "missing HTTP path".to_string())?
        .to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    Ok(SimpleHttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| idx + 4)
}

fn parse_content_length(headers_raw: &[u8]) -> usize {
    let header_text = String::from_utf8_lossy(headers_raw);
    for line in header_text.lines() {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                return value.trim().parse::<usize>().unwrap_or(0);
            }
        }
    }
    0
}

fn write_json_response(stream: &mut TcpStream, status: u16, payload: Value) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let body =
        serde_json::to_vec(&payload).map_err(|e| format!("failed to encode JSON response: {e}"))?;
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .map_err(|e| format!("failed to write HTTP response: {e}"))
}

fn write_error_envelope(
    stream: &mut TcpStream,
    status: u16,
    message: &str,
    now_ms: fn() -> u64,
) -> Result<(), String> {
    write_json_response(
        stream,
        status,
        json!({
            "ok": false,
            "message": message,
            "data": {},
            "warnings": [],
            "timestamp": now_ms(),
        }),
    )
}
