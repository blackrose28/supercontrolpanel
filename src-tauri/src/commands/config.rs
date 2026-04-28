use std::{env, fs, path::PathBuf, process::Command};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::cc::{init_cc_connection, sync_cc_status_from_settings};
use crate::mcp::{sync_mcp_runtime_from_settings, McpRuntimeState};
use crate::models::{AppConfig, AppConfigForDisk};
use crate::services::{
    config_file_path, default_app_config, detect_openvpn_executable_path, keyring_get_optional,
    keyring_set_or_delete, load_sync_settings_from_disk, save_sync_settings_to_disk,
    servers_file_path, ConfigPaths, SyncSettings, SyncSettingsPatch,
    KEYRING_BOUNDARY_OIDC_OTP_SECRET, KEYRING_BOUNDARY_OIDC_PASSWORD, KEYRING_BOUNDARY_PASSWORD,
    KEYRING_OPENVPN_OTP_SECRET, KEYRING_OPENVPN_PASSWORD,
};
use crate::{CONFIG_FILE_NAME, SERVERS_FILE_NAME, SYNC_SETTINGS_FILE_NAME};

pub(crate) fn load_app_config_from_disk(app: &AppHandle) -> Result<AppConfig, String> {
    let path = config_file_path(app, CONFIG_FILE_NAME)?;
    let mut config = if !path.exists() {
        default_app_config()
    } else {
        let raw = fs::read_to_string(&path).map_err(|e| format!("failed to read config: {e}"))?;
        serde_json::from_str::<AppConfig>(&raw)
            .map_err(|e| format!("invalid config format: {e}"))?
    };

    let mut needs_disk_rewrite = false;
    let mut sync_settings = load_sync_settings_from_disk(app, SYNC_SETTINGS_FILE_NAME)?;
    let mut needs_sync_settings_rewrite = false;

    if sync_settings.openvpn_executable_path.trim().is_empty()
        && !config.openvpn.executable_path.trim().is_empty()
    {
        sync_settings.openvpn_executable_path = config.openvpn.executable_path.clone();
        needs_sync_settings_rewrite = true;
        needs_disk_rewrite = true;
    }
    if sync_settings.openvpn_ovpn_path.trim().is_empty()
        && !config.openvpn.ovpn_path.trim().is_empty()
    {
        sync_settings.openvpn_ovpn_path = config.openvpn.ovpn_path.clone();
        needs_sync_settings_rewrite = true;
        needs_disk_rewrite = true;
    }
    if sync_settings.boundary_cli_path.trim().is_empty()
        && !config.boundary.cli_path.trim().is_empty()
    {
        sync_settings.boundary_cli_path = config.boundary.cli_path.clone();
        needs_sync_settings_rewrite = true;
        needs_disk_rewrite = true;
    }

    config.openvpn.executable_path = sync_settings.openvpn_executable_path.clone();
    config.openvpn.ovpn_path = sync_settings.openvpn_ovpn_path.clone();
    config.boundary.cli_path = sync_settings.boundary_cli_path.clone();
    if config.openvpn.command.trim().is_empty() && !config.openvpn.executable_path.trim().is_empty()
    {
        config.openvpn.command = config.openvpn.executable_path.clone();
    }

    match keyring_get_optional(KEYRING_OPENVPN_PASSWORD)? {
        Some(p) => config.openvpn.password = p,
        None => {
            if !config.openvpn.password.is_empty() {
                keyring_set_or_delete(KEYRING_OPENVPN_PASSWORD, &config.openvpn.password)?;
                needs_disk_rewrite = true;
            }
        }
    }

    match keyring_get_optional(KEYRING_OPENVPN_OTP_SECRET)? {
        Some(s) => config.openvpn.otp_secret = s,
        None => {
            if !config.openvpn.otp_secret.is_empty() {
                keyring_set_or_delete(KEYRING_OPENVPN_OTP_SECRET, &config.openvpn.otp_secret)?;
                needs_disk_rewrite = true;
            }
        }
    }

    match keyring_get_optional(KEYRING_BOUNDARY_PASSWORD)? {
        Some(p) => config.boundary.password = p,
        None => {
            if !config.boundary.password.is_empty() {
                keyring_set_or_delete(KEYRING_BOUNDARY_PASSWORD, &config.boundary.password)?;
                needs_disk_rewrite = true;
            }
        }
    }

    match keyring_get_optional(KEYRING_BOUNDARY_OIDC_PASSWORD)? {
        Some(p) => config.boundary.oidc_password = p,
        None => {
            if !config.boundary.oidc_password.is_empty() {
                keyring_set_or_delete(
                    KEYRING_BOUNDARY_OIDC_PASSWORD,
                    &config.boundary.oidc_password,
                )?;
                needs_disk_rewrite = true;
            }
        }
    }

    match keyring_get_optional(KEYRING_BOUNDARY_OIDC_OTP_SECRET)? {
        Some(s) => config.boundary.oidc_otp_secret = s,
        None => {
            if !config.boundary.oidc_otp_secret.is_empty() {
                keyring_set_or_delete(
                    KEYRING_BOUNDARY_OIDC_OTP_SECRET,
                    &config.boundary.oidc_otp_secret,
                )?;
                needs_disk_rewrite = true;
            }
        }
    }

    if needs_disk_rewrite {
        save_app_config_to_disk(app, &config)?;
    }
    if needs_sync_settings_rewrite {
        save_sync_settings_to_disk(app, &sync_settings, SYNC_SETTINGS_FILE_NAME)?;
    }

    Ok(config)
}

