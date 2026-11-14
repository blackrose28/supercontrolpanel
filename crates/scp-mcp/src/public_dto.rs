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

impl From<&scp_core::models::AppConfig> for SafeAppConfig {
    fn from(config: &scp_core::models::AppConfig) -> Self {
        use scp_core::models::{BoundaryAuthKind, BoundaryOidcLoginMode};
        Self {
            app: SafeAppRuntime {
                default_terminal_cols: config.app.default_terminal_cols,
                default_terminal_rows: config.app.default_terminal_rows,
            },
            openvpn: SafeOpenVpn {
                username: config.openvpn.username.clone(),
                terminate_open_vpn_when_helper_exits: config
                    .openvpn
                    .terminate_open_vpn_when_helper_exits,
                auto_reconnect_enabled: config.openvpn.auto_reconnect_enabled,
                auto_reconnect_base_seconds: config.openvpn.auto_reconnect_base_seconds,
                auto_reconnect_max_seconds: config.openvpn.auto_reconnect_max_seconds,
            },
            boundary: SafeBoundary {
                addr: config.boundary.addr.clone(),
                auth_method_id: config.boundary.auth_method_id.clone(),
                login_name: config.boundary.login_name.clone(),
                auth_kind: match config.boundary.auth_kind {
                    BoundaryAuthKind::Password => "password",
                    BoundaryAuthKind::Oidc => "oidc",
                }
                .into(),
                oidc_login_mode: match config.boundary.oidc_login_mode {
                    BoundaryOidcLoginMode::Browser => "browser",
                    BoundaryOidcLoginMode::Automated => "automated",
                }
                .into(),
                oidc_username: config.boundary.oidc_username.clone(),
            },
        }
    }
}

impl From<&scp_core::models::SyncSettings> for SafeSyncSettings {
    fn from(settings: &scp_core::models::SyncSettings) -> Self {
        Self {
            repo_root: settings.repo_root.clone(),
            openvpn_executable_path: settings.openvpn_executable_path.clone(),
            openvpn_ovpn_path: settings.openvpn_ovpn_path.clone(),
            boundary_cli_path: settings.boundary_cli_path.clone(),
            caddy_folder: settings.caddy_folder.clone(),
            mcp_bind_address: settings.mcp_bind_address.clone(),
            mcp_sensitive_action_policy: settings.mcp_sensitive_action_policy.clone(),
            cc_server_url: settings.cc_server_url.clone(),
            cc_auth_mode: settings.cc_auth_mode.clone(),
            cc_device_id: settings.cc_device_id.clone(),
            cc_device_name: settings.cc_device_name.clone(),
            mcp_enabled: settings.mcp_enabled,
            mcp_port: settings.mcp_port,
            mcp_allowed_tool_categories: settings.mcp_allowed_tool_categories.clone(),
            cc_enabled: settings.cc_enabled,
            first_run_completed: settings.first_run_completed,
        }
    }
}

impl From<&scp_core::models::FunctionStatus> for SafeFunctionStatus {
    fn from(status: &scp_core::models::FunctionStatus) -> Self {
        Self {
            function_id: status.function_id.clone(),
            state: status.state.clone(),
            message: String::new(),
            boundary_token_source: status.boundary_token_source.clone(),
            session_id: status.session_id.clone(),
            updated_at_ms: status.updated_at_ms,
        }
    }
}

impl From<&scp_core::models::ScpTransferEntry> for SafeTransfer {
    fn from(entry: &scp_core::models::ScpTransferEntry) -> Self {
        use scp_core::models::ScpTransferState;
        Self {
            id: entry.id.clone(),
            title: String::new(),
            detail: String::new(),
            state: match entry.state {
                ScpTransferState::Pending => "pending",
                ScpTransferState::Success => "success",
                ScpTransferState::Error => "error",
            }
            .into(),
            created_at_ms: entry.created_at_ms,
            updated_at_ms: entry.updated_at_ms,
            progress_percent: entry.progress_percent,
            source: None,
            destination: None,
            session_id: entry.session_id.clone(),
        }
    }
}
