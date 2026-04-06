#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    sync::atomic::AtomicBool,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

mod cc;
use cc::{get_cc_status, init_cc_connection, telemetry::forward_log_event};
use portable_pty::{CommandBuilder, PtySize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

mod app_state;
mod mcp;
mod models;
mod server_mgmt;
mod services;
mod stores;
use mcp::{
    call_mcp_tool, list_mcp_resources, list_mcp_tools, mcp_runtime_status, read_mcp_resource,
    stop_mcp_runtime, sync_mcp_runtime_from_settings, McpResourceDescriptor, McpRuntimeState,
    McpRuntimeStatus, McpToolCallEnvelope, McpToolDescriptor,
};
use models::{
    AppConfig, AppConfigForDisk, BoundaryConfig, BoundaryTargetSummary, FunctionStatus,
    PtyExitPayload, RemoteBrowseEntry, RemoteBrowseNodeInfo, RemoteBrowseSession, ScpTransferState,
    StartFunctionResult,
};
#[cfg(target_os = "windows")]
use models::{HelperStatusFile, OpenVpnLaunchConfig};
use server_mgmt::{load_servers, new_server_id, save_servers, Server};
#[cfg(target_os = "windows")]
use services::build_openvpn_args_windows;
use services::prepare_openvpn_start;
#[cfg(not(target_os = "windows"))]
use services::run_openvpn_automation as run_openvpn_automation_service;
#[cfg(not(target_os = "windows"))]
use services::start_openvpn_pty_session as start_openvpn_pty_session_service;
#[cfg(not(target_os = "windows"))]
use services::stop_openvpn_pty_session as stop_openvpn_pty_session_service;
use services::{
    add_watcher, attach_scp_transfer_session, close_pty_internal,
    close_remote_browser as close_remote_browser_service,
    close_remote_browser_sessions_for_server as close_remote_browser_sessions_for_server_service,
    create_remote_browser_directory as create_remote_browser_directory_service,
    create_scp_transfer_entry, delete_remote_browser_path as delete_remote_browser_path_service,
    emit_pty_exit, emit_pty_output_line,
    list_remote_browser_children as list_remote_browser_children_service, lock_err, now_ms,
    open_pty_with_command_builder, open_remote_browser as open_remote_browser_service,
    passive_pty_command, scp_download_service, scp_upload_service,
    start_caddy_run as start_caddy_run_service, start_scp_download_service,
    start_scp_upload_service, start_server_rdp as start_server_rdp_service,
    start_server_ssh as start_server_ssh_service,
    start_server_tunnel as start_server_tunnel_service,
    stat_remote_browser_path as stat_remote_browser_path_service,
    stop_caddy_run as stop_caddy_run_service, stop_server_ssh as stop_server_ssh_service,
    stop_server_tunnel as stop_server_tunnel_service, update_scp_transfer_entry,
    write_pty_internal,
};
use services::{
    apply_boundary_env, apply_boundary_env_builder, apply_boundary_token_arg_if_present,
    apply_boundary_token_arg_if_present_builder, boundary_addr_base, boundary_cli_path,
    config_file_path, default_app_config, detect_openvpn_executable_path, keyring_get_optional,
    keyring_set_or_delete, load_sync_settings_from_disk, save_sync_settings_to_disk,
    servers_file_path, ConfigPaths, SyncSettings, SyncSettingsPatch, KEYRING_BOUNDARY_APP_TOKEN,
    KEYRING_BOUNDARY_OIDC_OTP_SECRET, KEYRING_BOUNDARY_OIDC_PASSWORD, KEYRING_BOUNDARY_PASSWORD,
    KEYRING_OPENVPN_OTP_SECRET, KEYRING_OPENVPN_PASSWORD,
};
use services::{
    boundary_http_client, build_openvpn_command_unix, generate_openvpn_password,
    generate_totp_code, normalized_openvpn_config,
};
use services::{
    boundary_oidc_init_script as boundary_oidc_init_script_service,
    boundary_token_summary as boundary_token_summary_service,
    complete_boundary_automation_session as complete_boundary_automation_session_service,
    prepare_boundary_auth_start,
    run_boundary_oidc_automation as run_boundary_oidc_automation_service,
    stop_boundary_auth as stop_boundary_auth_service,
    stop_boundary_automation_session as stop_boundary_automation_session_service,
    BoundaryAuthLaunch, BoundaryTokenSummary,
};
#[cfg(target_os = "windows")]
use services::{
    emit_openvpn_log as emit_openvpn_log_service, escape_ps_single_quoted,
    get_openvpn_admin_session, openvpn_runtime_dir, set_openvpn_admin_session,
    start_openvpn_admin_worker as start_openvpn_admin_worker_service,
    write_openvpn_helper_script as write_openvpn_helper_script_service,
};
#[cfg(target_os = "windows")]
use stores::OpenVpnAdminSession;
use stores::{
    BoundaryAutomationSession, BoundaryAutomationStore, FunctionStore, OpenVpnAdminStore,
    OpenVpnReconnectStore, RemoteBrowserStore, ScpTransferStore, SessionStore,
};

const OPENVPN_FUNCTION_ID: &str = "openvpn-connect";
const CADDY_FUNCTION_ID: &str = "caddy-run";
const CONFIG_FILE_NAME: &str = "config.json";
const SERVERS_FILE_NAME: &str = "servers.json";
const SYNC_SETTINGS_FILE_NAME: &str = "sync-local.json";
#[cfg(target_os = "windows")]
const OPENVPN_HELPER_SCRIPT_NAME: &str = "openvpn-admin-helper.ps1";
const BOUNDARY_AUTH_FUNCTION_ID: &str = "boundary-auth";
const BOUNDARY_OIDC_WINDOW_LABEL: &str = "boundary-oidc-automation";

fn server_tunnel_function_id(server_id: &str) -> String {
    format!("server-tunnel-{server_id}")
}
fn server_ssh_function_id(server_id: &str) -> String {
    format!("server-ssh-{server_id}")
}
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Dev-only traces (visible in the terminal that runs `npm run tauri dev`).
#[cfg(debug_assertions)]
macro_rules! scp_debug {
    ($($arg:tt)*) => {
        eprintln!("[SCP-debug] {}", format!($($arg)*))
    };
}
#[cfg(not(debug_assertions))]
macro_rules! scp_debug {
    ($($arg:tt)*) => {};
}

impl Default for FunctionStore {
    fn default() -> Self {
        let statuses = services::default_function_statuses(
            OPENVPN_FUNCTION_ID,
            CADDY_FUNCTION_ID,
            BOUNDARY_AUTH_FUNCTION_ID,
        );
        Self {
            statuses: Mutex::new(statuses),
        }
    }
}

fn load_app_config_from_disk(app: &AppHandle) -> Result<AppConfig, String> {
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

#[cfg(target_os = "windows")]
fn is_windows_pid_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        let _ = CloseHandle(h);
        if ok == 0 {
            return false;
        }
        code == STILL_ACTIVE
    }
}