fn save_app_config_to_disk(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = config_file_path(app, CONFIG_FILE_NAME)?;
    let disk = AppConfigForDisk::from(config);
    let serialized =
        serde_json::to_string_pretty(&disk).map_err(|e| format!("failed to encode config: {e}"))?;
    fs::write(path, serialized).map_err(|e| format!("failed to write config: {e}"))
}

fn save_local_app_settings(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let mut sync_settings = load_sync_settings_from_disk(app, SYNC_SETTINGS_FILE_NAME)?;
    sync_settings.openvpn_executable_path = config.openvpn.executable_path.trim().to_string();
    sync_settings.openvpn_ovpn_path = config.openvpn.ovpn_path.trim().to_string();
    sync_settings.boundary_cli_path = config.boundary.cli_path.trim().to_string();
    save_sync_settings_to_disk(app, &sync_settings, SYNC_SETTINGS_FILE_NAME)
}

pub(crate) fn get_app_config_action(app: AppHandle) -> Result<AppConfig, String> {
    let config = load_app_config_from_disk(&app)?;
    scp_debug!(
        "get_app_config: exe_len={} ovpn_len={} user_len={} password_in_memory={} otp_configured={}",
        config.openvpn.executable_path.len(),
        config.openvpn.ovpn_path.len(),
        config.openvpn.username.len(),
        !config.openvpn.password.is_empty(),
        !config.openvpn.otp_secret.is_empty()
    );
    save_app_config_to_disk(&app, &config)?;
    Ok(config)
}

#[tauri::command]
pub fn get_app_config(app: AppHandle) -> Result<AppConfig, String> {
    get_app_config_action(app)
}

#[tauri::command]
pub fn get_config_paths(app: AppHandle) -> Result<ConfigPaths, String> {
    super::get_config_paths_internal(&app)
}

#[tauri::command]
pub fn get_sync_settings(app: AppHandle) -> Result<SyncSettings, String> {
    load_sync_settings_from_disk(&app, SYNC_SETTINGS_FILE_NAME)
}

