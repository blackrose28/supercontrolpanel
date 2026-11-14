//! Config, server inventory, credential, sync-push, RDP, and backup mutations for ProfileRuntime.
use crate::backup::{
    compile_restore, crypto, describe, io as backup_io, is_valid_server_id, snapshot_with_version,
    transaction, validate_export_target, PendingRestore, Preview, RestoreOptions,
};
use crate::error::CoreError;
use crate::models::{
    keyring_server_ssh_password_account, AppConfig, AppConfigForDisk, ScpTransferState, Server,
    ServerOs, ServersFile, SyncSettings, SyncSettingsPatch, KEYRING_BOUNDARY_OIDC_OTP_SECRET,
    KEYRING_BOUNDARY_OIDC_PASSWORD, KEYRING_BOUNDARY_PASSWORD, KEYRING_OPENVPN_OTP_SECRET,
    KEYRING_OPENVPN_PASSWORD,
};
use crate::runtime::ProfileRuntime;
use crate::secrets::KeyringBackend;
use crate::sftp_ops::close_browsers_for_server_impl;
use crate::ssh_ops::{server_ssh_function_id, server_tunnel_function_id};
use crate::transfer_store::list_scp_transfer_entries;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use zeroize::Zeroizing;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const SERVER_ALLOWED_KEYS: &[&str] = &[
    "id",
    "name",
    "host",
    "os",
    "connectionMethod",
    "sshPort",
    "sshUser",
    "boundaryTargetId",
    "boundaryListenPort",
    "services",
];

const SERVER_SERVICE_ALLOWED_KEYS: &[&str] = &["name", "port"];

const CONFIG_TOP_KEYS: &[&str] = &["app", "openvpn", "boundary"];
const CONFIG_APP_KEYS: &[&str] = &["defaultTerminalCols", "defaultTerminalRows"];
const CONFIG_OPENVPN_KEYS: &[&str] = &[
    "command",
    "executablePath",
    "ovpnPath",
    "extraArgs",
    "username",
    "password",
    "otpSecret",
    "terminateOpenVpnWhenHelperExits",
    "terminateOpenvpnWhenHelperExits",
    "autoReconnectEnabled",
    "autoReconnectBaseSeconds",
    "autoReconnectMaxSeconds",
];
const CONFIG_BOUNDARY_KEYS: &[&str] = &[
    "cliPath",
    "addr",
    "authMethodId",
    "loginName",
    "password",
    "authKind",
    "oidcLoginMode",
    "oidcUsername",
    "oidcPassword",
    "oidcOtpSecret",
    "connectSshOptions",
];

const SYNC_ALLOWED_KEYS: &[&str] = &[
    "repoRoot",
    "openvpnExecutablePath",
    "openvpnOvpnPath",
    "boundaryCliPath",
    "caddyFolder",
    "mcpEnabled",
    "mcpBindAddress",
    "mcpPort",
    "mcpAuthToken",
    "mcpAllowedToolCategories",
    "mcpSensitiveActionPolicy",
    "ccEnabled",
    "ccServerUrl",
    "ccAuthMode",
    "ccAgentToken",
    "ccDeviceId",
    "ccDeviceName",
    "firstRunCompleted",
];

pub fn validate_server_id(id: &str) -> Result<(), CoreError> {
    if !is_valid_server_id(id) {
        return Err(CoreError::invalid_arguments(
            "invalid_server_id",
            "Server ID must be 1..=256 ASCII alphanumeric or '-_.:' characters",
        ));
    }
    Ok(())
}

pub fn validate_rdp_target(
    os: &str,
    interactive: bool,
    windows_server: bool,
    host: &str,
) -> Result<(), CoreError> {
    if !matches!(os, "windows" | "macos" | "linux") || !interactive || !windows_server {
        return Err(CoreError::capability_unavailable(
            "rdp_unavailable",
            "RDP requires an interactive session on Windows, macOS, or Linux targeting a Windows server",
            "check_capabilities",
        ));
    }
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c))
    {
        return Err(CoreError::invalid_arguments(
            "invalid_rdp_host",
            "Server host is required and must be a safe hostname or IP address for RDP",
        ));
    }
    Ok(())
}

pub fn resolve_config_secret_account(key: &str) -> Result<&'static str, CoreError> {
    match key.trim() {
        "openvpn_password" | "openvpn-password" => Ok(KEYRING_OPENVPN_PASSWORD),
        "openvpn_otp_secret" | "openvpn-otp-secret" => Ok(KEYRING_OPENVPN_OTP_SECRET),
        "boundary_password" | "boundary-password" => Ok(KEYRING_BOUNDARY_PASSWORD),
        "boundary_oidc_password" | "boundary-oidc-password" => Ok(KEYRING_BOUNDARY_OIDC_PASSWORD),
        "boundary_oidc_otp_secret" | "boundary-oidc-otp-secret" => {
            Ok(KEYRING_BOUNDARY_OIDC_OTP_SECRET)
        }
        other => Err(CoreError::invalid_arguments(
            "unknown_secret_key",
            format!(
                "Unsupported config secret key '{other}'; expected one of: openvpn-password, openvpn-otp-secret, boundary-password, boundary-oidc-password, boundary-oidc-otp-secret"
            ),
        )),
    }
}

fn sha256_revision(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

fn check_expected_revision(current: &str, expected: Option<&str>) -> Result<(), CoreError> {
    if let Some(exp) = expected {
        if exp != current {
            return Err(CoreError::conflict(
                "revision_mismatch",
                format!("Target state changed since preview (expected {exp}, actual {current})"),
                false,
                "preview_again",
            ));
        }
    }
    Ok(())
}

fn servers_revision(servers: &[Server]) -> Result<String, CoreError> {
    let bytes = serde_json::to_vec(&ServersFile {
        servers: servers.to_vec(),
    })
    .map_err(|e| {
        CoreError::operation_failed(
            "servers_encode_failed",
            e.to_string(),
            false,
            "fix_configuration",
        )
    })?;
    Ok(sha256_revision(&bytes))
}

fn config_revision(cfg: &AppConfig, sync: &SyncSettings) -> Result<String, CoreError> {
    let payload = json!({
        "config": AppConfigForDisk::from(cfg),
        "openvpnExecutablePath": sync.openvpn_executable_path,
        "openvpnOvpnPath": sync.openvpn_ovpn_path,
        "boundaryCliPath": sync.boundary_cli_path,
    });
    let bytes = Zeroizing::new(serde_json::to_vec(&payload).map_err(|e| {
        CoreError::operation_failed(
            "config_encode_failed",
            e.to_string(),
            false,
            "fix_configuration",
        )
    })?);
    Ok(sha256_revision(&bytes))
}

fn sync_revision(sync: &SyncSettings) -> Result<String, CoreError> {
    let bytes = Zeroizing::new(serde_json::to_vec(sync).map_err(|e| {
        CoreError::operation_failed(
            "sync_settings_encode_failed",
            e.to_string(),
            false,
            "fix_configuration",
        )
    })?);
    Ok(sha256_revision(&bytes))
}

fn reject_unknown_keys(
    obj: &Map<String, Value>,
    allowed: &[&str],
    context: &str,
) -> Result<(), CoreError> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(CoreError::invalid_arguments(
                "unknown_field",
                format!("Unknown {context} field '{key}'"),
            ));
        }
    }
    Ok(())
}