fn upsert_function_status(
    app: &AppHandle,
    function_store: &FunctionStore,
    function_id: &str,
    state: &str,
    message: &str,
    session_id: Option<String>,
) -> Result<(), String> {
    services::upsert_function_status(app, function_store, function_id, state, message, session_id)
}

fn upsert_boundary_status(
    app: &AppHandle,
    function_store: &FunctionStore,
    state: &str,
    message: &str,
    boundary_token_source: Option<&str>,
    session_id: Option<String>,
) -> Result<(), String> {
    services::upsert_boundary_status(
        app,
        function_store,
        state,
        message,
        boundary_token_source,
        session_id,
        BOUNDARY_AUTH_FUNCTION_ID,
    )
}

fn update_functions_for_session_exit(
    app: &AppHandle,
    function_store: &FunctionStore,
    session_id: &str,
    exit_code: Option<i32>,
    reason: &str,
) -> Result<(), String> {
    let mut changed = Vec::<FunctionStatus>::new();
    let mut openvpn_terminal: Option<(String, String)> = None;
    {
        let mut statuses = lock_err(&function_store.statuses)?;
        for status in statuses.values_mut() {
            if status.session_id.as_deref() != Some(session_id) {
                continue;
            }
            if status.function_id == BOUNDARY_AUTH_FUNCTION_ID {
                status.session_id = None;
                status.updated_at_ms = now_ms();
                let (state, message, source) = match boundary_token_summary(app) {
                    Ok(summary) => (
                        "authenticated".to_string(),
                        summary.message,
                        Some(summary.source.to_string()),
                    ),
                    Err(_) => {
                        if exit_code == Some(0) {
                            (
                                "authenticated".to_string(),
                                "Login completed, but the saved Boundary token could not be verified yet. Refresh Status or retry login."
                                    .to_string(),
                                None,
                            )
                        } else {
                            (
                                "unauthenticated".to_string(),
                                format!(
                                    "Not logged in or login interrupted (exit {}). {}",
                                    exit_code
                                        .map(|v| v.to_string())
                                        .unwrap_or_else(|| "?".to_string()),
                                    if reason.len() > 80 {
                                        format!("{}…", &reason[..80])
                                    } else {
                                        reason.to_string()
                                    }
                                ),
                                None,
                            )
                        }
                    }
                };
                status.state = state;
                status.message = message;
                status.boundary_token_source = source;
                changed.push(status.clone());
                continue;
            }
            let function_id = status.function_id.clone();
            status.state = if exit_code == Some(0) {
                "stopped".to_string()
            } else {
                "error".to_string()
            };
            status.message = format!(
                "PTY exited (code={}, reason={reason})",
                exit_code
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            );
            status.session_id = None;
            status.updated_at_ms = now_ms();
            changed.push(status.clone());
            if function_id == OPENVPN_FUNCTION_ID {
                openvpn_terminal = Some((status.state.clone(), status.message.clone()));
            }
        }
    }

    if let Some((state, message)) = openvpn_terminal {
        handle_openvpn_terminal_state(app, &state, &message);
    }
    for status in changed {
        let _ = app.emit("function-status", status.clone());
        forward_log_event(
            app,
            "function-status",
            status.session_id.as_deref(),
            Some(&status.function_id),
            &status,
        );
    }
    Ok(())
}

/// Probes the currently active Boundary token via `boundary config get-token`.
/// This supports both the CLI-managed keyring token and the app-managed token injected through `BOUNDARY_TOKEN`.
fn boundary_token_summary(app: &AppHandle) -> Result<BoundaryTokenSummary, String> {
    #[cfg(target_os = "windows")]
    {
        boundary_token_summary_service(app, load_app_config_from_disk, CREATE_NO_WINDOW)
    }

    #[cfg(not(target_os = "windows"))]
    {
        boundary_token_summary_service(app, load_app_config_from_disk)
    }
}

