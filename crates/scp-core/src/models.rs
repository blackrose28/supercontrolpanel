use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub const DESKTOP_APP_IDENTIFIER: &str = "com.chuonglv.supercontrolpanel";
pub const OPENVPN_FUNCTION_ID: &str = "openvpn-connect";
pub const CADDY_FUNCTION_ID: &str = "caddy-start";
pub const BOUNDARY_AUTH_FUNCTION_ID: &str = "boundary-auth";

/// OS credential store service name shared across desktop and standalone namespaces.
pub const KEYRING_SERVICE: &str = "com.example.supercontrolpanel";
pub const KEYRING_OPENVPN_PASSWORD: &str = "openvpn_password";
pub const KEYRING_OPENVPN_OTP_SECRET: &str = "openvpn_otp_secret";
pub const KEYRING_BOUNDARY_PASSWORD: &str = "boundary_password";
pub const KEYRING_BOUNDARY_OIDC_PASSWORD: &str = "boundary_oidc_password";
pub const KEYRING_BOUNDARY_OIDC_OTP_SECRET: &str = "boundary_oidc_otp_secret";
pub const KEYRING_BOUNDARY_APP_TOKEN: &str = "boundary_app_token";
pub const KEYRING_CC_TELEMETRY_QUEUE_KEY: &str = "cc-telemetry-queue-key";

pub const STATIC_KEYRING_ACCOUNTS: &[&str] = &[
    KEYRING_OPENVPN_PASSWORD,
    KEYRING_OPENVPN_OTP_SECRET,
    KEYRING_BOUNDARY_PASSWORD,
    KEYRING_BOUNDARY_OIDC_PASSWORD,
    KEYRING_BOUNDARY_OIDC_OTP_SECRET,
    KEYRING_BOUNDARY_APP_TOKEN,
    KEYRING_CC_TELEMETRY_QUEUE_KEY,
];

