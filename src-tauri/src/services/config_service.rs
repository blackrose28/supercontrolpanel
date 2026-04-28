use std::{fs, path::PathBuf, process::Command};

use uuid::Uuid;

use keyring::{Entry, Error as KeyringError};
use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::models::{
    default_openvpn_reconnect_base_seconds, default_openvpn_reconnect_max_seconds, AppConfig,
    AppRuntimeConfig, BoundaryConfig, OpenVpnConfig,
};

/// OS credential store: Windows Credential Manager, macOS Keychain, Linux Secret Service.
pub const KEYRING_SERVICE: &str = "com.example.supercontrolpanel";
pub const KEYRING_OPENVPN_PASSWORD: &str = "openvpn_password";
pub const KEYRING_OPENVPN_OTP_SECRET: &str = "openvpn_otp_secret";
pub const KEYRING_BOUNDARY_PASSWORD: &str = "boundary_password";
pub const KEYRING_BOUNDARY_OIDC_PASSWORD: &str = "boundary_oidc_password";
pub const KEYRING_BOUNDARY_OIDC_OTP_SECRET: &str = "boundary_oidc_otp_secret";
pub const KEYRING_BOUNDARY_APP_TOKEN: &str = "boundary_app_token";

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

pub fn default_app_config() -> AppConfig {
    AppConfig {
        app: AppRuntimeConfig {
            default_terminal_cols: 120,
            default_terminal_rows: 36,
        },
        openvpn: OpenVpnConfig {
            command: String::new(),
            executable_path: String::new(),
            ovpn_path: String::new(),
            extra_args: String::new(),
            username: String::new(),
            password: String::new(),
            otp_secret: String::new(),
            terminate_open_vpn_when_helper_exits: false,
            auto_reconnect_enabled: false,
            auto_reconnect_base_seconds: default_openvpn_reconnect_base_seconds(),
            auto_reconnect_max_seconds: default_openvpn_reconnect_max_seconds(),
        },
        boundary: BoundaryConfig::default(),
    }
}

pub fn config_file_path(app: &AppHandle, config_file_name: &str) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create config dir: {e}"))?;
    Ok(dir.join(config_file_name))
}

pub fn servers_file_path(app: &AppHandle, servers_file_name: &str) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create config dir: {e}"))?;
    Ok(dir.join(servers_file_name))
}

pub fn sync_settings_path(
    app: &AppHandle,
    sync_settings_file_name: &str,
) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create config dir: {e}"))?;
    Ok(dir.join(sync_settings_file_name))
}

pub fn load_sync_settings_from_disk(
    app: &AppHandle,
    sync_settings_file_name: &str,
) -> Result<SyncSettings, String> {
    let path = sync_settings_path(app, sync_settings_file_name)?;
    if !path.exists() {
        return Ok(SyncSettings::default());
    }
    let raw =
        fs::read_to_string(&path).map_err(|e| format!("failed to read sync settings: {e}"))?;
    let mut settings: SyncSettings =
        serde_json::from_str(&raw).map_err(|e| format!("invalid sync settings format: {e}"))?;
    if settings.mcp_auth_token.trim().is_empty() {
        settings.mcp_auth_token = generate_mcp_auth_token();
    }
    if settings.mcp_bind_address.trim().is_empty() {
        settings.mcp_bind_address = default_mcp_bind_address();
    }
    if settings.mcp_port == 0 {
        settings.mcp_port = default_mcp_port();
    }
    if settings.mcp_allowed_tool_categories.is_empty() {
        settings.mcp_allowed_tool_categories = default_mcp_allowed_tool_categories();
    }
    if settings.mcp_sensitive_action_policy.trim().is_empty() {
        settings.mcp_sensitive_action_policy = default_mcp_sensitive_action_policy();
    }
    Ok(settings)
}

pub fn save_sync_settings_to_disk(
    app: &AppHandle,
    settings: &SyncSettings,
    sync_settings_file_name: &str,
) -> Result<(), String> {
    let path = sync_settings_path(app, sync_settings_file_name)?;
    let encoded = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("failed to encode sync settings: {e}"))?;
    fs::write(path, encoded).map_err(|e| format!("failed to write sync settings: {e}"))
}

pub fn detect_openvpn_executable_path() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            r"C:\Program Files\OpenVPN\bin\openvpn.exe",
            r"C:\Program Files\OpenVPN Connect\bin\openvpn.exe",
            r"C:\Program Files (x86)\OpenVPN\bin\openvpn.exe",
        ];
        for candidate in candidates {
            if PathBuf::from(candidate).exists() {
                return Some(candidate.to_string());
            }
        }

        if let Ok(output) = Command::new("where").arg("openvpn").output() {
            if output.status.success() {
                let value = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = value.lines().map(str::trim).find(|line| !line.is_empty()) {
                    return Some(line.to_string());
                }
            }
        }
        None
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(output) = Command::new("which").arg("openvpn").output() {
            if output.status.success() {
                let value = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = value.lines().map(str::trim).find(|line| !line.is_empty()) {
                    return Some(line.to_string());
                }
            }
        }

        let candidates = [
            "/usr/sbin/openvpn",
            "/usr/local/sbin/openvpn",
            "/opt/homebrew/sbin/openvpn",
        ];
        for candidate in candidates {
            if PathBuf::from(candidate).exists() {
                return Some(candidate.to_string());
            }
        }
        None
    }
}