pub(crate) fn save_sync_settings_action(
    app: AppHandle,
    settings: SyncSettingsPatch,
    mcp_state: State<'_, McpRuntimeState>,
) -> Result<SyncSettings, String> {
    let mut merged = load_sync_settings_from_disk(&app, SYNC_SETTINGS_FILE_NAME)?;
    if let Some(repo_root) = settings.repo_root {
        merged.repo_root = repo_root;
    }
    if let Some(openvpn_executable_path) = settings.openvpn_executable_path {
        if !openvpn_executable_path.is_empty() {
            merged.openvpn_executable_path = openvpn_executable_path;
        }
    }
    if let Some(openvpn_ovpn_path) = settings.openvpn_ovpn_path {
        if !openvpn_ovpn_path.is_empty() {
            merged.openvpn_ovpn_path = openvpn_ovpn_path;
        }
    }
    if let Some(boundary_cli_path) = settings.boundary_cli_path {
        if !boundary_cli_path.is_empty() {
            merged.boundary_cli_path = boundary_cli_path;
        }
    }
    if let Some(caddy_folder) = settings.caddy_folder {
        merged.caddy_folder = caddy_folder;
    }
    if let Some(mcp_enabled) = settings.mcp_enabled {
        merged.mcp_enabled = mcp_enabled;
    }
    if let Some(mcp_bind_address) = settings.mcp_bind_address {
        if !mcp_bind_address.trim().is_empty() {
            merged.mcp_bind_address = mcp_bind_address;
        }
    }
    if let Some(mcp_port) = settings.mcp_port {
        if mcp_port != 0 {
            merged.mcp_port = mcp_port;
        }
    }
    if let Some(mcp_auth_token) = settings.mcp_auth_token {
        if !mcp_auth_token.trim().is_empty() {
            merged.mcp_auth_token = mcp_auth_token;
        }
    }
    if let Some(mcp_allowed_tool_categories) = settings.mcp_allowed_tool_categories {
        if !mcp_allowed_tool_categories.is_empty() {
            merged.mcp_allowed_tool_categories = mcp_allowed_tool_categories;
        }
    }
    if let Some(mcp_sensitive_action_policy) = settings.mcp_sensitive_action_policy {
        if !mcp_sensitive_action_policy.trim().is_empty() {
            merged.mcp_sensitive_action_policy = mcp_sensitive_action_policy;
        }
    }
    if let Some(cc_enabled) = settings.cc_enabled {
        merged.cc_enabled = cc_enabled;
    }
    if let Some(cc_server_url) = settings.cc_server_url {
        merged.cc_server_url = cc_server_url;
    }
    if let Some(cc_auth_mode) = settings.cc_auth_mode {
        if !cc_auth_mode.trim().is_empty() {
            merged.cc_auth_mode = cc_auth_mode;
        }
    }
    if let Some(cc_agent_token) = settings.cc_agent_token {
        merged.cc_agent_token = cc_agent_token;
    }
    if let Some(cc_device_id) = settings.cc_device_id {
        merged.cc_device_id = cc_device_id;
    }
    if let Some(cc_device_name) = settings.cc_device_name {
        merged.cc_device_name = cc_device_name;
    }
    if let Some(first_run_completed) = settings.first_run_completed {
        merged.first_run_completed = first_run_completed;
    }
    save_sync_settings_to_disk(&app, &merged, SYNC_SETTINGS_FILE_NAME)?;
    let _ = sync_mcp_runtime_from_settings(&app, &mcp_state, &merged, crate::services::now_ms);
    let _ = sync_cc_status_from_settings(&app);
    Ok(merged)
}

#[tauri::command]
pub fn save_sync_settings(
    app: AppHandle,
    settings: SyncSettingsPatch,
    mcp_state: State<'_, McpRuntimeState>,
) -> Result<(), String> {
    save_sync_settings_action(app, settings, mcp_state).map(|_| ())
}

#[tauri::command]
pub async fn enroll_device(
    app: AppHandle,
    server_url: String,
    bootstrap_token: String,
    device_name: Option<String>,
    mcp_state: State<'_, McpRuntimeState>,
) -> Result<EnrollResult, String> {
    let server_url = server_url.trim();
    let bootstrap_token = bootstrap_token.trim();

    if server_url.is_empty() {
        return Err("Server URL cannot be empty".to_string());
    }
    if bootstrap_token.is_empty() {
        return Err("Bootstrap token cannot be empty".to_string());
    }

    let enroll_url = format!(
        "{}/v1/enroll",
        server_url.trim_end_matches('/')
    );

    let body = serde_json::json!({
        "bootstrapToken": bootstrap_token,
        "deviceName": device_name.as_deref().unwrap_or(""),
    });

    let enroll_url_clone = enroll_url.clone();
    let body_string = serde_json::to_string(&body)
        .map_err(|e| format!("Failed to encode enrollment body: {e}"))?;

    let (status_code, response_body) = tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

        let response = client
            .post(&enroll_url_clone)
            .header("Content-Type", "application/json")
            .body(body_string)
            .send()
            .map_err(|e| format!("Enrollment request failed: {e}"))?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .map_err(|e| format!("Failed to read enrollment response: {e}"))?;
        Ok::<_, String>((status, body))
    })
    .await
    .map_err(|e| format!("Enrollment task panicked: {e}"))??;

    if !(200..300).contains(&status_code) {
        return Err(format!(
            "Enrollment failed (HTTP {}): {}",
            status_code, response_body
        ));
    }

    let enroll_response: EnrollResponse =
        serde_json::from_str(&response_body)
            .map_err(|e| format!("Invalid enrollment response: {e}"))?;

    let mut merged =
        load_sync_settings_from_disk(&app, SYNC_SETTINGS_FILE_NAME)?;
    merged.cc_enabled = true;
    merged.cc_server_url = server_url.to_string();
    merged.cc_device_id = enroll_response.device_id.clone();
    merged.cc_agent_token = enroll_response.cc_agent_token.clone();
    merged.cc_device_name = enroll_response.device_name.clone();
    merged.first_run_completed = true;
    save_sync_settings_to_disk(&app, &merged, SYNC_SETTINGS_FILE_NAME)?;

    let _ = sync_mcp_runtime_from_settings(
        &app,
        &mcp_state,
        &merged,
        crate::services::now_ms,
    );
    let _ = sync_cc_status_from_settings(&app);

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = init_cc_connection(app_handle).await {
            eprintln!("CC post-enrollment init failed: {err}");
        }
    });

    Ok(EnrollResult {
        device_id: enroll_response.device_id,
        cc_agent_token: enroll_response.cc_agent_token,
        device_name: enroll_response.device_name,
    })
}