pub fn keyring_server_ssh_password_account(server_id: &str) -> String {
    format!("server_ssh_password:{server_id}")
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub app: AppRuntimeConfig,
    #[serde(default)]
    pub openvpn: OpenVpnConfig,
    #[serde(default)]
    pub boundary: BoundaryConfig,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoundaryAuthKind {
    Password,
    #[default]
    Oidc,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoundaryOidcLoginMode {
    #[default]
    Browser,
    Automated,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryConfig {
    #[serde(default)]
    pub cli_path: String,
    #[serde(default)]
    pub addr: String,
    #[serde(default)]
    pub auth_method_id: String,
    #[serde(default)]
    pub login_name: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub auth_kind: BoundaryAuthKind,
    #[serde(default)]
    pub oidc_login_mode: BoundaryOidcLoginMode,
    #[serde(default)]
    pub oidc_username: String,
    #[serde(default)]
    pub oidc_password: String,
    #[serde(default)]
    pub oidc_otp_secret: String,
    #[serde(default)]
    pub connect_ssh_options: String,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryConfigForDisk {
    #[serde(default)]
    pub addr: String,
    #[serde(default)]
    pub auth_method_id: String,
    #[serde(default)]
    pub login_name: String,
    #[serde(default)]
    pub auth_kind: BoundaryAuthKind,
    #[serde(default)]
    pub oidc_login_mode: BoundaryOidcLoginMode,
    #[serde(default)]
    pub oidc_username: String,
    #[serde(default)]
    pub connect_ssh_options: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppRuntimeConfig {
    pub default_terminal_cols: u16,
    pub default_terminal_rows: u16,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub executable_path: String,
    #[serde(default)]
    pub ovpn_path: String,
    #[serde(default)]
    pub extra_args: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub otp_secret: String,
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    pub terminate_open_vpn_when_helper_exits: bool,
    #[serde(default)]
    pub auto_reconnect_enabled: bool,
    #[serde(default = "default_openvpn_reconnect_base_seconds")]
    pub auto_reconnect_base_seconds: u64,
    #[serde(default = "default_openvpn_reconnect_max_seconds")]
    pub auto_reconnect_max_seconds: u64,
}

pub fn default_openvpn_reconnect_base_seconds() -> u64 {
    5
}

pub fn default_openvpn_reconnect_max_seconds() -> u64 {
    3600
}

#[cfg(target_os = "windows")]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnLaunchConfig {
    pub executable_path: String,
    pub args: String,
    pub username: String,
    pub password: String,
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    pub terminate_open_vpn_when_helper_exits: bool,
    #[serde(default)]
    pub auto_reconnect_enabled: bool,
    #[serde(default = "default_openvpn_reconnect_base_seconds")]
    pub auto_reconnect_base_seconds: u64,
    #[serde(default = "default_openvpn_reconnect_max_seconds")]
    pub auto_reconnect_max_seconds: u64,
}

#[cfg(target_os = "windows")]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperStatusFile {
    pub state: String,
    pub message: String,
    pub updated_at_ms: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnConfigForDisk {
    pub command: String,
    pub extra_args: String,
    pub username: String,
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    pub terminate_open_vpn_when_helper_exits: bool,
    #[serde(default)]
    pub auto_reconnect_enabled: bool,
    #[serde(default = "default_openvpn_reconnect_base_seconds")]
    pub auto_reconnect_base_seconds: u64,
    #[serde(default = "default_openvpn_reconnect_max_seconds")]
    pub auto_reconnect_max_seconds: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfigForDisk {
    pub app: AppRuntimeConfig,
    pub openvpn: OpenVpnConfigForDisk,
    #[serde(default)]
    pub boundary: BoundaryConfigForDisk,
}

impl From<&AppConfig> for AppConfigForDisk {
    fn from(c: &AppConfig) -> Self {
        Self {
            app: c.app.clone(),
            openvpn: OpenVpnConfigForDisk {
                command: String::new(),
                extra_args: c.openvpn.extra_args.clone(),
                username: c.openvpn.username.clone(),
                terminate_open_vpn_when_helper_exits: c
                    .openvpn
                    .terminate_open_vpn_when_helper_exits,
                auto_reconnect_enabled: c.openvpn.auto_reconnect_enabled,
                auto_reconnect_base_seconds: c.openvpn.auto_reconnect_base_seconds,
                auto_reconnect_max_seconds: c.openvpn.auto_reconnect_max_seconds,
            },
            boundary: BoundaryConfigForDisk {
                addr: c.boundary.addr.clone(),
                auth_method_id: c.boundary.auth_method_id.clone(),
                login_name: c.boundary.login_name.clone(),
                auth_kind: c.boundary.auth_kind,
                oidc_login_mode: c.boundary.oidc_login_mode,
                oidc_username: c.boundary.oidc_username.clone(),
                connect_ssh_options: c.boundary.connect_ssh_options.clone(),
            },
        }
    }
}

pub fn default_app_config() -> AppConfig {
    AppConfig {
        app: AppRuntimeConfig {
            default_terminal_cols: 120,
            default_terminal_rows: 36,
        },
        openvpn: OpenVpnConfig {
            auto_reconnect_base_seconds: default_openvpn_reconnect_base_seconds(),
            auto_reconnect_max_seconds: default_openvpn_reconnect_max_seconds(),
            ..OpenVpnConfig::default()
        },
        boundary: BoundaryConfig::default(),
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PtyOutputPayload {
    pub id: String,
    pub data: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PtyExitPayload {
    pub id: String,
    pub exit_code: Option<i32>,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FunctionStatus {
    pub function_id: String,
    pub state: String,
    pub message: String,
    pub boundary_token_source: Option<String>,
    pub session_id: Option<String>,
    pub updated_at_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StartFunctionResult {
    pub function_id: String,
    pub session_id: String,
}

#[cfg(target_os = "windows")]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnLogPayload {
    pub function_id: String,
    pub session_id: String,
    pub data: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScpTransferState {
    Pending,
    Success,
    Error,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScpTransferEntry {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub state: ScpTransferState,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub progress_percent: Option<u8>,
    pub source: Option<String>,
    pub destination: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ServerOs {
    #[default]
    Linux,
    Windows,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionMethod {
    Ssh,
    Boundary,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ServerService {
    pub name: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub os: ServerOs,
    pub connection_method: ConnectionMethod,
    pub ssh_port: u16,
    pub ssh_user: String,
    #[serde(default)]
    pub boundary_target_id: String,
    #[serde(default)]
    pub boundary_listen_port: u16,
    #[serde(default)]
    pub services: Vec<ServerService>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServersFile {
    #[serde(default)]
    pub servers: Vec<Server>,
}

pub fn new_server_id() -> String {
    format!("srv-{}", Uuid::new_v4())
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncSettings {
    #[serde(default)]
    pub repo_root: String,
    #[serde(default)]
    pub openvpn_executable_path: String,
    #[serde(default)]
    pub openvpn_ovpn_path: String,
    #[serde(default)]
    pub boundary_cli_path: String,
    #[serde(default)]
    pub caddy_folder: String,
    #[serde(default)]
    pub mcp_enabled: bool,
    #[serde(default = "default_mcp_bind_address")]
    pub mcp_bind_address: String,
    #[serde(default = "default_mcp_port")]
    pub mcp_port: u16,
    #[serde(default = "generate_mcp_auth_token")]
    pub mcp_auth_token: String,
    #[serde(default = "default_mcp_allowed_tool_categories")]
    pub mcp_allowed_tool_categories: Vec<String>,
    #[serde(default = "default_mcp_sensitive_action_policy")]
    pub mcp_sensitive_action_policy: String,
    #[serde(default)]
    pub cc_enabled: bool,
    #[serde(default)]
    pub cc_server_url: String,
    #[serde(default)]
    pub cc_auth_mode: String,
    #[serde(default)]
    pub cc_agent_token: String,
    #[serde(default)]
    pub cc_device_id: String,
    #[serde(default)]
    pub cc_device_name: String,
    #[serde(default)]
    pub first_run_completed: bool,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            repo_root: String::new(),
            openvpn_executable_path: String::new(),
            openvpn_ovpn_path: String::new(),
            boundary_cli_path: String::new(),
            caddy_folder: String::new(),
            mcp_enabled: false,
            mcp_bind_address: default_mcp_bind_address(),
            mcp_port: default_mcp_port(),
            mcp_auth_token: generate_mcp_auth_token(),
            mcp_allowed_tool_categories: default_mcp_allowed_tool_categories(),
            mcp_sensitive_action_policy: default_mcp_sensitive_action_policy(),
            cc_enabled: false,
            cc_server_url: String::new(),
            cc_auth_mode: String::from("token"),
            cc_agent_token: String::new(),
            cc_device_id: String::new(),
            cc_device_name: String::new(),
            first_run_completed: false,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncSettingsPatch {
    #[serde(default)]
    pub repo_root: Option<String>,
    #[serde(default)]
    pub openvpn_executable_path: Option<String>,
    #[serde(default)]
    pub openvpn_ovpn_path: Option<String>,
    #[serde(default)]
    pub boundary_cli_path: Option<String>,
    #[serde(default)]
    pub caddy_folder: Option<String>,
    #[serde(default)]
    pub mcp_enabled: Option<bool>,
    #[serde(default)]
    pub mcp_bind_address: Option<String>,
    #[serde(default)]
    pub mcp_port: Option<u16>,
    #[serde(default)]
    pub mcp_auth_token: Option<String>,
    #[serde(default)]
    pub mcp_allowed_tool_categories: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_sensitive_action_policy: Option<String>,
    #[serde(default)]
    pub cc_enabled: Option<bool>,
    #[serde(default)]
    pub cc_server_url: Option<String>,
    #[serde(default)]
    pub cc_auth_mode: Option<String>,
    #[serde(default)]
    pub cc_agent_token: Option<String>,
    #[serde(default)]
    pub cc_device_id: Option<String>,
    #[serde(default)]
    pub cc_device_name: Option<String>,
    #[serde(default)]
    pub first_run_completed: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPaths {
    pub app_config_path: String,
}

pub fn default_mcp_bind_address() -> String {
    "127.0.0.1".to_string()
}

pub fn default_mcp_port() -> u16 {
    46739
}

pub fn default_mcp_allowed_tool_categories() -> Vec<String> {
    vec!["read-only".to_string(), "runtime-control".to_string()]
}

pub fn default_mcp_sensitive_action_policy() -> String {
    "require-ui-approval".to_string()
}

pub fn generate_mcp_auth_token() -> String {
    Uuid::new_v4().simple().to_string()
}

pub fn decode_sync_settings(raw: &str) -> Result<SyncSettings, String> {
    let mut settings: SyncSettings =
        serde_json::from_str(raw).map_err(|e| format!("invalid sync settings format: {e}"))?;
    if settings.mcp_auth_token.trim().is_empty() {
        settings.mcp_auth_token = generate_mcp_auth_token();
    }
    if settings.mcp_bind_address.trim().is_empty() {
        settings.mcp_bind_address = default_mcp_bind_address();
    }
    if settings.mcp_port == 0 {
        settings.mcp_port = default_mcp_port();
    }
    Ok(settings)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StdioPolicy {
    #[serde(default = "default_stdio_policy_mode")]
    pub sensitive_action_policy: String,
    #[serde(default = "default_stdio_allowed_categories")]
    pub allowed_tool_categories: Vec<String>,
}

pub fn default_stdio_policy_mode() -> String {
    "read-only".to_string()
}

pub fn default_stdio_allowed_categories() -> Vec<String> {
    vec!["read-only".to_string()]
}

impl Default for StdioPolicy {
    fn default() -> Self {
        Self {
            sensitive_action_policy: default_stdio_policy_mode(),
            allowed_tool_categories: default_stdio_allowed_categories(),
        }
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn default_function_statuses(
    openvpn_function_id: &str,
    caddy_function_id: &str,
    boundary_auth_function_id: &str,
) -> HashMap<String, FunctionStatus> {
    let mut statuses = HashMap::new();
    for (id, msg) in [
        (openvpn_function_id, "Ready"),
        (caddy_function_id, "Ready"),
        (boundary_auth_function_id, "Not authenticated"),
    ] {
        statuses.insert(
            id.to_string(),
            FunctionStatus {
                function_id: id.to_string(),
                state: "idle".to_string(),
                message: msg.to_string(),
                boundary_token_source: None,
                session_id: None,
                updated_at_ms: now_ms(),
            },
        );
    }
    statuses
}