fn open_pty_internal(
    cols: u16,
    rows: u16,
    app: &AppHandle,
    store: &SessionStore,
) -> Result<String, String> {
    open_pty_with_command_builder(
        cols,
        rows,
        default_shell_command(),
        app,
        store,
        update_functions_for_session_exit,
    )
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn adapt_command_for_shell(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    #[cfg(target_os = "windows")]
    {
        // PowerShell requires '&' when invoking a quoted executable path.
        if trimmed.starts_with('&') {
            trimmed.to_string()
        } else if trimmed.starts_with('"') || trimmed.starts_with('\'') {
            format!("& {trimmed}")
        } else {
            trimmed.to_string()
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        trimmed.to_string()
    }
}

fn boundary_oidc_init_script(cfg: &BoundaryConfig) -> String {
    boundary_oidc_init_script_service(cfg, generate_totp_code)
}

fn compute_openvpn_reconnect_delay_secs(
    failure_count: u32,
    base_seconds: u64,
    max_seconds: u64,
) -> u64 {
    if failure_count <= 1 {
        return 0;
    }
    let effective_base = base_seconds.max(1);
    let effective_max = max_seconds.max(effective_base);
    let exponent = failure_count.saturating_sub(1).min(62);
    let factor = (1_u128 << exponent).saturating_sub(1);
    let delay = (effective_base as u128).saturating_mul(factor);
    delay.min(effective_max as u128) as u64
}

fn mark_openvpn_connected(app: &AppHandle) {
    if let Ok(mut state) = app.state::<OpenVpnReconnectStore>().state.lock() {
        state.consecutive_failures = 0;
        state.next_retry_at_ms = None;
    }
}

fn reset_openvpn_reconnect(app: &AppHandle, user_requested_stop: bool) {
    if let Ok(mut state) = app.state::<OpenVpnReconnectStore>().state.lock() {
        state.consecutive_failures = 0;
        state.next_retry_at_ms = None;
        state.retry_generation = state.retry_generation.saturating_add(1);
        state.user_requested_stop = user_requested_stop;
    }
}

fn prepare_openvpn_manual_start(app: &AppHandle) {
    reset_openvpn_reconnect(app, false);
}

fn handle_openvpn_terminal_state(app: &AppHandle, state: &str, reason: &str) {
    let reconnect_store = app.state::<OpenVpnReconnectStore>();
    let cfg = match load_app_config_from_disk(app) {
        Ok(cfg) => cfg,
        Err(_) => return,
    };
    let now = now_ms();
    let (generation, delay_secs) = {
        let mut reconnect = match reconnect_store.state.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if reconnect.user_requested_stop || !cfg.openvpn.auto_reconnect_enabled {
            reconnect.next_retry_at_ms = None;
            return;
        }
        reconnect.consecutive_failures = reconnect.consecutive_failures.saturating_add(1);
        // Safety cap: stop retrying after too many consecutive failures to avoid
        // unbounded retry chains when config is permanently broken.
        const MAX_RECONNECT_RETRIES: u32 = 50;
        if reconnect.consecutive_failures > MAX_RECONNECT_RETRIES {
            reconnect.next_retry_at_ms = None;
            drop(reconnect);
            let _ = upsert_function_status(
                app,
                &app.state::<FunctionStore>(),
                OPENVPN_FUNCTION_ID,
                "error",
                &format!(
                    "Auto-reconnect gave up after {MAX_RECONNECT_RETRIES} consecutive failures. {reason}"
                ),
                None,
            );
            return;
        }
        let delay_secs = compute_openvpn_reconnect_delay_secs(
            reconnect.consecutive_failures,
            cfg.openvpn.auto_reconnect_base_seconds,
            cfg.openvpn.auto_reconnect_max_seconds,
        );
        reconnect.next_retry_at_ms = Some(now.saturating_add(delay_secs.saturating_mul(1000)));
        (reconnect.retry_generation, delay_secs)
    };

    let message = if delay_secs == 0 {
        format!("OpenVPN exited unexpectedly ({state}); reconnecting now. {reason}")
    } else {
        format!("OpenVPN exited unexpectedly ({state}); retrying in {delay_secs}s. {reason}")
    };
    let _ = upsert_function_status(
        app,
        &app.state::<FunctionStore>(),
        OPENVPN_FUNCTION_ID,
        "reconnecting",
        &message,
        None,
    );

    let app_clone = app.clone();
    thread::spawn(move || {
        if delay_secs > 0 {
            thread::sleep(Duration::from_secs(delay_secs));
        }
        {
            let reconnect_store = app_clone.state::<OpenVpnReconnectStore>();
            let mut reconnect = match reconnect_store.state.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            if reconnect.retry_generation != generation || reconnect.user_requested_stop {
                return;
            }
            reconnect.next_retry_at_ms = None;
        }
        // Empty command/username/password: start_openvpn_action loads stored
        // config from disk when these are empty, so the reconnect uses the
        // same credentials as the original manual start.
        if let Err(err) = start_openvpn_action(
            String::new(),
            String::new(),
            String::new(),
            app_clone.clone(),
            app_clone.state::<SessionStore>(),
            app_clone.state::<FunctionStore>(),
            app_clone.state::<OpenVpnAdminStore>(),
        ) {
            handle_openvpn_terminal_state(
                &app_clone,
                "error",
                &format!("Reconnect start failed: {err}"),
            );
        }
    });
}

#[cfg(target_os = "windows")]
fn start_openvpn_admin_worker(
    app: &AppHandle,
    function_store: &FunctionStore,
    admin_store: &OpenVpnAdminStore,
    executable_path: String,
    args: String,
    username: String,
    password: String,
    terminate_open_vpn_when_helper_exits: bool,
    auto_reconnect_enabled: bool,
    auto_reconnect_base_seconds: u64,
    auto_reconnect_max_seconds: u64,
) -> Result<String, String> {
    start_openvpn_admin_worker_service(
        app,
        function_store,
        admin_store,
        executable_path,
        args,
        username,
        password,
        terminate_open_vpn_when_helper_exits,
        auto_reconnect_enabled,
        auto_reconnect_base_seconds,
        auto_reconnect_max_seconds,
        OPENVPN_HELPER_SCRIPT_NAME,
        OPENVPN_FUNCTION_ID,
        now_ms,
        is_windows_pid_running,
        mark_openvpn_connected,
        upsert_function_status,
        handle_openvpn_terminal_state,
    )
}

#[cfg(not(target_os = "windows"))]
fn run_openvpn_automation(
    app: AppHandle,
    session_id: String,
    command: String,
    username: String,
    password: String,
    otp_secret: String,
    close_pty: fn(&str, &SessionStore) -> Result<Option<PtyExitPayload>, String>,
    emit_exit: fn(&AppHandle, PtyExitPayload),
    on_session_exit: fn(&AppHandle, &FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
) {
    run_openvpn_automation_service(
        app,
        session_id,
        command,
        username,
        password,
        otp_secret,
        OPENVPN_FUNCTION_ID,
        add_watcher,
        write_pty_internal,
        upsert_function_status,
        generate_openvpn_password,
        mark_openvpn_connected,
        adapt_command_for_shell,
        lock_err,
        close_pty,
        emit_exit,
        on_session_exit,
    )
}

fn default_shell_command() -> CommandBuilder {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = CommandBuilder::new("powershell.exe");
        cmd.arg("-NoLogo");
        cmd.arg("-NoProfile");
        cmd
    }

    #[cfg(not(target_os = "windows"))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut cmd = CommandBuilder::new(shell);
        cmd.arg("-i");
        cmd
    }
}

#[tauri::command]
fn open_pty(
    cols: u16,
    rows: u16,
    app: AppHandle,
    store: State<'_, SessionStore>,
) -> Result<String, String> {
    open_pty_internal(cols, rows, &app, &store)
}

#[tauri::command]
fn write_pty(id: String, data: String, store: State<'_, SessionStore>) -> Result<(), String> {
    write_pty_internal(&id, &data, &store)
}

#[tauri::command]
fn resize_pty(
    id: String,
    cols: u16,
    rows: u16,
    store: State<'_, SessionStore>,
) -> Result<(), String> {
    let session = {
        let sessions = lock_err(&store.sessions)?;
        sessions
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("session not found: {id}"))?
    };

    let master = lock_err(&session.master)?;
    master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("failed to resize pty: {e}"))?;

    Ok(())
}

