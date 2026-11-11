//! Explicit public output types. Runtime models must be converted field by field in the desktop host.
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SafeAppRuntime {
    pub default_terminal_cols: u16,
    pub default_terminal_rows: u16,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SafeOpenVpn {
    pub username: String,
    pub terminate_open_vpn_when_helper_exits: bool,
    pub auto_reconnect_enabled: bool,
    pub auto_reconnect_base_seconds: u64,
    pub auto_reconnect_max_seconds: u64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SafeBoundary {
    pub addr: String,
    pub auth_method_id: String,
    pub login_name: String,
    pub auth_kind: String,
    pub oidc_login_mode: String,
    pub oidc_username: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SafeAppConfig {
    pub app: SafeAppRuntime,
    pub openvpn: SafeOpenVpn,
    pub boundary: SafeBoundary,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SafeSyncSettings {
    pub repo_root: String,
    pub openvpn_executable_path: String,
    pub openvpn_ovpn_path: String,
    pub boundary_cli_path: String,
    pub caddy_folder: String,
    pub mcp_bind_address: String,
    pub mcp_sensitive_action_policy: String,
    pub cc_server_url: String,
    pub cc_auth_mode: String,
    pub cc_device_id: String,
    pub cc_device_name: String,
    pub mcp_enabled: bool,
    pub mcp_port: u16,
    pub mcp_allowed_tool_categories: Vec<String>,
    pub cc_enabled: bool,
    pub first_run_completed: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SafeFunctionStatus {
    pub function_id: String,
    pub state: String,
    pub message: String,
    pub boundary_token_source: Option<String>,
    pub session_id: Option<String>,
    pub updated_at_ms: u64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SafeTransfer {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub state: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub progress_percent: Option<u8>,
    pub source: Option<String>,
    pub destination: Option<String>,
    pub session_id: Option<String>,
}

impl SafeAppRuntime {
    pub fn schema() -> serde_json::Value {
        super::validation::output_object(serde_json::json!({
            "defaultTerminalCols": serde_json::json!({"type":"integer","minimum":0}),
            "defaultTerminalRows": serde_json::json!({"type":"integer","minimum":0}),
        }))
    }
}

impl SafeOpenVpn {
    pub fn schema() -> serde_json::Value {
        super::validation::output_object(serde_json::json!({
            "username": serde_json::json!({"type":"string"}),
            "terminateOpenVpnWhenHelperExits": serde_json::json!({"type":"boolean"}),
            "autoReconnectEnabled": serde_json::json!({"type":"boolean"}),
            "autoReconnectBaseSeconds": serde_json::json!({"type":"integer","minimum":0}),
            "autoReconnectMaxSeconds": serde_json::json!({"type":"integer","minimum":0}),
        }))
    }
}

impl SafeBoundary {
    pub fn schema() -> serde_json::Value {
        super::validation::output_object(serde_json::json!({
            "addr": serde_json::json!({"type":"string"}),
            "authMethodId": serde_json::json!({"type":"string"}),
            "loginName": serde_json::json!({"type":"string"}),
            "authKind": serde_json::json!({"type":"string"}),
            "oidcLoginMode": serde_json::json!({"type":"string"}),
            "oidcUsername": serde_json::json!({"type":"string"}),
        }))
    }
}

impl SafeAppConfig {
    pub fn schema() -> serde_json::Value {
        super::validation::output_object(serde_json::json!({
            "app": SafeAppRuntime::schema(),
            "openvpn": SafeOpenVpn::schema(),
            "boundary": SafeBoundary::schema(),
        }))
    }
}

impl SafeSyncSettings {
    pub fn schema() -> serde_json::Value {
        super::validation::output_object(serde_json::json!({
            "repoRoot": serde_json::json!({"type":"string"}),
            "openvpnExecutablePath": serde_json::json!({"type":"string"}),
            "openvpnOvpnPath": serde_json::json!({"type":"string"}),
            "boundaryCliPath": serde_json::json!({"type":"string"}),
            "caddyFolder": serde_json::json!({"type":"string"}),
            "mcpBindAddress": serde_json::json!({"type":"string"}),
            "mcpSensitiveActionPolicy": serde_json::json!({"type":"string"}),
            "ccServerUrl": serde_json::json!({"type":"string"}),
            "ccAuthMode": serde_json::json!({"type":"string"}),
            "ccDeviceId": serde_json::json!({"type":"string"}),
            "ccDeviceName": serde_json::json!({"type":"string"}),
            "mcpEnabled": serde_json::json!({"type":"boolean"}),
            "mcpPort": serde_json::json!({"type":"integer","minimum":0}),
            "mcpAllowedToolCategories": serde_json::json!({"type":"array","items":serde_json::json!({"type":"string"})}),
            "ccEnabled": serde_json::json!({"type":"boolean"}),
            "firstRunCompleted": serde_json::json!({"type":"boolean"}),
        }))
    }
}

impl SafeFunctionStatus {
    pub fn schema() -> serde_json::Value {
        super::validation::output_object(serde_json::json!({
            "functionId": serde_json::json!({"type":"string"}),
            "state": serde_json::json!({"type":"string"}),
            "message": serde_json::json!({"type":"string"}),
            "boundaryTokenSource": serde_json::json!({"anyOf":[serde_json::json!({"type":"string"}),{"type":"null"}]}),
            "sessionId": serde_json::json!({"anyOf":[serde_json::json!({"type":"string"}),{"type":"null"}]}),
            "updatedAtMs": serde_json::json!({"type":"integer","minimum":0}),
        }))
    }
}

impl SafeTransfer {
    pub fn schema() -> serde_json::Value {
        super::validation::output_object(serde_json::json!({
            "id": serde_json::json!({"type":"string"}),
            "title": serde_json::json!({"type":"string"}),
            "detail": serde_json::json!({"type":"string"}),
            "state": serde_json::json!({"type":"string"}),
            "createdAtMs": serde_json::json!({"type":"integer","minimum":0}),
            "updatedAtMs": serde_json::json!({"type":"integer","minimum":0}),
            "progressPercent": serde_json::json!({"anyOf":[serde_json::json!({"type":"integer","minimum":0}),{"type":"null"}]}),
            "source": serde_json::json!({"anyOf":[serde_json::json!({"type":"string"}),{"type":"null"}]}),
            "destination": serde_json::json!({"anyOf":[serde_json::json!({"type":"string"}),{"type":"null"}]}),
            "sessionId": serde_json::json!({"anyOf":[serde_json::json!({"type":"string"}),{"type":"null"}]}),
        }))
    }
}
