use std::{
    env,
    fs,
    path::PathBuf,
    process,
    time::Duration,
};

use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};

const DEFAULT_TIMEOUT_MS: u64 = 5_000;
const APP_IDENTIFIER: &str = "com.chuonglv.supercontrolpanel";

#[derive(Default)]
struct GlobalOptions {
    json: bool,
    timeout_ms: u64,
    endpoint: Option<String>,
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncSettingsFile {
    #[serde(default)]
    mcp_enabled: bool,
    #[serde(default)]
    mcp_bind_address: String,
    #[serde(default)]
    mcp_port: u16,
    #[serde(default)]
    mcp_auth_token: String,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct McpRuntimeStatus {
    enabled: bool,
    active: bool,
    transport: String,
    bind_address: String,
    port: u16,
    implementation_stage: String,
    last_error: Option<String>,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct McpEnvelope {
    ok: bool,
    message: String,
    data: Value,
    warnings: Vec<String>,
    timestamp: u64,
}

struct McpClient {
    endpoint: String,
    token: String,
    client: Client,
}

fn main() {
    match run() {
        Ok(code) => process::exit(code),
        Err((code, message, as_json)) => {
            if as_json {
                let payload = json!({
                    "ok": false,
                    "message": message,
                    "data": {},
                    "warnings": [],
                    "timestamp": 0
                });
                eprintln!("{}", serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()));
            } else {
                eprintln!("{message}");
            }
            process::exit(code);
        }
    }
}

fn run() -> Result<i32, (i32, String, bool)> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let options = parse_global_options(&mut args)?;

    if args.is_empty() {
        return Err((1, usage(), options.json));
    }

    let client = McpClient::from_options(&options)?;

    let result = match args[0].as_str() {
        "status" => print_envelope(options.json, client.call_tool("app_get_overview", json!({}))?),
        "function" => handle_function_command(&client, &args[1..], options.json),
        "openvpn" => handle_openvpn_command(&client, &args[1..], options.json),
        "boundary" => handle_boundary_command(&client, &args[1..], options.json),
        "server" => handle_server_command(&client, &args[1..], options.json),
        "config" => handle_config_command(&client, &args[1..], options.json),
        "mcp" => handle_mcp_command(&client, &args[1..], options.json),
        _ => return Err((1, usage(), options.json)),
    };
    result?;

    Ok(0)
}

fn parse_global_options(args: &mut Vec<String>) -> Result<GlobalOptions, (i32, String, bool)> {
    let mut options = GlobalOptions {
        json: false,
        timeout_ms: DEFAULT_TIMEOUT_MS,
        endpoint: None,
        token: None,
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                options.json = true;
                args.remove(index);
            }
            "--timeout-ms" => {
                let value = args.get(index + 1).cloned().ok_or_else(|| {
                    (1, "--timeout-ms requires a value".to_string(), options.json)
                })?;
                options.timeout_ms = value.parse::<u64>().map_err(|_| {
                    (1, "--timeout-ms must be a number".to_string(), options.json)
                })?;
                args.drain(index..=index + 1);
            }
            "--endpoint" => {
                let value = args.get(index + 1).cloned().ok_or_else(|| {
                    (1, "--endpoint requires a value".to_string(), options.json)
                })?;
                options.endpoint = Some(value);
                args.drain(index..=index + 1);
            }
            "--token" => {
                let value = args.get(index + 1).cloned().ok_or_else(|| {
                    (1, "--token requires a value".to_string(), options.json)
                })?;
                options.token = Some(value);
                args.drain(index..=index + 1);
            }
            "--quiet" | "--verbose" => {
                args.remove(index);
            }
            _ => index += 1,
        }
    }
    Ok(options)
}