pub fn keyring_entry(account: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, account).map_err(|e| format!("keyring: {e}"))
}

pub fn keyring_get_optional(account: &str) -> Result<Option<String>, String> {
    let e = keyring_entry(account)?;
    match e.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(err) => Err(format!("keyring read ({account}): {err}")),
    }
}

pub fn keyring_set_or_delete(account: &str, value: &str) -> Result<(), String> {
    let e = keyring_entry(account)?;
    if value.is_empty() {
        match e.delete_credential() {
            Ok(()) => Ok(()),
            Err(KeyringError::NoEntry) => Ok(()),
            Err(err) => Err(format!("keyring delete ({account}): {err}")),
        }
    } else {
        e.set_password(value)
            .map_err(|err| format!("keyring write ({account}): {err}"))
    }
}

pub fn boundary_addr_base(addr: &str) -> String {
    addr.trim().trim_end_matches('/').to_string()
}

pub fn boundary_cli_path(cfg: &AppConfig) -> String {
    if cfg.boundary.cli_path.trim().is_empty() {
        "boundary".to_string()
    } else {
        cfg.boundary.cli_path.trim().to_string()
    }
}

pub fn boundary_app_token() -> Result<Option<String>, String> {
    keyring_get_optional(KEYRING_BOUNDARY_APP_TOKEN)
}

pub fn apply_boundary_env(cmd: &mut Command, cfg: &AppConfig) -> Result<(), String> {
    let addr = cfg.boundary.addr.trim();
    if !addr.is_empty() {
        cmd.env("BOUNDARY_ADDR", addr);
    }
    if let Some(token) = boundary_app_token()? {
        if !token.trim().is_empty() {
            cmd.env("BOUNDARY_TOKEN", token);
        }
    }
    Ok(())
}

pub fn apply_boundary_env_builder(cmd: &mut CommandBuilder, cfg: &AppConfig) -> Result<(), String> {
    let addr = cfg.boundary.addr.trim();
    if !addr.is_empty() {
        cmd.env("BOUNDARY_ADDR", addr);
    }
    if let Some(token) = boundary_app_token()? {
        if !token.trim().is_empty() {
            cmd.env("BOUNDARY_TOKEN", token);
        }
    }
    Ok(())
}

pub fn apply_boundary_token_arg_if_present(cmd: &mut Command) -> Result<(), String> {
    if let Some(token) = boundary_app_token()? {
        if !token.trim().is_empty() {
            cmd.arg("-token");
            cmd.arg("env://BOUNDARY_TOKEN");
        }
    }
    Ok(())
}

pub fn apply_boundary_token_arg_if_present_builder(cmd: &mut CommandBuilder) -> Result<(), String> {
    if let Some(token) = boundary_app_token()? {
        if !token.trim().is_empty() {
            cmd.arg("-token");
            cmd.arg("env://BOUNDARY_TOKEN");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_settings_first_run_completed_defaults_to_false() {
        let settings = SyncSettings::default();
        assert!(!settings.first_run_completed, "first_run_completed should default to false");
    }

    #[test]
    fn sync_settings_round_trips_first_run_completed() {
        let json = r#"{"repoRoot":"/tmp","firstRunCompleted":true}"#;
        let settings: SyncSettings = serde_json::from_str(json).expect("should deserialize");
        assert!(settings.first_run_completed);
        assert_eq!(settings.repo_root, "/tmp");

        let serialized = serde_json::to_string(&settings).expect("should serialize");
        assert!(serialized.contains("firstRunCompleted"));
        assert!(serialized.contains("true"));
    }

    #[test]
    fn sync_settings_deserialize_without_first_run_completed_defaults_false() {
        let json = r#"{"repoRoot":"/tmp", "ccServerUrl":"http://example.com"}"#;
        let settings: SyncSettings = serde_json::from_str(json).expect("should deserialize");
        assert!(!settings.first_run_completed, "missing field should default to false");
        assert_eq!(settings.cc_server_url, "http://example.com");
    }

    #[test]
    fn sync_settings_patch_includes_first_run_completed() {
        let json = r#"{"firstRunCompleted":true,"ccServerUrl":"http://example.com"}"#;
        let patch: SyncSettingsPatch = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(patch.first_run_completed, Some(true));
        assert_eq!(patch.cc_server_url, Some("http://example.com".to_string()));
    }

    #[test]
    fn sync_settings_patch_excludes_first_run_completed_defaults_none() {
        let json = r#"{"ccServerUrl":"http://example.com"}"#;
        let patch: SyncSettingsPatch = serde_json::from_str(json).expect("should deserialize");
        assert!(patch.first_run_completed.is_none());
    }
}