fn check_string_bounds(value: &Value, context: &str) -> Result<(), CoreError> {
    match value {
        Value::String(text) => {
            if text.contains('\0') || text.len() > 4096 {
                return Err(CoreError::invalid_arguments(
                    "invalid_string_field",
                    format!("Invalid {context} string length or NUL byte"),
                ));
            }
        }
        Value::Array(items) => {
            for item in items {
                check_string_bounds(item, context)?;
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                check_string_bounds(item, context)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn as_object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>, CoreError> {
    value.as_object().ok_or_else(|| {
        CoreError::invalid_arguments(
            "invalid_payload",
            format!("{context} payload must be a JSON object"),
        )
    })
}

fn parse_and_validate_server(value: &Value) -> Result<Server, CoreError> {
    check_string_bounds(value, "server")?;
    let obj = as_object(value, "server")?;
    reject_unknown_keys(obj, SERVER_ALLOWED_KEYS, "server")?;
    if let Some(services) = obj.get("services").and_then(Value::as_array) {
        if services.len() > 128 {
            return Err(CoreError::invalid_arguments(
                "invalid_server",
                "Server services cannot exceed 128 entries",
            ));
        }
        for svc in services {
            let svc_obj = as_object(svc, "server service")?;
            reject_unknown_keys(svc_obj, SERVER_SERVICE_ALLOWED_KEYS, "server service")?;
        }
    }
    let server: Server = serde_json::from_value(value.clone()).map_err(|e| {
        CoreError::invalid_arguments("invalid_server", format!("Invalid server payload: {e}"))
    })?;
    validate_server_id(&server.id)?;
    if server.name.trim().is_empty()
        || server.host.trim().is_empty()
        || server.ssh_user.trim().is_empty()
        || server.ssh_port == 0
    {
        return Err(CoreError::invalid_arguments(
            "invalid_server",
            "Server id, name, host, sshUser, and positive sshPort are required",
        ));
    }
    if server
        .services
        .iter()
        .any(|svc| svc.name.trim().is_empty() || svc.port == 0)
    {
        return Err(CoreError::invalid_arguments(
            "invalid_server",
            "Each server service requires a non-empty name and positive port",
        ));
    }
    Ok(server)
}

#[derive(Default)]
struct ExtractedConfigSecrets {
    openvpn_password: Option<Zeroizing<String>>,
    openvpn_otp_secret: Option<Zeroizing<String>>,
    boundary_password: Option<Zeroizing<String>>,
    boundary_oidc_password: Option<Zeroizing<String>>,
    boundary_oidc_otp_secret: Option<Zeroizing<String>>,
}

fn validate_config_object(value: &mut Value) -> Result<(), CoreError> {
    let _ = validate_config_object_impl(value, false)?;
    Ok(())
}

fn validate_mcp_config_object(value: &mut Value) -> Result<ExtractedConfigSecrets, CoreError> {
    validate_config_object_impl(value, true)
}

fn validate_config_object_impl(
    value: &mut Value,
    allow_secrets: bool,
) -> Result<ExtractedConfigSecrets, CoreError> {
    check_string_bounds(value, "config")?;
    let obj = value.as_object_mut().ok_or_else(|| {
        CoreError::invalid_arguments("invalid_config", "Config payload must be a JSON object")
    })?;
    reject_unknown_keys(obj, CONFIG_TOP_KEYS, "config")?;
    if let Some(app) = obj.get("app") {
        let app_obj = as_object(app, "config.app")?;
        reject_unknown_keys(app_obj, CONFIG_APP_KEYS, "config.app")?;
    }
    let mut secrets = ExtractedConfigSecrets::default();
    if let Some(vpn) = obj.get_mut("openvpn") {
        let vpn_obj = vpn.as_object_mut().ok_or_else(|| {
            CoreError::invalid_arguments("invalid_config", "config.openvpn must be an object")
        })?;
        reject_unknown_keys(vpn_obj, CONFIG_OPENVPN_KEYS, "config.openvpn")?;
        if allow_secrets {
            secrets.openvpn_password = extract_optional_secret_field(vpn_obj, "password")?;
            secrets.openvpn_otp_secret = extract_optional_secret_field(vpn_obj, "otpSecret")?;
        } else {
            reject_inline_secret_fields(vpn_obj, &["password", "otpSecret"])?;
        }
    }
    if let Some(boundary) = obj.get_mut("boundary") {
        let b_obj = boundary.as_object_mut().ok_or_else(|| {
            CoreError::invalid_arguments("invalid_config", "config.boundary must be an object")
        })?;
        reject_unknown_keys(b_obj, CONFIG_BOUNDARY_KEYS, "config.boundary")?;
        if allow_secrets {
            secrets.boundary_password = extract_optional_secret_field(b_obj, "password")?;
            secrets.boundary_oidc_password = extract_optional_secret_field(b_obj, "oidcPassword")?;
            secrets.boundary_oidc_otp_secret =
                extract_optional_secret_field(b_obj, "oidcOtpSecret")?;
        } else {
            reject_inline_secret_fields(b_obj, &["password", "oidcPassword", "oidcOtpSecret"])?;
        }
    }
    Ok(secrets)
}

fn extract_optional_secret_field(
    obj: &mut Map<String, Value>,
    key: &str,
) -> Result<Option<Zeroizing<String>>, CoreError> {
    let Some(val) = obj.remove(key) else {
        return Ok(None);
    };
    let s = val.as_str().ok_or_else(|| {
        CoreError::invalid_arguments(
            "invalid_config",
            format!("Config secret field '{key}' must be a string"),
        )
    })?;
    if s.is_empty() {
        Ok(None)
    } else {
        Ok(Some(Zeroizing::new(s.to_string())))
    }
}

fn apply_optional_local_path_updates(
    sanitized: &Value,
    current_sync: &SyncSettings,
    next_sync: &mut SyncSettings,
    next: &mut AppConfig,
) {
    let vpn_obj = sanitized.get("openvpn").and_then(Value::as_object);
    let boundary_obj = sanitized.get("boundary").and_then(Value::as_object);
    let updates: [(bool, &str, &mut String, &mut String); 3] = [
        (
            vpn_obj.is_some_and(|m| m.contains_key("executablePath")),
            current_sync.openvpn_executable_path.as_str(),
            &mut next_sync.openvpn_executable_path,
            &mut next.openvpn.executable_path,
        ),
        (
            vpn_obj.is_some_and(|m| m.contains_key("ovpnPath")),
            current_sync.openvpn_ovpn_path.as_str(),
            &mut next_sync.openvpn_ovpn_path,
            &mut next.openvpn.ovpn_path,
        ),
        (
            boundary_obj.is_some_and(|m| m.contains_key("cliPath")),
            current_sync.boundary_cli_path.as_str(),
            &mut next_sync.boundary_cli_path,
            &mut next.boundary.cli_path,
        ),
    ];
    for (provided, current_val, sync_slot, cfg_slot) in updates {
        if provided {
            *sync_slot = cfg_slot.trim().to_string();
        } else {
            *sync_slot = current_val.to_string();
            *cfg_slot = current_val.to_string();
        }
    }
}

fn reject_inline_secret_fields(
    obj: &mut Map<String, Value>,
    secret_keys: &[&str],
) -> Result<(), CoreError> {
    for key in secret_keys {
        if let Some(val) = obj.remove(*key) {
            match val.as_str() {
                Some("") => {}
                _ => {
                    return Err(CoreError::invalid_arguments(
                        "inline_secrets_forbidden",
                        format!(
                            "Inline secret field '{key}' is forbidden; use 'scpctl config secret set' instead"
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_app_config_values(cfg: &AppConfig) -> Result<(), CoreError> {
    if cfg.app.default_terminal_cols == 0 || cfg.app.default_terminal_rows == 0 {
        return Err(CoreError::invalid_arguments(
            "invalid_config",
            "Terminal cols and rows must be between 1 and 65535",
        ));
    }
    if cfg.openvpn.auto_reconnect_base_seconds == 0
        || cfg.openvpn.auto_reconnect_base_seconds > 86_400
        || cfg.openvpn.auto_reconnect_max_seconds == 0
        || cfg.openvpn.auto_reconnect_max_seconds > 86_400
    {
        return Err(CoreError::invalid_arguments(
            "invalid_config",
            "OpenVPN reconnect delays must be between 1 and 86400 seconds",
        ));
    }
    Ok(())
}

fn merge_json_object(target: &mut Value, patch: &Value) {
    if let (Some(t), Some(p)) = (target.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            match t.get_mut(k) {
                Some(existing) if existing.is_object() && v.is_object() => {
                    merge_json_object(existing, v);
                }
                _ => {
                    t.insert(k.clone(), v.clone());
                }
            }
        }
    }
}

fn redacted_config_summary(cfg: &AppConfig, sync: &SyncSettings) -> Value {
    let mut disk = serde_json::to_value(AppConfigForDisk::from(cfg)).unwrap_or(Value::Null);
    if let Some(vpn) = disk.get_mut("openvpn").and_then(Value::as_object_mut) {
        vpn.insert(
            "executablePath".into(),
            Value::String(sync.openvpn_executable_path.clone()),
        );
        vpn.insert(
            "ovpnPath".into(),
            Value::String(sync.openvpn_ovpn_path.clone()),
        );
    }
    if let Some(boundary) = disk.get_mut("boundary").and_then(Value::as_object_mut) {
        boundary.insert(
            "cliPath".into(),
            Value::String(sync.boundary_cli_path.clone()),
        );
    }
    disk
}

fn redacted_sync_summary(sync: &SyncSettings) -> Value {
    json!({
        "repoRoot": sync.repo_root,
        "openvpnExecutablePath": sync.openvpn_executable_path,
        "openvpnOvpnPath": sync.openvpn_ovpn_path,
        "boundaryCliPath": sync.boundary_cli_path,
        "caddyFolder": sync.caddy_folder,
        "mcpEnabled": sync.mcp_enabled,
        "mcpBindAddress": sync.mcp_bind_address,
        "mcpPort": sync.mcp_port,
        "mcpAllowedToolCategories": sync.mcp_allowed_tool_categories,
        "mcpSensitiveActionPolicy": sync.mcp_sensitive_action_policy,
        "ccEnabled": sync.cc_enabled,
        "ccServerUrl": sync.cc_server_url,
        "ccAuthMode": sync.cc_auth_mode,
        "ccDeviceName": sync.cc_device_name,
        "firstRunCompleted": sync.first_run_completed,
    })
}

fn atomic_write_json<T: serde::Serialize>(
    path: &Path,
    value: &T,
    reason: &'static str,
) -> Result<(), CoreError> {
    let encoded = Zeroizing::new(serde_json::to_vec_pretty(value).map_err(|e| {
        CoreError::operation_failed(reason, e.to_string(), false, "fix_configuration")
    })?);
    backup_io::atomic_write(path, &encoded, true)
        .map_err(|e| CoreError::operation_failed(reason, e, false, "fix_configuration"))
}

fn write_servers_atomic(path: &Path, servers: &[Server]) -> Result<(), CoreError> {
    atomic_write_json(
        path,
        &ServersFile {
            servers: servers.to_vec(),
        },
        "servers_write_failed",
    )
}

fn write_app_config_atomic(path: &Path, cfg: &AppConfig) -> Result<(), CoreError> {
    atomic_write_json(path, &AppConfigForDisk::from(cfg), "config_write_failed")
}

fn write_sync_settings_atomic(path: &Path, sync: &SyncSettings) -> Result<(), CoreError> {
    atomic_write_json(path, sync, "sync_settings_write_failed")
}

fn reject_forbidden_sync_fields(patch: &SyncSettingsPatch) -> Result<(), CoreError> {
    if patch.mcp_enabled == Some(true) || patch.cc_enabled == Some(true) {
        return Err(CoreError::invalid_arguments(
            "desktop_service_activation_forbidden",
            "Standalone CLI cannot enable desktop MCP HTTP or Control Center services",
        ));
    }
    if [
        patch.mcp_auth_token.as_deref(),
        patch.cc_agent_token.as_deref(),
        patch.cc_device_id.as_deref(),
        patch.cc_device_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|v| !v.is_empty())
    {
        return Err(CoreError::invalid_arguments(
            "desktop_service_activation_forbidden",
            "Standalone CLI cannot write MCP auth tokens or Control Center enrollment credentials",
        ));
    }
    Ok(())
}

fn apply_sync_patch(merged: &mut SyncSettings, patch: SyncSettingsPatch) -> Result<(), CoreError> {
    if let Some(repo_root) = patch.repo_root {
        merged.repo_root = repo_root;
    }
    if let Some(path) = patch.openvpn_executable_path {
        merged.openvpn_executable_path = path;
    }
    if let Some(path) = patch.openvpn_ovpn_path {
        merged.openvpn_ovpn_path = path;
    }
    if let Some(path) = patch.boundary_cli_path {
        merged.boundary_cli_path = path;
    }
    if let Some(folder) = patch.caddy_folder {
        merged.caddy_folder = folder;
    }
    if let Some(enabled) = patch.mcp_enabled {
        merged.mcp_enabled = enabled;
    }
    if let Some(bind) = patch.mcp_bind_address {
        if bind.trim().is_empty() {
            return Err(CoreError::invalid_arguments(
                "invalid_sync_settings",
                "mcpBindAddress cannot be empty",
            ));
        }
        merged.mcp_bind_address = bind;
    }
    if let Some(port) = patch.mcp_port {
        if port == 0 {
            return Err(CoreError::invalid_arguments(
                "invalid_sync_settings",
                "mcpPort must be between 1 and 65535",
            ));
        }
        merged.mcp_port = port;
    }
    if let Some(cats) = patch.mcp_allowed_tool_categories {
        if cats.is_empty() {
            return Err(CoreError::invalid_arguments(
                "invalid_sync_settings",
                "mcpAllowedToolCategories cannot be empty",
            ));
        }
        merged.mcp_allowed_tool_categories = cats;
    }
    if let Some(policy) = patch.mcp_sensitive_action_policy {
        if policy.trim().is_empty() {
            return Err(CoreError::invalid_arguments(
                "invalid_sync_settings",
                "mcpSensitiveActionPolicy cannot be empty",
            ));
        }
        merged.mcp_sensitive_action_policy = policy;
    }
    if let Some(enabled) = patch.cc_enabled {
        merged.cc_enabled = enabled;
    }
    if let Some(url) = patch.cc_server_url {
        merged.cc_server_url = url;
    }
    if let Some(mode) = patch.cc_auth_mode {
        if mode.trim().is_empty() {
            return Err(CoreError::invalid_arguments(
                "invalid_sync_settings",
                "ccAuthMode cannot be empty",
            ));
        }
        merged.cc_auth_mode = mode;
    }
    if let Some(completed) = patch.first_run_completed {
        merged.first_run_completed = completed;
    }
    Ok(())
}

impl<B: KeyringBackend> ProfileRuntime<B> {
    pub fn require_quiescent_for_import(&self) -> Result<(), CoreError> {
        if self.supervisor().session_count() > 0 || self.vpn_lease_arc().lock().is_some() {
            return Err(CoreError::conflict(
                "active_sessions",
                "Stop all active runtime sessions before importing settings",
                true,
                "stop_sessions",
            ));
        }
        if !self.remote_browsers().is_empty() {
            return Err(CoreError::conflict(
                "active_browsers",
                "Close open remote browser sessions before importing settings",
                true,
                "stop_sessions",
            ));
        }
        let pending_transfers = list_scp_transfer_entries(self.transfer_store())
            .map(|items| {
                items
                    .iter()
                    .any(|item| matches!(item.state, ScpTransferState::Pending))
            })
            .unwrap_or(false);
        if pending_transfers {
            return Err(CoreError::conflict(
                "active_transfers",
                "Wait for pending SCP transfers before importing settings",
                true,
                "wait_or_stop_transfers",
            ));
        }
        let active_fn = self
            .runtime_state_arc()
            .lock()
            .function_statuses
            .values()
            .any(|s| {
                matches!(
                    s.state.as_str(),
                    "starting"
                        | "running"
                        | "reconnecting"
                        | "connecting"
                        | "authenticating"
                        | "stopping"
                )
            });
        if active_fn {
            return Err(CoreError::conflict(
                "active_functions",
                "Stop running functions before importing settings",
                true,
                "stop_sessions",
            ));
        }
        Ok(())
    }

    pub fn require_server_quiescent(&self, server_id: &str) -> Result<(), CoreError> {
        let tunnel_id = server_tunnel_function_id(server_id);
        let ssh_id = server_ssh_function_id(server_id);
        let active = self
            .supervisor()
            .active_session_for_function(&tunnel_id)
            .is_some()
            || self
                .supervisor()
                .active_session_for_function(&ssh_id)
                .is_some()
            || self
                .runtime_state_arc()
                .lock()
                .function_statuses
                .iter()
                .any(|(fid, st)| {
                    (fid == &tunnel_id || fid == &ssh_id)
                        && matches!(
                            st.state.as_str(),
                            "starting" | "running" | "reconnecting" | "connecting" | "stopping"
                        )
                });
        if active {
            return Err(CoreError::conflict(
                "server_session_active",
                format!("Stop active tunnel or SSH session for '{server_id}' before modifying it"),
                true,
                "stop_sessions",
            ));
        }
        Ok(())
    }

    pub fn upsert_server(
        &self,
        server_json: &Value,
        preview: bool,
        expected_revision: Option<&str>,
    ) -> Result<Value, CoreError> {
        let server = parse_and_validate_server(server_json)?;
        let _guard = self.gate().enter()?;
        let current = self.get_servers_snapshot();
        let current_rev = servers_revision(&current)?;
        check_expected_revision(&current_rev, expected_revision)?;

        let existing_idx = current.iter().position(|s| s.id == server.id);
        let before = existing_idx.map(|idx| json!(current[idx]));
        let action = if existing_idx.is_some() {
            "update"
        } else {
            "create"
        };
        let mut next = current.clone();
        match existing_idx {
            Some(idx) => next[idx] = server.clone(),
            None => next.push(server.clone()),
        }
        next.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.name.cmp(&b.name)));
        let next_rev = servers_revision(&next)?;

        if preview {
            return Ok(json!({
                "preview": true,
                "action": action,
                "serverId": server.id,
                "revision": current_rev,
                "nextRevision": next_rev,
                "before": before,
                "after": server,
                "changed": current_rev != next_rev,
            }));
        }

        self.require_server_quiescent(&server.id)?;
        close_browsers_for_server_impl(self, &server.id, "server_updated");
        write_servers_atomic(&self.profile_paths().servers_path, &next)?;
        self.replace_servers_snapshot(next);
        self.gate().changed();
        Ok(json!({
            "preview": false,
            "action": action,
            "serverId": server.id,
            "revision": next_rev,
            "previousRevision": current_rev,
            "server": server,
            "changed": current_rev != next_rev,
        }))
    }

    pub fn delete_server(
        &self,
        server_id: &str,
        preview: bool,
        expected_revision: Option<&str>,
    ) -> Result<Value, CoreError> {
        validate_server_id(server_id)?;
        let _guard = self.gate().enter()?;
        let current = self.get_servers_snapshot();
        let Some(existing) = current.iter().find(|s| s.id == server_id).cloned() else {
            return Err(CoreError::not_found(
                "server_not_found",
                format!("Unknown serverId: {server_id}"),
                "inspect_servers",
            ));
        };
        let current_rev = servers_revision(&current)?;
        check_expected_revision(&current_rev, expected_revision)?;

        let mut next = current;
        next.retain(|s| s.id != server_id);
        let next_rev = servers_revision(&next)?;

        if preview {
            let has_pw = self
                .secrets_arc()
                .lock()
                .get(&keyring_server_ssh_password_account(server_id))?
                .is_some();
            return Ok(json!({
                "preview": true,
                "action": "delete",
                "serverId": server_id,
                "revision": current_rev,
                "nextRevision": next_rev,
                "before": existing,
                "after": Value::Null,
                "deletesSshPassword": has_pw,
                "changed": true,
            }));
        }

        self.require_server_quiescent(server_id)?;
        close_browsers_for_server_impl(self, server_id, "server_deleted");
        self.write_secret(&keyring_server_ssh_password_account(server_id), None)?;
        write_servers_atomic(&self.profile_paths().servers_path, &next)?;
        self.replace_servers_snapshot(next);
        self.gate().changed();
        Ok(json!({
            "preview": false,
            "action": "delete",
            "serverId": server_id,
            "revision": next_rev,
            "previousRevision": current_rev,
            "deleted": true,
        }))
    }

    pub fn set_server_ssh_password(
        &self,
        server_id: &str,
        password: &str,
    ) -> Result<Value, CoreError> {
        validate_server_id(server_id)?;
        if password.is_empty() || password.contains('\0') || password.len() > 65536 {
            return Err(CoreError::invalid_arguments(
                "invalid_secret",
                "Secret must be 1..=65536 bytes without NUL bytes",
            ));
        }
        let _guard = self.gate().enter()?;
        let _ = self.get_server(server_id)?;
        self.write_secret(
            &keyring_server_ssh_password_account(server_id),
            Some(password),
        )?;
        self.gate().changed();
        Ok(json!({
            "serverId": server_id,
            "configured": true,
        }))
    }

    pub fn clear_server_ssh_password(&self, server_id: &str) -> Result<Value, CoreError> {
        validate_server_id(server_id)?;
        let _guard = self.gate().enter()?;
        let _ = self.get_server(server_id)?;
        self.write_secret(&keyring_server_ssh_password_account(server_id), None)?;
        self.gate().changed();
        Ok(json!({
            "serverId": server_id,
            "configured": false,
        }))
    }

    pub fn set_config_secret(&self, key: &str, value: &str) -> Result<Value, CoreError> {
        let account = resolve_config_secret_account(key)?;
        if value.is_empty() || value.contains('\0') || value.len() > 65536 {
            return Err(CoreError::invalid_arguments(
                "invalid_secret",
                "Secret must be 1..=65536 bytes without NUL bytes",
            ));
        }
        let _guard = self.gate().enter()?;
        self.write_secret(account, Some(value))?;
        self.gate().changed();
        Ok(json!({
            "account": account,
            "configured": true,
        }))
    }

    pub fn clear_config_secret(&self, key: &str) -> Result<Value, CoreError> {
        let account = resolve_config_secret_account(key)?;
        let _guard = self.gate().enter()?;
        self.write_secret(account, None)?;
        self.gate().changed();
        Ok(json!({
            "account": account,
            "configured": false,
        }))
    }

    pub fn save_app_config(
        &self,
        config_json: &Value,
        preview: bool,
        expected_revision: Option<&str>,
    ) -> Result<Value, CoreError> {
        let mut sanitized = config_json.clone();
        validate_config_object(&mut sanitized)?;
        if !sanitized.get("app").is_some_and(Value::is_object) {
            return Err(CoreError::invalid_arguments(
                "invalid_config",
                "Full config save requires 'app' object with defaultTerminalCols and defaultTerminalRows",
            ));
        }
        let mut next: AppConfig = serde_json::from_value(sanitized).map_err(|e| {
            CoreError::invalid_arguments("invalid_config", format!("Invalid config JSON: {e}"))
        })?;
        validate_app_config_values(&next)?;

        let _guard = self.gate().enter()?;
        let current = self.get_app_config()?;
        let current_sync = self.get_sync_settings()?;
        let current_rev = config_revision(&current, &current_sync)?;
        check_expected_revision(&current_rev, expected_revision)?;

        next.openvpn.password = current.openvpn.password.clone();
        next.openvpn.otp_secret = current.openvpn.otp_secret.clone();
        next.boundary.password = current.boundary.password.clone();
        next.boundary.oidc_password = current.boundary.oidc_password.clone();
        next.boundary.oidc_otp_secret = current.boundary.oidc_otp_secret.clone();

        let mut next_sync = current_sync.clone();
        next_sync.openvpn_executable_path = next.openvpn.executable_path.trim().to_string();
        next_sync.openvpn_ovpn_path = next.openvpn.ovpn_path.trim().to_string();
        next_sync.boundary_cli_path = next.boundary.cli_path.trim().to_string();
        let next_rev = config_revision(&next, &next_sync)?;

        let before_view = redacted_config_summary(&current, &current_sync);
        let after_view = redacted_config_summary(&next, &next_sync);
        if preview {
            return Ok(json!({
                "preview": true,
                "action": "save",
                "revision": current_rev,
                "nextRevision": next_rev,
                "before": before_view,
                "after": after_view,
                "changed": current_rev != next_rev,
            }));
        }

        write_sync_settings_atomic(&self.profile_paths().sync_local_path, &next_sync)?;
        write_app_config_atomic(&self.profile_paths().config_path, &next)?;
        self.replace_app_config_and_sync(next, next_sync);
        Ok(json!({
            "preview": false,
            "action": "save",
            "revision": next_rev,
            "previousRevision": current_rev,
            "appConfig": after_view,
            "changed": current_rev != next_rev,
        }))
    }

    pub fn patch_app_config(
        &self,
        patch_json: &Value,
        preview: bool,
        expected_revision: Option<&str>,
    ) -> Result<Value, CoreError> {
        let mut sanitized = patch_json.clone();
        validate_config_object(&mut sanitized)?;

        let _guard = self.gate().enter()?;
        let current = self.get_app_config()?;
        let current_sync = self.get_sync_settings()?;
        let current_rev = config_revision(&current, &current_sync)?;
        check_expected_revision(&current_rev, expected_revision)?;

        let mut merged = serde_json::to_value(AppConfigForDisk::from(&current)).map_err(|e| {
            CoreError::operation_failed(
                "config_encode_failed",
                e.to_string(),
                false,
                "fix_configuration",
            )
        })?;
        // Include current local paths before merging so omitted path fields remain intact.
        merged["openvpn"]["executablePath"] = json!(current_sync.openvpn_executable_path);
        merged["openvpn"]["ovpnPath"] = json!(current_sync.openvpn_ovpn_path);
        merged["boundary"]["cliPath"] = json!(current_sync.boundary_cli_path);
        merge_json_object(&mut merged, &sanitized);

        let mut next: AppConfig = serde_json::from_value(merged).map_err(|e| {
            CoreError::invalid_arguments(
                "invalid_config",
                format!("Invalid config patch result: {e}"),
            )
        })?;
        validate_app_config_values(&next)?;
        next.openvpn.password = current.openvpn.password.clone();
        next.openvpn.otp_secret = current.openvpn.otp_secret.clone();
        next.boundary.password = current.boundary.password.clone();
        next.boundary.oidc_password = current.boundary.oidc_password.clone();
        next.boundary.oidc_otp_secret = current.boundary.oidc_otp_secret.clone();

        let mut next_sync = current_sync.clone();
        let vpn_patch = sanitized.get("openvpn").and_then(Value::as_object);
        let boundary_patch = sanitized.get("boundary").and_then(Value::as_object);
        if vpn_patch.is_some_and(|m| m.contains_key("executablePath")) {
            next_sync.openvpn_executable_path = next.openvpn.executable_path.trim().to_string();
        }
        if vpn_patch.is_some_and(|m| m.contains_key("ovpnPath")) {
            next_sync.openvpn_ovpn_path = next.openvpn.ovpn_path.trim().to_string();
        }
        if boundary_patch.is_some_and(|m| m.contains_key("cliPath")) {
            next_sync.boundary_cli_path = next.boundary.cli_path.trim().to_string();
        }
        let next_rev = config_revision(&next, &next_sync)?;

        let before_view = redacted_config_summary(&current, &current_sync);
        let after_view = redacted_config_summary(&next, &next_sync);
        if preview {
            return Ok(json!({
                "preview": true,
                "action": "patch",
                "revision": current_rev,
                "nextRevision": next_rev,
                "before": before_view,
                "after": after_view,
                "changed": current_rev != next_rev,
            }));
        }

        self.persist_config_and_sync_delta(&current_sync, next_sync, next.clone())?;
        Ok(json!({
            "preview": false,
            "action": "patch",
            "revision": next_rev,
            "previousRevision": current_rev,
            "appConfig": after_view,
            "changed": current_rev != next_rev,
        }))
    }

    /// Saves a full MCP config payload, routing non-empty secrets to the keyring while
    /// preserving omitted keyring secrets and omitted local executable/profile paths.
    pub fn save_app_config_mcp(&self, raw: &Value) -> Result<AppConfig, CoreError> {
        let mut sanitized = raw.clone();
        let secrets = validate_mcp_config_object(&mut sanitized)?;
        if !sanitized.get("app").is_some_and(Value::is_object) {
            return Err(CoreError::invalid_arguments(
                "invalid_config",
                "Full config save requires 'app' object with defaultTerminalCols and defaultTerminalRows",
            ));
        }
        let mut next: AppConfig = serde_json::from_value(sanitized.clone()).map_err(|e| {
            CoreError::invalid_arguments("invalid_config", format!("Invalid config JSON: {e}"))
        })?;
        validate_app_config_values(&next)?;

        let _guard = self.gate().enter()?;
        let current = self.get_app_config()?;
        let current_sync = self.get_sync_settings()?;
        let mut next_sync = current_sync.clone();
        apply_optional_local_path_updates(&sanitized, &current_sync, &mut next_sync, &mut next);
        self.apply_extracted_config_secrets(&current, &secrets, &mut next)?;
        self.persist_config_and_sync_delta(&current_sync, next_sync, next.clone())?;
        Ok(next)
    }

    /// Patches the active configuration from an MCP JSON payload, routing non-empty secrets
    /// to the keyring while preserving existing keyring secrets when omitted or empty.
    pub fn patch_app_config_mcp(&self, patch: &Value) -> Result<AppConfig, CoreError> {
        let mut sanitized = patch.clone();
        let secrets = validate_mcp_config_object(&mut sanitized)?;

        let _guard = self.gate().enter()?;
        let current = self.get_app_config()?;
        let current_sync = self.get_sync_settings()?;
        let mut merged = serde_json::to_value(AppConfigForDisk::from(&current)).map_err(|e| {
            CoreError::operation_failed(
                "config_encode_failed",
                e.to_string(),
                false,
                "fix_configuration",
            )
        })?;
        merged["openvpn"]["executablePath"] = json!(current_sync.openvpn_executable_path);
        merged["openvpn"]["ovpnPath"] = json!(current_sync.openvpn_ovpn_path);
        merged["boundary"]["cliPath"] = json!(current_sync.boundary_cli_path);
        merge_json_object(&mut merged, &sanitized);

        let mut next: AppConfig = serde_json::from_value(merged).map_err(|e| {
            CoreError::invalid_arguments(
                "invalid_config",
                format!("Invalid config patch result: {e}"),
            )
        })?;
        validate_app_config_values(&next)?;

        let mut next_sync = current_sync.clone();
        apply_optional_local_path_updates(&sanitized, &current_sync, &mut next_sync, &mut next);
        self.apply_extracted_config_secrets(&current, &secrets, &mut next)?;
        self.persist_config_and_sync_delta(&current_sync, next_sync, next.clone())?;
        Ok(next)
    }

    fn apply_extracted_config_secrets(
        &self,
        current: &AppConfig,
        secrets: &ExtractedConfigSecrets,
        next: &mut AppConfig,
    ) -> Result<(), CoreError> {
        let mut store = self.secrets_arc().lock();
        let slots: [(&str, Option<&Zeroizing<String>>, &str, &mut String); 5] = [
            (
                KEYRING_OPENVPN_PASSWORD,
                secrets.openvpn_password.as_ref(),
                current.openvpn.password.as_str(),
                &mut next.openvpn.password,
            ),
            (
                KEYRING_OPENVPN_OTP_SECRET,
                secrets.openvpn_otp_secret.as_ref(),
                current.openvpn.otp_secret.as_str(),
                &mut next.openvpn.otp_secret,
            ),
            (
                KEYRING_BOUNDARY_PASSWORD,
                secrets.boundary_password.as_ref(),
                current.boundary.password.as_str(),
                &mut next.boundary.password,
            ),
            (
                KEYRING_BOUNDARY_OIDC_PASSWORD,
                secrets.boundary_oidc_password.as_ref(),
                current.boundary.oidc_password.as_str(),
                &mut next.boundary.oidc_password,
            ),
            (
                KEYRING_BOUNDARY_OIDC_OTP_SECRET,
                secrets.boundary_oidc_otp_secret.as_ref(),
                current.boundary.oidc_otp_secret.as_str(),
                &mut next.boundary.oidc_otp_secret,
            ),
        ];
        for (account, incoming, current_val, target_slot) in slots {
            *target_slot = match incoming {
                Some(v) => {
                    store.set(account, v)?;
                    v.to_string()
                }
                None => current_val.to_string(),
            };
        }
        Ok(())
    }

    fn persist_config_and_sync_delta(
        &self,
        current_sync: &SyncSettings,
        next_sync: SyncSettings,
        next: AppConfig,
    ) -> Result<(), CoreError> {
        if next_sync.openvpn_executable_path != current_sync.openvpn_executable_path
            || next_sync.openvpn_ovpn_path != current_sync.openvpn_ovpn_path
            || next_sync.boundary_cli_path != current_sync.boundary_cli_path
        {
            write_sync_settings_atomic(&self.profile_paths().sync_local_path, &next_sync)?;
        }
        write_app_config_atomic(&self.profile_paths().config_path, &next)?;
        self.replace_app_config_and_sync(next, next_sync);
        Ok(())
    }

    pub fn patch_sync_settings(
        &self,
        patch_json: &Value,
        preview: bool,
        expected_revision: Option<&str>,
    ) -> Result<Value, CoreError> {
        check_string_bounds(patch_json, "settings")?;
        let obj = as_object(patch_json, "settings")?;
        reject_unknown_keys(obj, SYNC_ALLOWED_KEYS, "sync settings")?;
        let patch: SyncSettingsPatch = serde_json::from_value(patch_json.clone()).map_err(|e| {
            CoreError::invalid_arguments(
                "invalid_sync_settings",
                format!("Invalid sync settings patch: {e}"),
            )
        })?;
        reject_forbidden_sync_fields(&patch)?;

        let _guard = self.gate().enter()?;
        let current_sync = self.get_sync_settings()?;
        let current_rev = sync_revision(&current_sync)?;
        check_expected_revision(&current_rev, expected_revision)?;

        let mut next_sync = current_sync.clone();
        apply_sync_patch(&mut next_sync, patch)?;
        let next_rev = sync_revision(&next_sync)?;

        let before_view = redacted_sync_summary(&current_sync);
        let after_view = redacted_sync_summary(&next_sync);
        if preview {
            return Ok(json!({
                "preview": true,
                "action": "patch",
                "revision": current_rev,
                "nextRevision": next_rev,
                "before": before_view,
                "after": after_view,
                "changed": current_rev != next_rev,
            }));
        }

        write_sync_settings_atomic(&self.profile_paths().sync_local_path, &next_sync)?;
        let mut next_cfg = self.get_app_config()?;
        next_cfg.openvpn.executable_path = next_sync.openvpn_executable_path.clone();
        next_cfg.openvpn.ovpn_path = next_sync.openvpn_ovpn_path.clone();
        next_cfg.boundary.cli_path = next_sync.boundary_cli_path.clone();
        self.replace_app_config_and_sync(next_cfg, next_sync);
        Ok(json!({
            "preview": false,
            "action": "patch",
            "revision": next_rev,
            "previousRevision": current_rev,
            "syncSettings": after_view,
            "changed": current_rev != next_rev,
        }))
    }

    pub fn push_config_sync(
        &self,
        repo_root_override: Option<&str>,
        preview: bool,
        expected_revision: Option<&str>,
    ) -> Result<Value, CoreError> {
        let _guard = self.gate().enter()?;
        let sync = self.get_sync_settings()?;
        let raw_root = repo_root_override
            .map(str::trim)
            .unwrap_or_else(|| sync.repo_root.trim());
        if raw_root.is_empty() {
            return Err(CoreError::invalid_arguments(
                "repo_root_required",
                "Sync repoRoot is required (pass --repo-root or configure syncSettings.repoRoot)",
            ));
        }
        let root_path = Path::new(raw_root);
        if !root_path.is_dir() {
            return Err(CoreError::invalid_arguments(
                "invalid_repo_root",
                format!("Sync repoRoot directory does not exist: {raw_root}"),
            ));
        }
        let canon_root = root_path.canonicalize().map_err(|e| {
            CoreError::operation_failed(
                "repo_root_failed",
                e.to_string(),
                false,
                "fix_configuration",
            )
        })?;
        let canon_profile = self
            .profile_paths()
            .profile_dir
            .canonicalize()
            .map_err(|e| {
                CoreError::operation_failed(
                    "profile_dir_failed",
                    e.to_string(),
                    false,
                    "fix_configuration",
                )
            })?;
        if canon_root == canon_profile || canon_root.starts_with(&canon_profile) {
            return Err(CoreError::policy_denied(
                "unsafe_sync_destination",
                "Sync destination cannot be inside the profile directory",
                "select_external_repo_root",
            ));
        }

        let app_cfg = self.get_app_config()?;
        let servers = self.get_servers_snapshot();
        let cfg_bytes =
            serde_json::to_vec_pretty(&AppConfigForDisk::from(&app_cfg)).map_err(|e| {
                CoreError::operation_failed(
                    "config_encode_failed",
                    e.to_string(),
                    false,
                    "fix_configuration",
                )
            })?;
        let write_servers = self.profile_paths().servers_path.exists() || !servers.is_empty();
        let servers_bytes = if write_servers {
            Some(
                serde_json::to_vec_pretty(&ServersFile { servers }).map_err(|e| {
                    CoreError::operation_failed(
                        "servers_encode_failed",
                        e.to_string(),
                        false,
                        "fix_configuration",
                    )
                })?,
            )
        } else {
            None
        };

        let app_config_dir = canon_root.join("app-config");
        let target_cfg_path = app_config_dir.join("config.json");
        let target_srv_path = app_config_dir.join("servers.json");
        let existing_cfg = backup_io::read_bounded(&target_cfg_path).map_err(|e| {
            CoreError::operation_failed("sync_read_failed", e, false, "fix_configuration")
        })?;
        let existing_srv = backup_io::read_bounded(&target_srv_path).map_err(|e| {
            CoreError::operation_failed("sync_read_failed", e, false, "fix_configuration")
        })?;

        let current_rev = compute_push_revision(
            &cfg_bytes,
            servers_bytes.as_deref(),
            existing_cfg.as_deref(),
            existing_srv.as_deref(),
        );
        check_expected_revision(&current_rev, expected_revision)?;

        let cfg_changed = existing_cfg.as_deref() != Some(cfg_bytes.as_slice());
        let srv_changed = servers_bytes
            .as_ref()
            .is_some_and(|bytes| existing_srv.as_deref() != Some(bytes.as_slice()));
        let next_rev = compute_push_revision(
            &cfg_bytes,
            servers_bytes.as_deref(),
            Some(&cfg_bytes),
            servers_bytes.as_deref().or(existing_srv.as_deref()),
        );
        let mut files = vec![json!({"name": "config.json", "changed": cfg_changed})];
        if servers_bytes.is_some() {
            files.push(json!({"name": "servers.json", "changed": srv_changed}));
        }

        if preview {
            return Ok(json!({
                "preview": true,
                "repoRoot": canon_root,
                "targetDir": app_config_dir,
                "revision": current_rev,
                "nextRevision": next_rev,
                "files": files,
                "changed": cfg_changed || srv_changed,
            }));
        }

        std::fs::create_dir_all(&app_config_dir).map_err(|e| {
            CoreError::operation_failed(
                "sync_write_failed",
                format!("Failed to create {}: {e}", app_config_dir.display()),
                false,
                "fix_configuration",
            )
        })?;
        backup_io::atomic_write(&target_cfg_path, &cfg_bytes, true).map_err(|e| {
            CoreError::operation_failed("sync_write_failed", e, false, "fix_configuration")
        })?;
        if let Some(bytes) = &servers_bytes {
            backup_io::atomic_write(&target_srv_path, bytes, true).map_err(|e| {
                CoreError::operation_failed("sync_write_failed", e, false, "fix_configuration")
            })?;
        }

        Ok(json!({
            "preview": false,
            "repoRoot": canon_root,
            "targetDir": app_config_dir,
            "revision": next_rev,
            "previousRevision": current_rev,
            "files": files,
            "changed": cfg_changed || srv_changed,
        }))
    }

    pub fn launch_server_rdp(
        &self,
        server_id: &str,
        interactive: bool,
    ) -> Result<Value, CoreError> {
        let server = self.get_server(server_id)?;
        let gui_ready = std::env::consts::OS != "linux"
            || ["DISPLAY", "WAYLAND_DISPLAY"]
                .iter()
                .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()));
        let host = server.host.trim();
        validate_rdp_target(
            std::env::consts::OS,
            interactive && gui_ready,
            server.os == ServerOs::Windows,
            host,
        )?;
        spawn_rdp_client(host)?;
        Ok(json!({
            "launched": true,
            "serverId": server.id,
            "host": host,
        }))
    }

    pub fn export_settings_backup(
        &self,
        output_path: &Path,
        passphrase: &str,
        overwrite: bool,
    ) -> Result<Value, CoreError> {
        if passphrase.chars().count() < 12 || passphrase.len() > 65536 {
            return Err(CoreError::invalid_arguments(
                "invalid_passphrase",
                "Backup passphrase must be at least 12 characters",
            ));
        }
        let _maint = self.gate().maintain()?;
        let output = validate_export_target(output_path, &self.profile_paths().profile_dir, false)
            .map_err(|e| CoreError::invalid_arguments("unsafe_export_path", e))?;
        let mut secrets = self.secrets_arc().lock();
        let archive = snapshot_with_version(
            &self.profile_paths().profile_dir,
            env!("CARGO_PKG_VERSION"),
            &mut *secrets,
        )
        .map_err(|e| {
            CoreError::operation_failed(
                "backup_snapshot_failed",
                e,
                false,
                "unlock_keyring_and_retry",
            )
        })?;
        let bytes = crypto::encrypt(&archive, passphrase).map_err(|e| {
            CoreError::operation_failed("backup_encrypt_failed", e, false, "retry_later")
        })?;
        backup_io::atomic_write(&output, &bytes, overwrite).map_err(|e| {
            CoreError::operation_failed("backup_write_failed", e, false, "fix_configuration")
        })?;
        let sections: Vec<Value> = crate::backup::FILES
            .iter()
            .map(|name| {
                json!({
                    "name": name,
                    "state": if archive.files[*name].is_some() { "included" } else { "absent" }
                })
            })
            .collect();
        Ok(json!({
            "complete": true,
            "outputPath": output,
            "servers": archive.server_ids().map(|v| v.len()).unwrap_or(0),
            "credentialsPresent": archive.credentials.values().filter(|v| v.is_some()).count(),
            "credentialsAbsent": archive.credentials.values().filter(|v| v.is_none()).count(),
            "sections": sections,
        }))
    }

    pub fn prepare_settings_import(
        &self,
        input_path: &Path,
        passphrase: &str,
        mut options: RestoreOptions,
        ttl: Duration,
    ) -> Result<(Preview, PendingRestore), CoreError> {
        options.recover_identity = false;
        let bytes = Zeroizing::new(
            backup_io::read_bounded(input_path)
                .map_err(|e| CoreError::invalid_arguments("invalid_backup_file", e))?
                .ok_or_else(|| {
                    CoreError::not_found(
                        "backup_not_found",
                        format!("Backup file not found: {}", input_path.display()),
                        "check_backup_path",
                    )
                })?,
        );
        let source: crate::backup::Archive = crypto::decrypt(&bytes, passphrase)
            .map_err(|e| CoreError::invalid_arguments("backup_decrypt_failed", e))?;
        source
            .validate()
            .map_err(|e| CoreError::invalid_arguments("invalid_backup_archive", e))?;

        let _maint = self.gate().maintain()?;
        let mut secrets = self.secrets_arc().lock();
        let destination = snapshot_with_version(
            &self.profile_paths().profile_dir,
            env!("CARGO_PKG_VERSION"),
            &mut *secrets,
        )
        .map_err(|e| {
            CoreError::operation_failed(
                "backup_snapshot_failed",
                e,
                false,
                "unlock_keyring_and_retry",
            )
        })?;
        let mut preview = describe(&source, &destination, &options)
            .map_err(|e| CoreError::invalid_arguments("invalid_restore_options", e))?;
        let pending =
            PendingRestore::new(source, options, &destination, self.gate().revision(), ttl)
                .map_err(|e| {
                    CoreError::operation_failed("preview_failed", e, false, "retry_later")
                })?;
        preview.id = pending.id.clone();
        Ok((preview, pending))
    }

    pub fn refresh_pending_import_preview(
        &self,
        pending: &mut PendingRestore,
        mut new_options: RestoreOptions,
        ttl: Duration,
    ) -> Result<Preview, CoreError> {
        new_options.recover_identity = false;
        let _maint = self.gate().maintain()?;
        let mut secrets = self.secrets_arc().lock();
        let destination = snapshot_with_version(
            &self.profile_paths().profile_dir,
            env!("CARGO_PKG_VERSION"),
            &mut *secrets,
        )
        .map_err(|e| {
            CoreError::operation_failed(
                "backup_snapshot_failed",
                e,
                false,
                "unlock_keyring_and_retry",
            )
        })?;
        let mut preview = describe(&pending.source, &destination, &new_options)
            .map_err(|e| CoreError::invalid_arguments("invalid_restore_options", e))?;
        let refreshed = PendingRestore::new(
            pending.source.clone(),
            new_options,
            &destination,
            self.gate().revision(),
            ttl,
        )
        .map_err(|e| CoreError::operation_failed("preview_failed", e, false, "retry_later"))?;
        preview.id = refreshed.id.clone();
        *pending = refreshed;
        Ok(preview)
    }

    pub fn commit_pending_import(
        &self,
        pending: PendingRestore,
        preview_id: &str,
    ) -> Result<Value, CoreError> {
        let _maint = self.gate().maintain()?;
        pending.verify_identity(preview_id).map_err(|e| {
            CoreError::conflict("preview_expired_or_replaced", e, false, "preview_again")
        })?;
        self.require_quiescent_for_import()?;
        let mut secrets = self.secrets_arc().lock();
        let dir = &self.profile_paths().profile_dir;
        let destination = snapshot_with_version(dir, env!("CARGO_PKG_VERSION"), &mut *secrets)
            .map_err(|e| {
                CoreError::operation_failed(
                    "backup_snapshot_failed",
                    e,
                    false,
                    "unlock_keyring_and_retry",
                )
            })?;
        pending
            .verify_destination(&destination, self.gate().revision())
            .map_err(|e| {
                CoreError::conflict(
                    "destination_changed_since_preview",
                    e,
                    false,
                    "preview_again",
                )
            })?;
        let mut options = pending.options.clone();
        options.recover_identity = false;
        let changes = compile_restore(&pending.source, &destination, &options)
            .map_err(|e| CoreError::invalid_arguments("invalid_restore_options", e))?;
        if let Err(err) = transaction::commit(dir, &changes, &mut *secrets) {
            if self.profile_paths().journal_path.exists() {
                self.gate().block(err.clone());
                return Err(CoreError::conflict(
                    "recovery_required",
                    err,
                    false,
                    "unlock_keyring_and_retry",
                ));
            }
            return Err(CoreError::operation_failed(
                "backup_import_failed",
                err,
                false,
                "inspect_status",
            ));
        }
        drop(secrets);
        self.reload_persisted_state()?;
        self.gate().changed();
        Ok(json!({
            "applied": true,
            "previewId": preview_id,
            "credentialNamespace": self.credential_namespace(),
        }))
    }
}

fn compute_push_revision(
    cfg: &[u8],
    srv: Option<&[u8]>,
    existing_cfg: Option<&[u8]>,
    existing_srv: Option<&[u8]>,
) -> String {
    let mut hash = Sha256::new();
    for (tag, data) in [
        (&b"cfg"[..], Some(cfg)),
        (&b"srv"[..], srv),
        (&b"ecfg"[..], existing_cfg),
        (&b"esrv"[..], existing_srv),
    ] {
        hash.update(tag);
        if let Some(bytes) = data {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        } else {
            hash.update(0u64.to_le_bytes());
        }
    }
    let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

fn spawn_rdp_client(host: &str) -> Result<(), CoreError> {
    if let Some(launcher) = std::env::var_os("SCPCTL_RDP_LAUNCHER").filter(|v| !v.is_empty()) {
        let arg = if cfg!(target_os = "windows") {
            format!("/v:{host}")
        } else {
            format!("rdp://{host}")
        };
        let status = Command::new(PathBuf::from(launcher))
            .arg(&arg)
            .status()
            .map_err(|e| {
                CoreError::operation_failed(
                    "rdp_launch_failed",
                    format!("Failed to launch RDP client: {e}"),
                    false,
                    "inspect_capabilities",
                )
            })?;
        if !status.success() {
            return Err(CoreError::operation_failed(
                "rdp_launch_failed",
                format!("RDP client launcher exited with {:?}", status.code()),
                false,
                "inspect_capabilities",
            ));
        }
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = Command::new("mstsc.exe");
        cmd.arg(format!("/v:{host}"));
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.spawn().map_err(|e| {
            CoreError::operation_failed(
                "rdp_launch_failed",
                format!("Failed to launch mstsc.exe: {e}"),
                false,
                "inspect_capabilities",
            )
        })?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let url = format!("rdp://{host}");
        let status = Command::new("open").arg(&url).status().map_err(|e| {
            CoreError::operation_failed(
                "rdp_launch_failed",
                format!("Failed to launch RDP URL via open: {e}"),
                false,
                "inspect_capabilities",
            )
        })?;
        if !status.success() {
            return Err(CoreError::operation_failed(
                "rdp_launch_failed",
                "Failed to launch RDP client via open",
                false,
                "inspect_capabilities",
            ));
        }
        Ok(())
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let url = format!("rdp://{host}");
        let status = Command::new("xdg-open").arg(&url).status().map_err(|e| {
            CoreError::operation_failed(
                "rdp_launch_failed",
                format!("Failed to launch RDP URL via xdg-open: {e}"),
                false,
                "inspect_capabilities",
            )
        })?;
        if !status.success() {
            return Err(CoreError::operation_failed(
                "rdp_launch_failed",
                "Failed to launch RDP client via xdg-open",
                false,
                "inspect_capabilities",
            ));
        }
        Ok(())
    }
}