impl McpClient {
    fn from_options(options: &GlobalOptions) -> Result<Self, (i32, String, bool)> {
        let endpoint = if let Some(endpoint) = &options.endpoint {
            endpoint.clone()
        } else if let Ok(endpoint) = env::var("SUPERCONTROLPANEL_MCP_ENDPOINT") {
            endpoint
        } else {
            let settings = load_sync_settings().map_err(|err| (1, err, options.json))?;
            let bind = if settings.mcp_bind_address.trim().is_empty() {
                "127.0.0.1".to_string()
            } else {
                settings.mcp_bind_address
            };
            let port = if settings.mcp_port == 0 { 46739 } else { settings.mcp_port };
            if !settings.mcp_enabled {
                return Err((4, "MCP is disabled in sync-local.json".to_string(), options.json));
            }
            format!("http://{bind}:{port}")
        };

        let token = if let Some(token) = &options.token {
            token.clone()
        } else if let Ok(token) = env::var("SUPERCONTROLPANEL_MCP_TOKEN") {
            token
        } else {
            let settings = load_sync_settings().map_err(|err| (1, err, options.json))?;
            if settings.mcp_auth_token.trim().is_empty() {
                return Err((4, "MCP auth token is empty in sync-local.json".to_string(), options.json));
            }
            settings.mcp_auth_token
        };

        let client = Client::builder()
            .timeout(Duration::from_millis(options.timeout_ms))
            .build()
            .map_err(|err| (2, format!("failed to build HTTP client: {err}"), options.json))?;

        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            token,
            client,
        })
    }

    fn call_tool(&self, tool_name: &str, args: Value) -> Result<McpEnvelope, (i32, String, bool)> {
        let response = self
            .client
            .post(format!("{}/mcp/tools/call", self.endpoint))
            .bearer_auth(&self.token)
            .json(&json!({
                "toolName": tool_name,
                "args": args,
            }))
            .send()
            .map_err(map_http_err)?;

        parse_envelope_response(response)
    }

    fn get_status(&self) -> Result<McpRuntimeStatus, (i32, String, bool)> {
        let response = self
            .client
            .get(format!("{}/mcp/status", self.endpoint))
            .bearer_auth(&self.token)
            .send()
            .map_err(map_http_err)?;

        if response.status().as_u16() == 401 {
            return Err((2, "Unauthorized".to_string(), false));
        }
        if !response.status().is_success() {
            return Err((2, format!("MCP status request failed: {}", response.status()), false));
        }
        response
            .json::<McpRuntimeStatus>()
            .map_err(|err| (2, format!("failed to decode MCP status: {err}"), false))
    }
}

fn parse_envelope_response(response: reqwest::blocking::Response) -> Result<McpEnvelope, (i32, String, bool)> {
    let status = response.status();
    let text = response
        .text()
        .map_err(|err| (2, format!("failed to read MCP response: {err}"), false))?;

    let envelope = serde_json::from_str::<McpEnvelope>(&text)
        .map_err(|err| (2, format!("invalid MCP response: {err}"), false))?;

    if status.as_u16() == 401 {
        return Err((2, envelope.message, false));
    }
    if !status.is_success() {
        return Err((3, envelope.message, false));
    }
    if !envelope.ok {
        return Err((3, envelope.message, false));
    }
    Ok(envelope)
}

fn map_http_err(err: reqwest::Error) -> (i32, String, bool) {
    if err.is_connect() {
        (4, format!("failed to connect to MCP endpoint: {err}"), false)
    } else if err.is_timeout() {
        (2, format!("MCP request timed out: {err}"), false)
    } else {
        (2, format!("MCP HTTP request failed: {err}"), false)
    }
}

fn handle_function_command(client: &McpClient, args: &[String], as_json: bool) -> Result<(), (i32, String, bool)> {
    match args {
        [sub] if sub == "list" => print_envelope(as_json, client.call_tool("function_status_list", json!({}))?),
        [sub, flag, function_id] if sub == "get" && flag == "--function-id" => {
            print_envelope(as_json, client.call_tool("function_status_get", json!({ "functionId": function_id }))?)
        }
        _ => Err((1, "Usage: supercontrolpanelhelper function list|get --function-id <id>".to_string(), as_json)),
    }
}

fn handle_openvpn_command(client: &McpClient, args: &[String], as_json: bool) -> Result<(), (i32, String, bool)> {
    match args {
        [sub] if sub == "status" => print_envelope(as_json, client.call_tool("openvpn_get_status", json!({}))?),
        [sub] if sub == "start" => print_envelope(as_json, client.call_tool("openvpn_start", json!({}))?),
        [sub] if sub == "stop" => print_envelope(as_json, client.call_tool("openvpn_stop", json!({}))?),
        _ => Err((1, "Usage: supercontrolpanelhelper openvpn status|start|stop".to_string(), as_json)),
    }
}

fn handle_boundary_command(client: &McpClient, args: &[String], as_json: bool) -> Result<(), (i32, String, bool)> {
    match args {
        [sub] if sub == "status" => print_envelope(as_json, client.call_tool("boundary_get_status", json!({}))?),
        [first, second, third] if first == "login" && second == "start" && third == "--hidden" => {
            print_envelope(as_json, client.call_tool("boundary_auth_start", json!({ "hidden": true }))?)
        }
        [first, second] if first == "login" && second == "start" => {
            print_envelope(as_json, client.call_tool("boundary_auth_start", json!({}))?)
        }
        [first, second] if first == "login" && second == "stop" => {
            print_envelope(as_json, client.call_tool("boundary_auth_stop", json!({}))?)
        }
        _ => Err((1, "Usage: supercontrolpanelhelper boundary status|login start [--hidden]|login stop".to_string(), as_json)),
    }
}

