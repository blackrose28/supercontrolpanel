use std::{fs, path::PathBuf, process::Command};

use keyring::{Entry, Error as KeyringError};
use portable_pty::CommandBuilder;
#[allow(unused_imports)]
pub use scp_core::models::{
    decode_sync_settings, default_app_config, default_mcp_allowed_tool_categories,
    default_mcp_bind_address, default_mcp_port, default_mcp_sensitive_action_policy,
    generate_mcp_auth_token, keyring_server_ssh_password_account, ConfigPaths, SyncSettings,
    SyncSettingsPatch, KEYRING_BOUNDARY_APP_TOKEN, KEYRING_BOUNDARY_OIDC_OTP_SECRET,
    KEYRING_BOUNDARY_OIDC_PASSWORD, KEYRING_BOUNDARY_PASSWORD, KEYRING_CC_TELEMETRY_QUEUE_KEY,
    KEYRING_OPENVPN_OTP_SECRET, KEYRING_OPENVPN_PASSWORD, KEYRING_SERVICE, STATIC_KEYRING_ACCOUNTS,
};
use tauri::{AppHandle, Manager};

use crate::models::AppConfig;

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
    let _settings_guard = crate::settings_backup::gate::enter()?;
    let path = sync_settings_path(app, sync_settings_file_name)?;
    if !path.exists() {
        return Ok(SyncSettings::default());
    }
    let raw =
        fs::read_to_string(&path).map_err(|e| format!("failed to read sync settings: {e}"))?;
    decode_sync_settings(&raw)
}

pub fn save_sync_settings_to_disk(
    app: &AppHandle,
    settings: &SyncSettings,
    sync_settings_file_name: &str,
) -> Result<(), String> {
    let _settings_guard = crate::settings_backup::gate::enter()?;
    crate::settings_backup::gate::changed();
    let path = sync_settings_path(app, sync_settings_file_name)?;
    let encoded = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("failed to encode sync settings: {e}"))?;
    fs::write(path, encoded).map_err(|e| format!("failed to write sync settings: {e}"))
}

pub fn detect_openvpn_executable_path() -> Option<String> {
    detect_openvpn_executable_checked().ok().flatten()
}

/// MCP needs to distinguish an absent executable from locator/filesystem failures.
/// The existing Option-returning desktop helper keeps its fallback contract.
pub fn detect_openvpn_executable_checked() -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    let (locator, candidates): (&str, &[&str]) = (
        "where",
        &[
            r"C:\Program Files\OpenVPN\bin\openvpn.exe",
            r"C:\Program Files\OpenVPN Connect\bin\openvpn.exe",
            r"C:\Program Files (x86)\OpenVPN\bin\openvpn.exe",
        ],
    );
    #[cfg(not(target_os = "windows"))]
    let (locator, candidates): (&str, &[&str]) = (
        "which",
        &[
            "/usr/sbin/openvpn",
            "/usr/local/sbin/openvpn",
            "/opt/homebrew/sbin/openvpn",
        ],
    );
    #[cfg(target_os = "windows")]
    if let Some(path) = common_openvpn_path(candidates)? {
        return Ok(Some(path));
    }

    let located = match Command::new(locator).arg("openvpn").output() {
        Ok(output) => openvpn_locator_result(output.status.code(), &output.stdout),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("OpenVPN executable detection failed".into()),
    };
    if let Ok(Some(path)) = located {
        return Ok(Some(path));
    }
    if let Some(path) = common_openvpn_path(candidates)? {
        return Ok(Some(path));
    }
    located
}

fn common_openvpn_path(candidates: &[&str]) -> Result<Option<String>, String> {
    for candidate in candidates {
        match fs::metadata(candidate) {
            Ok(metadata) if metadata.is_file() => return Ok(Some((*candidate).into())),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("OpenVPN executable detection failed".into()),
        }
    }
    Ok(None)
}

pub fn keyring_entry(account: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, account).map_err(|e| format!("keyring: {e}"))
}

/// Read a secret directly from the OS keyring (bypasses in-memory cache).
/// Used only during cache initialisation at startup.
pub fn keyring_load_from_os(account: &str) -> Result<Option<String>, String> {
    let e = keyring_entry(account)?;
    match e.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(err) => Err(format!("keyring read ({account}): {err}")),
    }
}

/// Read a secret.  Always served from the in-memory cache (populated at
/// startup by `initialize_keyring_cache`), so this never triggers an OS
/// keyring unlock prompt.
pub fn keyring_get_optional(account: &str) -> Result<Option<String>, String> {
    let _settings_guard = crate::settings_backup::gate::enter()?;
    Ok(crate::stores::keyring_cache_get(account))
}

