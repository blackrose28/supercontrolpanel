//! Device-local settings schema shared by desktop and future headless hosts.
//!
//! These models contain MCP/CC tokens, device identity and local paths. They are
//! secret-bearing persistence/runtime models, not public CLI/MCP output DTOs.
//! Schema defaults do not load files, normalize values or activate services.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