fn handle_server_command(client: &McpClient, args: &[String], as_json: bool) -> Result<(), (i32, String, bool)> {
    match args {
        [sub] if sub == "list" => print_envelope(as_json, client.call_tool("servers_list", json!({}))?),
        [sub, flag, server_id] if sub == "get" && flag == "--server-id" => {
            print_envelope(as_json, client.call_tool("server_get", json!({ "serverId": server_id }))?)
        }
        [group, action, flag, server_id] if group == "tunnel" && flag == "--server-id" => {
            let tool = if action == "start" { "server_tunnel_start" } else if action == "stop" { "server_tunnel_stop" } else {
                return Err((1, "Usage: supercontrolpanelhelper server tunnel start|stop --server-id <id>".to_string(), as_json));
            };
            print_envelope(as_json, client.call_tool(tool, json!({ "serverId": server_id }))?)
        }
        [group, action, flag, server_id] if group == "ssh" && flag == "--server-id" => {
            let tool = if action == "start" { "server_ssh_start" } else if action == "stop" { "server_ssh_stop" } else {
                return Err((1, "Usage: supercontrolpanelhelper server ssh start|stop --server-id <id>".to_string(), as_json));
            };
            print_envelope(as_json, client.call_tool(tool, json!({ "serverId": server_id }))?)
        }
        _ => Err((1, "Usage: supercontrolpanelhelper server list|get|tunnel|ssh ...".to_string(), as_json)),
    }
}

fn handle_config_command(client: &McpClient, args: &[String], as_json: bool) -> Result<(), (i32, String, bool)> {
    match args {
        [sub] if sub == "paths" => print_envelope(as_json, client.call_tool("config_get_paths", json!({}))?),
        [first, second, flag, repo_root] if first == "sync" && (second == "pull" || second == "push") && flag == "--repo-root" => {
            let tool = if second == "pull" { "config_sync_pull" } else { "config_sync_push" };
            print_envelope(as_json, client.call_tool(tool, json!({ "repoRoot": repo_root }))?)
        }
        _ => Err((1, "Usage: supercontrolpanelhelper config paths|sync pull --repo-root <path>|sync push --repo-root <path>".to_string(), as_json)),
    }
}

fn handle_mcp_command(client: &McpClient, args: &[String], as_json: bool) -> Result<(), (i32, String, bool)> {
    match args {
        [sub] if sub == "ping" => {
            let status = client.get_status()?;
            if as_json {
                println!("{}", serde_json::to_string_pretty(&json!({ "ok": true, "status": status })).unwrap());
            } else {
                println!("MCP OK: {}://{}:{} ({})", status.transport, status.bind_address, status.port, status.implementation_stage);
            }
            Ok(())
        }
        [sub] if sub == "info" => {
            let status = client.get_status()?;
            if as_json {
                println!("{}", serde_json::to_string_pretty(&status).unwrap());
            } else {
                println!("MCP enabled: {}", status.enabled);
                println!("MCP active: {}", status.active);
                println!("Bind: {}:{}", status.bind_address, status.port);
                println!("Stage: {}", status.implementation_stage);
                if let Some(last_error) = status.last_error {
                    println!("Last error: {last_error}");
                }
            }
            Ok(())
        }
        _ => Err((1, "Usage: supercontrolpanelhelper mcp ping|info".to_string(), as_json)),
    }
}

fn print_envelope(as_json: bool, envelope: McpEnvelope) -> Result<(), (i32, String, bool)> {
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&envelope)
                .map_err(|err| (1, format!("failed to encode JSON output: {err}"), as_json))?
        );
        return Ok(());
    }

    println!("{}", envelope.message);
    if !envelope.data.is_null() && envelope.data != json!({}) {
        println!(
            "{}",
            serde_json::to_string_pretty(&envelope.data)
                .map_err(|err| (1, format!("failed to format output: {err}"), as_json))?
        );
    }
    Ok(())
}

fn load_sync_settings() -> Result<SyncSettingsFile, String> {
    let path = sync_settings_path()?;
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.to_string_lossy()))?;
    serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", path.to_string_lossy()))
}

fn sync_settings_path() -> Result<PathBuf, String> {
    if let Ok(path) = env::var("SUPERCONTROLPANEL_SYNC_LOCAL_PATH") {
        return Ok(PathBuf::from(path));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = env::var("APPDATA").map_err(|_| "APPDATA is not set".to_string())?;
        return Ok(PathBuf::from(appdata).join(APP_IDENTIFIER).join("sync-local.json"));
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(xdg).join(APP_IDENTIFIER).join("sync-local.json"));
        }
        let home = env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
        Ok(PathBuf::from(home)
            .join(".config")
            .join(APP_IDENTIFIER)
            .join("sync-local.json"))
    }
}

fn usage() -> String {
    [
        "Usage: supercontrolpanelhelper [--json] [--timeout-ms N] [--endpoint URL] [--token TOKEN] <command>",
        "",
        "Commands:",
        "  status",
        "  function list",
        "  function get --function-id <id>",
        "  openvpn status|start|stop",
        "  boundary status",
        "  boundary login start [--hidden]",
        "  boundary login stop",
        "  server list",
        "  server get --server-id <id>",
        "  server tunnel start|stop --server-id <id>",
        "  server ssh start|stop --server-id <id>",
        "  config paths",
        "  config sync pull|push --repo-root <path>",
        "  mcp ping|info",
    ]
    .join("\n")
}