/// Write or delete a secret.  Writes through to both the OS keyring (for
/// persistence across app restarts) and the in-memory cache (so subsequent
/// reads see the new value immediately without hitting the OS keyring).
pub fn keyring_set_or_delete(account: &str, value: &str) -> Result<(), String> {
    let _settings_guard = crate::settings_backup::gate::enter()?;
    crate::settings_backup::gate::changed();
    // 1. Write to OS keyring for persistence.
    let e = keyring_entry(account)?;
    if value.is_empty() {
        match e.delete_credential() {
            Ok(()) => {}
            Err(KeyringError::NoEntry) => {}
            Err(err) => return Err(format!("keyring delete ({account}): {err}")),
        }
    } else {
        e.set_password(value)
            .map_err(|err| format!("keyring write ({account}): {err}"))?;
    }
    // 2. Update in-memory cache.
    crate::stores::keyring_cache_set(account, value);
    Ok(())
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

/// Populate the global keyring cache by bulk-reading every known secret from
/// the OS credential store.  This is called **once** during app startup so
/// that the OS only asks for the keyring unlock a single time.
///
/// `server_ids` is the list of server IDs from `servers.json` (each server
/// may have an SSH password stored under `server_ssh_password:{id}`).
pub fn initialize_keyring_cache(server_ids: &[String]) {
    use std::collections::HashMap;

    let mut entries = HashMap::new();

    // Load static accounts.
    for &account in STATIC_KEYRING_ACCOUNTS {
        match keyring_load_from_os(account) {
            Ok(Some(value)) if !value.is_empty() => {
                entries.insert(account.to_string(), value);
            }
            _ => {} // missing or error — skip, the cache just won't have it
        }
    }

    // Load dynamic per-server SSH passwords.
    for id in server_ids {
        let account = keyring_server_ssh_password_account(id);
        match keyring_load_from_os(&account) {
            Ok(Some(value)) if !value.is_empty() => {
                entries.insert(account, value);
            }
            _ => {}
        }
    }

    crate::stores::seed_keyring_cache(entries);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_settings_first_run_completed_defaults_to_false() {
        let settings = SyncSettings::default();
        assert!(
            !settings.first_run_completed,
            "first_run_completed should default to false"
        );
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
        assert!(
            !settings.first_run_completed,
            "missing field should default to false"
        );
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

#[cfg(test)]
mod mcp_policy_settings_tests {
    use super::*;
    #[test]
    fn explicit_empty_policy_and_categories_are_not_replaced_by_permissions() {
        let settings = decode_sync_settings(
            r#"{"mcpSensitiveActionPolicy":"","mcpAllowedToolCategories":[]}"#,
        )
        .unwrap();
        assert!(settings.mcp_allowed_tool_categories.is_empty());
        assert!(settings.mcp_sensitive_action_policy.is_empty());
        let missing: SyncSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.mcp_sensitive_action_policy, "require-ui-approval");
        assert_eq!(
            missing.mcp_allowed_tool_categories,
            default_mcp_allowed_tool_categories()
        );
    }
}

fn openvpn_locator_result(status: Option<i32>, stdout: &[u8]) -> Result<Option<String>, String> {
    if status == Some(1) {
        return Ok(None);
    }
    if status != Some(0) {
        return Err("OpenVPN executable detection failed".into());
    }
    std::str::from_utf8(stdout)
        .ok()
        .and_then(|value| value.lines().map(str::trim).find(|line| !line.is_empty()))
        .map(|line| Some(line.to_owned()))
        .ok_or_else(|| "OpenVPN executable detection failed".into())
}

#[cfg(test)]
mod openvpn_detection_tests {
    use super::*;
    #[test]
    fn locator_distinguishes_absence_failures_and_platform_output() {
        assert_eq!(
            openvpn_locator_result(Some(0), b"/usr/sbin/openvpn\n").unwrap(),
            Some("/usr/sbin/openvpn".into())
        );
        assert_eq!(
            openvpn_locator_result(Some(0), b"C:\\OpenVPN\\openvpn.exe\r\nD:\\other.exe\r\n")
                .unwrap(),
            Some(r"C:\OpenVPN\openvpn.exe".into())
        );
        assert_eq!(openvpn_locator_result(Some(1), b"").unwrap(), None);
        for (status, output) in [
            (Some(2), b"secret".as_slice()),
            (None, b""),
            (Some(0), b""),
            (Some(0), b"\xff"),
        ] {
            assert!(openvpn_locator_result(status, output).is_err());
        }
    }
}