#[tauri::command]
fn close_pty(
    id: String,
    app: AppHandle,
    store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<(), String> {
    let payload = close_pty_internal(&id, &store)?;
    if let Some(payload) = payload {
        emit_pty_exit(&app, payload.clone());
        let _ = update_functions_for_session_exit(
            &app,
            &function_store,
            &id,
            payload.exit_code,
            &payload.reason,
        );
    }
    Ok(())
}

#[tauri::command]
fn list_function_statuses(
    app: AppHandle,
    function_store: State<'_, FunctionStore>,
) -> Result<Vec<FunctionStatus>, String> {
    let mut values = lock_err(&function_store.statuses)?
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for s in &mut values {
        if s.function_id != BOUNDARY_AUTH_FUNCTION_ID {
            continue;
        }
        if s.session_id.is_some() {
            s.state = "authenticating".to_string();
            s.message =
                "Logging in — finish in the Boundary tab (browser or terminal).".to_string();
            s.boundary_token_source = None;
            continue;
        }
        match boundary_token_summary(&app) {
            Ok(summary) => {
                s.state = "authenticated".to_string();
                s.message = summary.message;
                s.boundary_token_source = Some(summary.source.to_string());
            }
            Err(e) => {
                s.state = "unauthenticated".to_string();
                s.boundary_token_source = None;
                let addr_hint = load_app_config_from_disk(&app)
                    .map(|c| {
                        if c.boundary.addr.trim().is_empty() {
                            " Set Controller URL in Config so the CLI targets the right keyring entry."
                        } else {
                            ""
                        }
                    })
                    .unwrap_or("");
                let err_short = if e.len() > 120 {
                    format!("{}…", &e[..117])
                } else {
                    e
                };
                s.message = format!(
                    "No active Boundary token — open Boundary tab and log in. [{err_short}]{addr_hint}"
                );
            }
        }
    }
    values.sort_by(|a, b| a.function_id.cmp(&b.function_id));
    Ok(values)
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
fn get_app_config(app: AppHandle) -> Result<AppConfig, String> {
    get_app_config_action(app)
}

fn get_config_paths_internal(app: &AppHandle) -> Result<ConfigPaths, String> {
    let path = config_file_path(app, CONFIG_FILE_NAME)?;
    Ok(ConfigPaths {
        app_config_path: path.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
fn get_config_paths(app: AppHandle) -> Result<ConfigPaths, String> {
    get_config_paths_internal(&app)
}

#[tauri::command]
fn get_sync_settings(app: AppHandle) -> Result<SyncSettings, String> {
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
    save_sync_settings_to_disk(&app, &merged, SYNC_SETTINGS_FILE_NAME)?;
    let _ = sync_mcp_runtime_from_settings(&app, &mcp_state, &merged, now_ms);
    Ok(merged)
}

#[tauri::command]
fn save_sync_settings(
    app: AppHandle,
    settings: SyncSettingsPatch,
    mcp_state: State<'_, McpRuntimeState>,
) -> Result<(), String> {
    save_sync_settings_action(app, settings, mcp_state).map(|_| ())
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
fn sync_config_pull(app: AppHandle, repo_root: String) -> Result<(), String> {
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
fn sync_config_push(app: AppHandle, repo_root: String) -> Result<(), String> {
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
        // Avoid hanging on credential prompts in a GUI app.
        .env("GIT_TERMINAL_PROMPT", "0")
        // On packaged Linux builds, inherited loader paths can point git and
        // git-remote-https at the app-bundled libcurl/libnghttp2 instead of
        // the system-matched pair, causing symbol lookup failures.
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
fn config_git_pull(repo_root: String) -> Result<String, String> {
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
fn config_git_push(repo_root: String) -> Result<String, String> {
    config_git_push_action(repo_root)
}

pub(crate) fn save_app_config_action(
    app: AppHandle,
    mut config: AppConfig,
) -> Result<AppConfig, String> {
    let _incoming_pw_len = config.openvpn.password.len();
    let _incoming_otp_len = config.openvpn.otp_secret.len();
    // Never wipe keyring when the UI sends empty secrets (WebViews often omit password field values).
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
fn save_app_config(app: AppHandle, config: AppConfig) -> Result<AppConfig, String> {
    save_app_config_action(app, config)
}

#[tauri::command]
fn detect_openvpn_executable() -> Result<String, String> {
    detect_openvpn_executable_path()
        .ok_or_else(|| "OpenVPN executable not found in PATH/common locations".to_string())
}

pub(crate) fn start_openvpn_action(
    _command: String,
    username: String,
    password: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    _admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<StartFunctionResult, String> {
    scp_debug!("start_openvpn_function: invoked");
    let saved_config = load_app_config_from_disk(&app)?;
    let (resolved_openvpn, resolved_username, combined_password) = match prepare_openvpn_start(
        &saved_config,
        username,
        password,
        normalized_openvpn_config,
        generate_openvpn_password,
    ) {
        Ok(values) => values,
        Err(e) => {
            scp_debug!("start_openvpn_function: ERR prepare_openvpn_start: {e}");
            return Err(e);
        }
    };

    scp_debug!(
        "start_openvpn_function: after load — exe_nonempty={} ovpn_nonempty={} user_nonempty={} password_nonempty={}",
        !resolved_openvpn.executable_path.trim().is_empty(),
        !resolved_openvpn.ovpn_path.trim().is_empty(),
        !resolved_username.trim().is_empty(),
        !combined_password.is_empty()
    );

    #[cfg(target_os = "windows")]
    {
        let _ = &session_store;
        let openvpn_args = match build_openvpn_args_windows(&resolved_openvpn) {
            Ok(a) => a,
            Err(e) => {
                scp_debug!("start_openvpn_function: ERR build_openvpn_args_windows: {e}");
                return Err(e);
            }
        };
        scp_debug!(
            "start_openvpn_function: Windows path — launching elevated helper; exe={}",
            resolved_openvpn.executable_path
        );
        if let Some(active) = get_openvpn_admin_session(&_admin_store)? {
            let _ = fs::write(&active.stop_path, "stop");
        }
        let session_id = start_openvpn_admin_worker(
            &app,
            &function_store,
            &_admin_store,
            resolved_openvpn.executable_path.clone(),
            openvpn_args,
            resolved_username,
            combined_password,
            resolved_openvpn.terminate_open_vpn_when_helper_exits,
            saved_config.openvpn.auto_reconnect_enabled,
            saved_config.openvpn.auto_reconnect_base_seconds,
            saved_config.openvpn.auto_reconnect_max_seconds,
        )?;
        scp_debug!("start_openvpn_function: Windows helper session_id={session_id}");

        return Ok(StartFunctionResult {
            function_id: OPENVPN_FUNCTION_ID.to_string(),
            session_id,
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let session_id = start_openvpn_pty_session_service(
            &app,
            &session_store,
            &function_store,
            &saved_config,
            resolved_openvpn,
            resolved_username,
            combined_password,
            OPENVPN_FUNCTION_ID,
            lock_err,
            close_pty_internal,
            upsert_function_status,
            open_pty_internal,
            build_openvpn_command_unix,
            run_openvpn_automation,
            emit_pty_exit,
            update_functions_for_session_exit,
        )?;

        return Ok(StartFunctionResult {
            function_id: OPENVPN_FUNCTION_ID.to_string(),
            session_id,
        });
    }
}

pub(crate) fn start_openvpn_manual_action(
    command: String,
    username: String,
    password: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<StartFunctionResult, String> {
    prepare_openvpn_manual_start(&app);
    start_openvpn_action(
        command,
        username,
        password,
        app,
        session_store,
        function_store,
        admin_store,
    )
}

#[tauri::command]
fn start_openvpn_function(
    command: String,
    username: String,
    password: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<StartFunctionResult, String> {
    start_openvpn_manual_action(
        command,
        username,
        password,
        app,
        session_store,
        function_store,
        admin_store,
    )
}

pub(crate) fn stop_openvpn_action(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    _admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<(), String> {
    reset_openvpn_reconnect(&app, true);
    #[cfg(target_os = "windows")]
    {
        let _ = &session_store;
        let active = get_openvpn_admin_session(&_admin_store)?;
        if let Some(active) = active {
            let helper_pid_path =
                openvpn_runtime_dir(&app)?.join(format!("{}.helper.pid", active.id));
            if let Ok(s) = fs::read_to_string(&helper_pid_path) {
                if let Ok(pid) = s.trim().parse::<u32>() {
                    if pid > 0 && !is_windows_pid_running(pid) {
                        let _ = fs::remove_file(&helper_pid_path);
                        upsert_function_status(
                            &app,
                            &function_store,
                            OPENVPN_FUNCTION_ID,
                            "stopped",
                            "Helper already exited",
                            None,
                        )?;
                        set_openvpn_admin_session(&_admin_store, None)?;
                        return Ok(());
                    }
                }
            }
            upsert_function_status(
                &app,
                &function_store,
                OPENVPN_FUNCTION_ID,
                "stopping",
                "Requesting elevated OpenVPN stop",
                Some(active.id.clone()),
            )?;
            fs::write(&active.stop_path, "stop")
                .map_err(|e| format!("failed to signal openvpn stop: {e}"))?;
        } else {
            upsert_function_status(
                &app,
                &function_store,
                OPENVPN_FUNCTION_ID,
                "idle",
                "No running admin session",
                None,
            )?;
        }

        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        return stop_openvpn_pty_session_service(
            &app,
            &session_store,
            &function_store,
            OPENVPN_FUNCTION_ID,
            close_pty_internal,
            emit_pty_exit,
            update_functions_for_session_exit,
            upsert_function_status,
            lock_err,
        );
    }
}

#[tauri::command]
fn stop_openvpn_function(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<(), String> {
    stop_openvpn_action(app, session_store, function_store, admin_store)
}

pub(crate) fn restart_openvpn_action(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let admin_store = app.state::<OpenVpnAdminStore>();
        if let Some(active) = get_openvpn_admin_session(&admin_store)? {
            let _ = fs::remove_file(&active.stop_path);
            match fs::write(&active.control_path, "restart") {
                Ok(_) => {
                    let function_store = app.state::<FunctionStore>();
                    upsert_function_status(
                        &app,
                        &function_store,
                        OPENVPN_FUNCTION_ID,
                        "restarting",
                        "Requesting elevated OpenVPN child restart",
                        Some(active.id.clone()),
                    )?;
                    return Ok(());
                }
                Err(err) => {
                    eprintln!(
                        "CC OpenVPN restart: helper control signal failed for session {}: {}",
                        active.id, err
                    );
                }
            }
        }
    }

    let is_running = {
        let function_store = app.state::<FunctionStore>();
        let running = lock_err(&function_store.statuses)?
            .get(OPENVPN_FUNCTION_ID)
            .and_then(|status| status.session_id.clone())
            .is_some();
        running
    };

    if is_running {
        let session_store = app.state::<SessionStore>();
        let function_store = app.state::<FunctionStore>();
        let admin_store = app.state::<OpenVpnAdminStore>();

        stop_openvpn_action(
            app.clone(),
            session_store,
            function_store,
            admin_store,
        )?;
    } else {
        let function_store = app.state::<FunctionStore>();
        upsert_function_status(
            &app,
            &function_store,
            OPENVPN_FUNCTION_ID,
            "starting",
            "CC restart requested; OpenVPN is idle, starting directly",
            None,
        )?;

        let session_store = app.state::<SessionStore>();
        let function_store = app.state::<FunctionStore>();
        let admin_store = app.state::<OpenVpnAdminStore>();

        return start_openvpn_manual_action(
            String::new(),
            String::new(),
            String::new(),
            app.clone(),
            session_store,
            function_store,
            admin_store,
        )
        .map(|_| ());
    }

    let restart_app = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(2));

        let session_store = restart_app.state::<SessionStore>();
        let function_store = restart_app.state::<FunctionStore>();
        let admin_store = restart_app.state::<OpenVpnAdminStore>();

        if let Err(err) = start_openvpn_manual_action(
            String::new(),
            String::new(),
            String::new(),
            restart_app.clone(),
            session_store,
            function_store,
            admin_store,
        ) {
            let function_store = restart_app.state::<FunctionStore>();
            let _ = upsert_function_status(
                &restart_app,
                &function_store,
                OPENVPN_FUNCTION_ID,
                "error",
                &format!("CC restart failed: {err}"),
                None,
            );
        }
    });

    Ok(())
}

pub(crate) fn start_caddy_action(
    cols: u16,
    rows: u16,
    folder: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<StartFunctionResult, String> {
    start_caddy_run_service(
        cols,
        rows,
        folder,
        &app,
        &session_store,
        &function_store,
        CADDY_FUNCTION_ID,
        lock_err,
        close_pty_internal,
        emit_pty_exit,
        update_functions_for_session_exit,
        open_pty_with_command_builder,
        upsert_function_status,
    )
}

#[tauri::command]
fn start_caddy_run(
    cols: u16,
    rows: u16,
    folder: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<StartFunctionResult, String> {
    start_caddy_action(cols, rows, folder, app, session_store, function_store)
}

pub(crate) fn stop_caddy_action(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<(), String> {
    stop_caddy_run_service(
        &app,
        &session_store,
        &function_store,
        CADDY_FUNCTION_ID,
        lock_err,
        close_pty_internal,
        emit_pty_exit,
        upsert_function_status,
    )
}

#[tauri::command]
fn list_servers(app: AppHandle) -> Result<Vec<Server>, String> {
    load_servers(&app)
}
fn close_remote_browser_sessions_for_server(
    server_id: &str,
    browser_store: &RemoteBrowserStore,
) -> Result<usize, String> {
    close_remote_browser_sessions_for_server_service(server_id, browser_store)
}
pub(crate) fn upsert_server_action(
    app: AppHandle,
    mut server: Server,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<Server>, String> {
    let mut list = load_servers(&app)?;
    if server.id.is_empty() {
        server.id = new_server_id();
        list.push(server);
    } else if let Some(i) = list.iter().position(|s| s.id == server.id) {
        close_remote_browser_sessions_for_server(&server.id, &browser_store)?;
        list[i] = server;
    } else {
        list.push(server);
    }
    save_servers(&app, &list)?;
    Ok(list)
}

#[tauri::command]
fn upsert_server(
    app: AppHandle,
    server: Server,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<Server>, String> {
    upsert_server_action(app, server, browser_store)
}

pub(crate) fn delete_server_action(
    app: AppHandle,
    id: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<Server>, String> {
    close_remote_browser_sessions_for_server(&id, &browser_store)?;
    let mut list = load_servers(&app)?;
    list.retain(|s| s.id != id);
    save_servers(&app, &list)?;
    Ok(list)
}

#[tauri::command]
fn delete_server(
    app: AppHandle,
    id: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<Server>, String> {
    delete_server_action(app, id, browser_store)
}

/// Search Boundary targets (uses CLI token). Optional text filter; empty = list all.
#[tauri::command]
fn search_boundary_targets(
    app: AppHandle,
    filter: Option<String>,
) -> Result<Vec<BoundaryTargetSummary>, String> {
    let cfg = load_app_config_from_disk(&app)?;
    let exe = boundary_cli_path(&cfg);
    if exe != "boundary" && !PathBuf::from(&exe).exists() {
        return Err("Boundary CLI not found in config".to_string());
    }
    let mut cmd = Command::new(&exe);
    cmd.arg("targets");
    cmd.arg("list");
    apply_boundary_token_arg_if_present(&mut cmd)?;
    cmd.arg("-recursive");
    cmd.arg("-format");
    cmd.arg("json");
    apply_boundary_env(&mut cmd, &cfg)?;
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd
        .output()
        .map_err(|e| format!("boundary targets list failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "boundary targets list failed (check Boundary auth in Status)".to_string()
        } else {
            stderr
        });
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("invalid JSON: {e}"))?;
    let items = v
        .get("items")
        .and_then(|a| a.as_array())
        .ok_or_else(|| "expected items array in boundary output".to_string())?;
    let filter_lower = filter
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let mut results: Vec<BoundaryTargetSummary> = Vec::new();
    for item in items {
        let obj = match item.as_object() {
            Some(o) => o,
            None => continue,
        };
        let id = obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let target_type = obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let description = obj
            .get("description")
            .or_else(|| obj.get("attributes").and_then(|a| a.get("description")))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let address = obj
            .get("address")
            .or_else(|| obj.get("attributes").and_then(|a| a.get("address")))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if let Some(ref f) = filter_lower {
            if !name.to_lowercase().contains(f)
                && !id.to_lowercase().contains(f)
                && !description.to_lowercase().contains(f)
            {
                continue;
            }
        }
        results.push(BoundaryTargetSummary {
            id,
            name,
            target_type,
            description,
            address,
        });
    }
    Ok(results)
}

#[tauri::command]
fn stop_caddy_run(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<(), String> {
    stop_caddy_action(app, session_store, function_store)
}

pub(crate) fn start_server_tunnel_action(
    server_id: String,
    cols: u16,
    rows: u16,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<StartFunctionResult, String> {
    start_server_tunnel_service(
        server_id,
        cols,
        rows,
        &app,
        &session_store,
        &function_store,
        &browser_store,
        close_remote_browser_sessions_for_server,
        close_pty_internal,
        emit_pty_exit,
        update_functions_for_session_exit,
        load_app_config_from_disk,
        boundary_cli_path,
        apply_boundary_token_arg_if_present_builder,
        apply_boundary_env_builder,
        open_pty_with_command_builder,
        upsert_function_status,
        server_tunnel_function_id,
    )
}

#[tauri::command]
fn start_server_tunnel(
    server_id: String,
    cols: u16,
    rows: u16,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<StartFunctionResult, String> {
    start_server_tunnel_action(
        server_id,
        cols,
        rows,
        app,
        session_store,
        function_store,
        browser_store,
    )
}

pub(crate) fn stop_server_tunnel_action(
    server_id: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    stop_server_tunnel_service(
        server_id,
        &app,
        &session_store,
        &function_store,
        &browser_store,
        close_remote_browser_sessions_for_server,
        close_pty_internal,
        emit_pty_exit,
        upsert_function_status,
        server_tunnel_function_id,
    )
}

#[tauri::command]
fn stop_server_tunnel(
    server_id: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    stop_server_tunnel_action(server_id, app, session_store, function_store, browser_store)
}

pub(crate) fn start_server_ssh_action(
    server_id: String,
    cols: u16,
    rows: u16,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<StartFunctionResult, String> {
    start_server_ssh_service(
        server_id,
        cols,
        rows,
        &app,
        &session_store,
        &function_store,
        close_pty_internal,
        emit_pty_exit,
        update_functions_for_session_exit,
        open_pty_with_command_builder,
        upsert_function_status,
        server_ssh_function_id,
    )
}

#[tauri::command]
fn start_server_ssh(
    server_id: String,
    cols: u16,
    rows: u16,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<StartFunctionResult, String> {
    start_server_ssh_action(server_id, cols, rows, app, session_store, function_store)
}

pub(crate) fn stop_server_ssh_action(
    server_id: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<(), String> {
    stop_server_ssh_service(
        server_id,
        &app,
        &session_store,
        &function_store,
        close_pty_internal,
        emit_pty_exit,
        upsert_function_status,
        server_ssh_function_id,
    )
}

pub(crate) fn open_remote_browser_action(
    app: AppHandle,
    server_id: String,
    browser_store: State<'_, RemoteBrowserStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<RemoteBrowseSession, String> {
    #[cfg(target_os = "windows")]
    {
        open_remote_browser_service(
            &app,
            server_id,
            &browser_store,
            &function_store,
            server_tunnel_function_id,
            CREATE_NO_WINDOW,
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        open_remote_browser_service(
            &app,
            server_id,
            &browser_store,
            &function_store,
            server_tunnel_function_id,
        )
    }
}

#[tauri::command]
fn open_remote_browser(
    app: AppHandle,
    server_id: String,
    browser_store: State<'_, RemoteBrowserStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<RemoteBrowseSession, String> {
    open_remote_browser_action(app, server_id, browser_store, function_store)
}

pub(crate) fn close_remote_browser_action(
    browser_id: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    close_remote_browser_service(browser_id, &browser_store)
}

#[tauri::command]
fn close_remote_browser(
    browser_id: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    close_remote_browser_action(browser_id, browser_store)
}

pub(crate) fn list_remote_browser_children_action(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<RemoteBrowseEntry>, String> {
    list_remote_browser_children_service(browser_id, path, &browser_store)
}

#[tauri::command]
fn list_remote_browser_children(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<RemoteBrowseEntry>, String> {
    list_remote_browser_children_action(browser_id, path, browser_store)
}

pub(crate) fn stat_remote_browser_path_action(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<RemoteBrowseNodeInfo, String> {
    stat_remote_browser_path_service(browser_id, path, &browser_store)
}

#[tauri::command]
fn stat_remote_browser_path(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<RemoteBrowseNodeInfo, String> {
    stat_remote_browser_path_action(browser_id, path, browser_store)
}

pub(crate) fn create_remote_browser_directory_action(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    create_remote_browser_directory_service(browser_id, path, &browser_store)
}

#[tauri::command]
fn create_remote_browser_directory(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    create_remote_browser_directory_action(browser_id, path, browser_store)
}

pub(crate) fn delete_remote_browser_path_action(
    browser_id: String,
    path: String,
    recursive: Option<bool>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    delete_remote_browser_path_service(browser_id, path, recursive, &browser_store)
}

#[tauri::command]
fn delete_remote_browser_path(
    browser_id: String,
    path: String,
    recursive: Option<bool>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    delete_remote_browser_path_action(browser_id, path, recursive, browser_store)
}

pub(crate) fn scp_upload_action(
    app: AppHandle,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    let transfer_id = create_scp_transfer_entry(
        &transfer_store,
        now_ms,
        "Upload",
        "Uploading…",
        Some(local_path.clone()),
        Some(remote_path.clone()),
    )?;
    match scp_upload_service(&app, server_id, local_path, remote_path, recursive) {
        Ok(message) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Success,
                message.clone(),
                Some(100),
                now_ms,
            )?;
            Ok(message)
        }
        Err(err) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Error,
                err.clone(),
                None,
                now_ms,
            )?;
            Err(err)
        }
    }
}

#[tauri::command]
fn scp_upload(
    app: AppHandle,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    scp_upload_action(
        app,
        server_id,
        local_path,
        remote_path,
        recursive,
        transfer_store,
    )
}

pub(crate) fn start_scp_upload_action(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    let transfer_id = create_scp_transfer_entry(
        &transfer_store,
        now_ms,
        "Upload",
        "Uploading…",
        Some(local_path.clone()),
        Some(remote_path.clone()),
    )?;
    let result = start_scp_upload_service(
        &app,
        &session_store,
        server_id,
        local_path,
        remote_path,
        recursive,
        open_pty_with_command_builder,
        update_functions_for_session_exit,
    );
    match result {
        Ok(session_id) => {
            attach_scp_transfer_session(&transfer_store, &transfer_id, &session_id)?;
            Ok(session_id)
        }
        Err(err) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Error,
                err.clone(),
                None,
                now_ms,
            )?;
            Err(err)
        }
    }
}

#[tauri::command]
fn start_scp_upload(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    start_scp_upload_action(
        app,
        session_store,
        server_id,
        local_path,
        remote_path,
        recursive,
        transfer_store,
    )
}

pub(crate) fn scp_download_action(
    app: AppHandle,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    let transfer_id = create_scp_transfer_entry(
        &transfer_store,
        now_ms,
        "Download",
        "Downloading…",
        Some(remote_path.clone()),
        Some(local_path.clone()),
    )?;
    match scp_download_service(&app, server_id, remote_path, local_path, recursive) {
        Ok(message) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Success,
                message.clone(),
                Some(100),
                now_ms,
            )?;
            Ok(message)
        }
        Err(err) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Error,
                err.clone(),
                None,
                now_ms,
            )?;
            Err(err)
        }
    }
}

#[tauri::command]
fn scp_download(
    app: AppHandle,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    scp_download_action(
        app,
        server_id,
        remote_path,
        local_path,
        recursive,
        transfer_store,
    )
}

pub(crate) fn start_scp_download_action(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    let transfer_id = create_scp_transfer_entry(
        &transfer_store,
        now_ms,
        "Download",
        "Downloading…",
        Some(remote_path.clone()),
        Some(local_path.clone()),
    )?;
    let result = start_scp_download_service(
        &app,
        &session_store,
        server_id,
        remote_path,
        local_path,
        recursive,
        open_pty_with_command_builder,
        update_functions_for_session_exit,
    );
    match result {
        Ok(session_id) => {
            attach_scp_transfer_session(&transfer_store, &transfer_id, &session_id)?;
            Ok(session_id)
        }
        Err(err) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Error,
                err.clone(),
                None,
                now_ms,
            )?;
            Err(err)
        }
    }
}

#[tauri::command]
fn start_scp_download(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    start_scp_download_action(
        app,
        session_store,
        server_id,
        remote_path,
        local_path,
        recursive,
        transfer_store,
    )
}

#[tauri::command]
fn start_server_rdp(app: AppHandle, server_id: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        start_server_rdp_service(&app, server_id, CREATE_NO_WINDOW)
    }
    #[cfg(not(target_os = "windows"))]
    {
        start_server_rdp_service(&app, server_id)
    }
}

fn stop_boundary_automation_session(
    app: &AppHandle,
    automation_store: &BoundaryAutomationStore,
    session_store: &SessionStore,
    function_store: &FunctionStore,
) -> Result<(), String> {
    stop_boundary_automation_session_service(
        app,
        automation_store,
        session_store,
        function_store,
        BOUNDARY_OIDC_WINDOW_LABEL,
        close_pty_internal,
        emit_pty_exit,
        update_functions_for_session_exit,
    )
}

fn complete_boundary_automation_session(
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    session_id: &str,
    exit_code: Option<i32>,
    reason: &str,
) {
    complete_boundary_automation_session_service(
        app,
        session_store,
        function_store,
        session_id,
        exit_code,
        reason,
        close_pty_internal,
        emit_pty_exit,
        update_functions_for_session_exit,
    )
}

fn emit_pty_output_line_owned(app: &AppHandle, session_id: &str, data: String) {
    emit_pty_output_line(app, session_id, data);
}

fn run_boundary_oidc_automation(
    app: AppHandle,
    session_id: String,
    cfg: BoundaryConfig,
    cancel: Arc<AtomicBool>,
) {
    run_boundary_oidc_automation_service(
        app,
        session_id,
        cfg,
        cancel,
        BOUNDARY_OIDC_WINDOW_LABEL,
        KEYRING_BOUNDARY_APP_TOKEN,
        boundary_addr_base,
        emit_pty_output_line_owned,
        boundary_http_client,
        boundary_oidc_init_script,
        keyring_set_or_delete,
        complete_boundary_automation_session,
    )
}

#[tauri::command]
fn stop_server_ssh(
    server_id: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<(), String> {
    stop_server_ssh_action(server_id, app, session_store, function_store)
}

pub(crate) fn start_boundary_auth_action(
    cols: u16,
    rows: u16,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    automation_store: State<'_, BoundaryAutomationStore>,
) -> Result<StartFunctionResult, String> {
    let cfg = load_app_config_from_disk(&app)?;
    let plan = prepare_boundary_auth_start(
        &cfg,
        apply_boundary_token_arg_if_present_builder,
        apply_boundary_env_builder,
    )?;
    keyring_set_or_delete(KEYRING_BOUNDARY_APP_TOKEN, "")?;
    stop_boundary_automation_session(&app, &automation_store, &session_store, &function_store)?;
    if let Some(prev) = lock_err(&function_store.statuses)?
        .get(BOUNDARY_AUTH_FUNCTION_ID)
        .and_then(|s| s.session_id.clone())
    {
        let _ = close_pty_internal(&prev, &session_store);
    }
    let is_oidc_automation = matches!(&plan.launch, BoundaryAuthLaunch::AutomatedOidc);
    let session_id = match plan.launch {
        BoundaryAuthLaunch::AutomatedOidc => open_pty_with_command_builder(
            cols,
            rows,
            passive_pty_command(),
            &app,
            &session_store,
            update_functions_for_session_exit,
        )?,
        BoundaryAuthLaunch::Cli(cmd) => open_pty_with_command_builder(
            cols,
            rows,
            cmd,
            &app,
            &session_store,
            update_functions_for_session_exit,
        )?,
    };
    upsert_function_status(
        &app,
        &function_store,
        BOUNDARY_AUTH_FUNCTION_ID,
        "authenticating",
        &plan.status_label,
        Some(session_id.clone()),
    )?;
    if is_oidc_automation {
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut guard = lock_err(&automation_store.session)?;
            *guard = Some(BoundaryAutomationSession {
                session_id: session_id.clone(),
                cancel: cancel.clone(),
            });
        }
        let app2 = app.clone();
        let cfg2 = cfg.boundary.clone();
        let sid = session_id.clone();
        thread::spawn(move || run_boundary_oidc_automation(app2, sid, cfg2, cancel));
    } else if let Some(pass) = plan.password_to_send {
        let sid = session_id.clone();
        let app2 = app.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(1200));
            let st = app2.state::<SessionStore>();
            let _ = write_pty_internal(&sid, &format!("{pass}\r"), &st);
        });
    }
    Ok(StartFunctionResult {
        function_id: BOUNDARY_AUTH_FUNCTION_ID.to_string(),
        session_id,
    })
}

#[tauri::command]
fn start_boundary_auth(
    cols: u16,
    rows: u16,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    automation_store: State<'_, BoundaryAutomationStore>,
) -> Result<StartFunctionResult, String> {
    start_boundary_auth_action(
        cols,
        rows,
        app,
        session_store,
        function_store,
        automation_store,
    )
}

pub(crate) fn stop_boundary_auth_action(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    automation_store: State<'_, BoundaryAutomationStore>,
) -> Result<(), String> {
    stop_boundary_auth_service(
        &app,
        &session_store,
        &function_store,
        &automation_store,
        BOUNDARY_AUTH_FUNCTION_ID,
        BOUNDARY_OIDC_WINDOW_LABEL,
        close_pty_internal,
        emit_pty_exit,
        update_functions_for_session_exit,
        boundary_token_summary,
        upsert_boundary_status,
    )
}

#[tauri::command]
fn stop_boundary_auth(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    automation_store: State<'_, BoundaryAutomationStore>,
) -> Result<(), String> {
    stop_boundary_auth_action(app, session_store, function_store, automation_store)
}

#[tauri::command]
fn mcp_get_runtime_status(
    mcp_state: State<'_, McpRuntimeState>,
) -> Result<McpRuntimeStatus, String> {
    mcp_runtime_status(&mcp_state)
}

#[tauri::command]
fn mcp_sync_runtime(
    app: AppHandle,
    mcp_state: State<'_, McpRuntimeState>,
) -> Result<McpRuntimeStatus, String> {
    let settings = load_sync_settings_from_disk(&app, SYNC_SETTINGS_FILE_NAME)?;
    sync_mcp_runtime_from_settings(&app, &mcp_state, &settings, now_ms)
}

#[tauri::command]
fn mcp_stop_runtime(mcp_state: State<'_, McpRuntimeState>) -> Result<McpRuntimeStatus, String> {
    stop_mcp_runtime(&mcp_state)
}

#[tauri::command]
fn mcp_list_tools_command() -> Result<Vec<McpToolDescriptor>, String> {
    Ok(list_mcp_tools())
}

#[tauri::command]
fn mcp_list_resources_command() -> Result<Vec<McpResourceDescriptor>, String> {
    Ok(list_mcp_resources())
}

#[tauri::command]
fn mcp_call_tool_command(
    app: AppHandle,
    tool_name: String,
    args: Value,
) -> Result<McpToolCallEnvelope, String> {
    call_mcp_tool(
        &app,
        &tool_name,
        args,
        now_ms,
        OPENVPN_FUNCTION_ID,
        BOUNDARY_AUTH_FUNCTION_ID,
        get_config_paths_internal,
    )
}

#[tauri::command]
fn mcp_read_resource_command(app: AppHandle, uri: String) -> Result<McpToolCallEnvelope, String> {
    read_mcp_resource(
        &app,
        &uri,
        now_ms,
        OPENVPN_FUNCTION_ID,
        BOUNDARY_AUTH_FUNCTION_ID,
        get_config_paths_internal,
    )
}

fn main() {
    tauri::Builder::default()
        .manage(SessionStore::default())
        .manage(FunctionStore::default())
        .manage(OpenVpnAdminStore::default())
        .manage(OpenVpnReconnectStore::default())
        .manage(BoundaryAutomationStore::default())
        .manage(RemoteBrowserStore::default())
        .manage(ScpTransferStore::default())
        .manage(McpRuntimeState::default())
        .setup(|app| {
            let state = app.state::<McpRuntimeState>();
            let settings = load_sync_settings_from_disk(app.handle(), SYNC_SETTINGS_FILE_NAME)?;
            let _ = sync_mcp_runtime_from_settings(app.handle(), &state, &settings, now_ms);
            if settings.cc_enabled {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = init_cc_connection(app_handle).await {
                        eprintln!("CC startup init failed: {err}");
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_pty,
            write_pty,
            resize_pty,
            close_pty,
            list_function_statuses,
            get_app_config,
            get_config_paths,
            mcp_get_runtime_status,
            mcp_sync_runtime,
            mcp_stop_runtime,
            mcp_list_tools_command,
            mcp_list_resources_command,
            mcp_call_tool_command,
            mcp_read_resource_command,
            get_sync_settings,
            save_sync_settings,
            init_cc_connection,
            get_cc_status,
            save_app_config,
            detect_openvpn_executable,
            start_openvpn_function,
            stop_openvpn_function,
            start_caddy_run,
            stop_caddy_run,
            list_servers,
            upsert_server,
            delete_server,
            search_boundary_targets,
            start_server_tunnel,
            stop_server_tunnel,
            start_server_ssh,
            stop_server_ssh,
            open_remote_browser,
            close_remote_browser,
            list_remote_browser_children,
            stat_remote_browser_path,
            create_remote_browser_directory,
            delete_remote_browser_path,
            start_scp_upload,
            scp_upload,
            start_scp_download,
            scp_download,
            start_server_rdp,
            start_boundary_auth,
            stop_boundary_auth,
            sync_config_pull,
            sync_config_push,
            config_git_pull,
            config_git_push
        ])
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