#[derive(Serialize, Deserialize)]
struct EnrollResponse {
    #[serde(alias = "deviceId")]
    device_id: String,
    #[serde(alias = "ccAgentToken")]
    cc_agent_token: String,
    #[serde(alias = "deviceName", default)]
    device_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollResult {
    pub device_id: String,
    pub cc_agent_token: String,
    pub device_name: String,
}

#[tauri::command]
pub async fn revoke_my_device(app: AppHandle) -> Result<String, String> {
    let settings =
        load_sync_settings_from_disk(&app, SYNC_SETTINGS_FILE_NAME)?;

    if !settings.cc_enabled
        || settings.cc_agent_token.trim().is_empty()
        || settings.cc_server_url.trim().is_empty()
    {
        return Err("CC is not configured. Nothing to revoke.".to_string());
    }

    let server_url = settings.cc_server_url.trim().to_string();
    let agent_token = settings.cc_agent_token.trim().to_string();

    let revoke_url = format!("{}/v1/revoke", server_url.trim_end_matches('/'));
    let token_for_url = agent_token.clone();

    tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
        let response = client
            .post(&revoke_url)
            .header("Authorization", format!("Bearer {token_for_url}"))
            .send()
            .map_err(|e| format!("Revoke request failed: {e}"))?;
        let status_code = response.status().as_u16();
        if !(200..300).contains(&status_code) {
            let body = response.text().unwrap_or_default();
            return Err(format!("Server revoke failed (HTTP {status_code}): {body}"));
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("Revoke task panicked: {e}"))??;

    let mut merged =
        load_sync_settings_from_disk(&app, SYNC_SETTINGS_FILE_NAME)?;
    merged.cc_enabled = false;
    merged.cc_server_url = String::new();
    merged.cc_agent_token = String::new();
    merged.cc_device_id = String::new();
    merged.cc_device_name = String::new();
    save_sync_settings_to_disk(&app, &merged, SYNC_SETTINGS_FILE_NAME)?;

    let _ = sync_cc_status_from_settings(&app);

    Ok("Device revoked and local CC configuration cleared.".to_string())
}

pub(crate) fn sync_config_pull_action(app: AppHandle, repo_root: String) -> Result<(), String> {
    let repo_root = repo_root.trim();
    if repo_root.is_empty() {
        return Err("repoRoot cannot be empty".into());
    }
    let app_path = config_file_path(&app, CONFIG_FILE_NAME)?;
    let servers_path = servers_file_path(&app, SERVERS_FILE_NAME)?;
    let repo_dir = PathBuf::from(repo_root).join("app-config");
    let repo_config_path = repo_dir.join(CONFIG_FILE_NAME);
    if !repo_config_path.exists() {
        return Err(format!(
            "Repo config not found at {}. Commit app-config/config.json first.",
            repo_config_path.to_string_lossy()
        ));
    }
    if let Some(parent) = app_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create app config dir: {e}"))?;
    }
    fs::copy(&repo_config_path, &app_path)
        .map_err(|e| format!("failed to copy from repo to app config: {e}"))?;

    let repo_servers_path = repo_dir.join(SERVERS_FILE_NAME);
    if repo_servers_path.exists() {
        if let Some(parent) = servers_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create servers config dir: {e}"))?;
        }
        fs::copy(&repo_servers_path, &servers_path)
            .map_err(|e| format!("failed to copy from repo to servers.json: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn sync_config_pull(app: AppHandle, repo_root: String) -> Result<(), String> {
    sync_config_pull_action(app, repo_root)
}

pub(crate) fn sync_config_push_action(app: AppHandle, repo_root: String) -> Result<(), String> {
    let repo_root = repo_root.trim();
    if repo_root.is_empty() {
        return Err("repoRoot cannot be empty".into());
    }
    let app_path = config_file_path(&app, CONFIG_FILE_NAME)?;
    if !app_path.exists() {
        return Err(format!(
            "App config not found at {}. Save config once in the UI first.",
            app_path.to_string_lossy()
        ));
    }
    let servers_path = servers_file_path(&app, SERVERS_FILE_NAME)?;

    let repo_dir = PathBuf::from(repo_root).join("app-config");
    let repo_config_path = repo_dir.join(CONFIG_FILE_NAME);
    let repo_servers_path = repo_dir.join(SERVERS_FILE_NAME);

    if let Some(parent) = repo_config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create repo app-config directory: {e}"))?;
    }
    fs::copy(&app_path, &repo_config_path)
        .map_err(|e| format!("failed to copy from app config to repo: {e}"))?;

    if servers_path.exists() {
        if let Some(parent) = repo_servers_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create repo app-config directory: {e}"))?;
        }
        fs::copy(&servers_path, &repo_servers_path)
            .map_err(|e| format!("failed to copy from servers.json to repo: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn sync_config_push(app: AppHandle, repo_root: String) -> Result<(), String> {
    sync_config_push_action(app, repo_root)
}

fn config_git_computer_name() -> String {
    if cfg!(target_os = "windows") {
        if let Ok(v) = env::var("COMPUTERNAME") {
            let t = v.trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    if let Ok(v) = env::var("HOSTNAME") {
        let t = v.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    "unknown-computer".to_string()
}

fn run_git(repo_root: &str, args: &[&str]) -> Result<String, String> {
    let root = repo_root.trim();
    if root.is_empty() {
        return Err("repoRoot cannot be empty".into());
    }
    let git_dir = PathBuf::from(root).join(".git");
    if !git_dir.exists() {
        return Err(format!(
            "Not a git repo: {} (missing .git)",
            PathBuf::from(root).to_string_lossy()
        ));
    }

    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("LD_LIBRARY_PATH")
        .output()
        .map_err(|e| format!("failed to run git {:?}: {e}", args))?;

    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&out.stdout));
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    let combined = combined.trim().to_string();

    if out.status.success() {
        Ok(if combined.is_empty() {
            "OK".into()
        } else {
            combined
        })
    } else {
        let code = out
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".into());
        Err(if combined.is_empty() {
            format!("git {:?} failed (exit code {code})", args)
        } else {
            format!("git {:?} failed (exit code {code}):\n{combined}", args)
        })
    }
}

pub(crate) fn config_git_pull_action(repo_root: String) -> Result<String, String> {
    run_git(repo_root.trim(), &["pull"])
}

#[tauri::command]
pub fn config_git_pull(repo_root: String) -> Result<String, String> {
    config_git_pull_action(repo_root)
}

pub(crate) fn config_git_push_action(repo_root: String) -> Result<String, String> {
    let repo_root = repo_root.trim().to_string();
    if repo_root.is_empty() {
        return Err("repoRoot cannot be empty".into());
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push(run_git(&repo_root, &["add", "."])?);

    let status = run_git(&repo_root, &["status", "--porcelain"])?;
    if status.trim().is_empty() || status.trim() == "OK" {
        return Ok("No changes to commit.".into());
    }

    let computer = config_git_computer_name();
    let msg = format!("Change config from {computer}");
    lines.push(run_git(&repo_root, &["commit", "-m", &msg])?);
    lines.push(run_git(&repo_root, &["push"])?);
    Ok(lines.join("\n"))
}

#[tauri::command]
pub fn config_git_push(repo_root: String) -> Result<String, String> {
    config_git_push_action(repo_root)
}

pub(crate) fn save_app_config_action(
    app: AppHandle,
    mut config: AppConfig,
) -> Result<AppConfig, String> {
    let _incoming_pw_len = config.openvpn.password.len();
    let _incoming_otp_len = config.openvpn.otp_secret.len();
    if !config.openvpn.password.is_empty() {
        scp_debug!("save_app_config: writing password to keyring (len={_incoming_pw_len})");
        keyring_set_or_delete(KEYRING_OPENVPN_PASSWORD, &config.openvpn.password)?;
    } else if let Some(p) = keyring_get_optional(KEYRING_OPENVPN_PASSWORD)? {
        scp_debug!(
            "save_app_config: password empty from UI, restored from keyring (len={})",
            p.len()
        );
        config.openvpn.password = p;
    } else {
        scp_debug!("save_app_config: password empty from UI, keyring has no entry");
    }
    if !config.openvpn.otp_secret.is_empty() {
        scp_debug!("save_app_config: writing OTP secret to keyring (len={_incoming_otp_len})");
        keyring_set_or_delete(KEYRING_OPENVPN_OTP_SECRET, &config.openvpn.otp_secret)?;
    } else if let Some(s) = keyring_get_optional(KEYRING_OPENVPN_OTP_SECRET)? {
        scp_debug!(
            "save_app_config: OTP empty from UI, restored from keyring (len={})",
            s.len()
        );
        config.openvpn.otp_secret = s;
    } else {
        scp_debug!("save_app_config: OTP empty from UI, keyring has no entry");
    }
    if !config.boundary.password.is_empty() {
        keyring_set_or_delete(KEYRING_BOUNDARY_PASSWORD, &config.boundary.password)?;
    } else if let Some(p) = keyring_get_optional(KEYRING_BOUNDARY_PASSWORD)? {
        config.boundary.password = p;
    }
    if !config.boundary.oidc_password.is_empty() {
        keyring_set_or_delete(
            KEYRING_BOUNDARY_OIDC_PASSWORD,
            &config.boundary.oidc_password,
        )?;
    } else if let Some(p) = keyring_get_optional(KEYRING_BOUNDARY_OIDC_PASSWORD)? {
        config.boundary.oidc_password = p;
    }
    if !config.boundary.oidc_otp_secret.is_empty() {
        keyring_set_or_delete(
            KEYRING_BOUNDARY_OIDC_OTP_SECRET,
            &config.boundary.oidc_otp_secret,
        )?;
    } else if let Some(s) = keyring_get_optional(KEYRING_BOUNDARY_OIDC_OTP_SECRET)? {
        config.boundary.oidc_otp_secret = s;
    }
    save_local_app_settings(&app, &config)?;
    save_app_config_to_disk(&app, &config)?;
    scp_debug!(
        "save_app_config: done; password_present={} otp_present={}",
        !config.openvpn.password.is_empty(),
        !config.openvpn.otp_secret.is_empty()
    );
    Ok(config)
}

#[tauri::command]
pub fn save_app_config(app: AppHandle, config: AppConfig) -> Result<AppConfig, String> {
    save_app_config_action(app, config)
}

#[tauri::command]
pub fn detect_openvpn_executable() -> Result<String, String> {
    detect_openvpn_executable_path()
        .ok_or_else(|| "OpenVPN executable not found in PATH/common locations".to_string())
}

#[tauri::command]
pub fn is_first_run(app: AppHandle) -> Result<bool, String> {
    let settings = load_sync_settings_from_disk(&app, SYNC_SETTINGS_FILE_NAME)?;
    Ok(!settings.first_run_completed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_first_run_returns_true_when_flag_not_set() {
        let settings = crate::services::SyncSettings::default();
        assert!(!settings.first_run_completed);
    }

    #[test]
    fn is_first_run_returns_false_when_flag_set() {
        let mut settings = crate::services::SyncSettings::default();
        settings.first_run_completed = true;
        assert!(settings.first_run_completed);
    }

    #[test]
    fn enroll_response_parses_camel_case_json() {
        let json = r#"{"deviceId":"abc-123","ccAgentToken":"tok-456","deviceName":"my-laptop"}"#;
        let resp: EnrollResponse = serde_json::from_str(json).expect("should parse");
        assert_eq!(resp.device_id, "abc-123");
        assert_eq!(resp.cc_agent_token, "tok-456");
        assert_eq!(resp.device_name, "my-laptop");
    }

    #[test]
    fn enroll_response_parses_missing_device_name() {
        let json = r#"{"deviceId":"abc-123","ccAgentToken":"tok-456"}"#;
        let resp: EnrollResponse = serde_json::from_str(json).expect("should parse");
        assert_eq!(resp.device_id, "abc-123");
        assert_eq!(resp.cc_agent_token, "tok-456");
        assert_eq!(resp.device_name, "");
    }
}
