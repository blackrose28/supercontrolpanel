#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

mod cc;
use cc::{get_cc_status, init_cc_connection, sync_cc_status_from_settings};
use tauri::Manager;

mod app_state;
mod mcp;
mod models;
mod server_mgmt;
mod services;
mod stores;
use mcp::{sync_mcp_runtime_from_settings, McpRuntimeState};
use services::{load_sync_settings_from_disk, now_ms};
use stores::{
    BoundaryAutomationStore, CcRuntimeState, FunctionStore, OpenVpnAdminStore,
    OpenVpnReconnectStore, RemoteBrowserStore, RemotePtyStore, ScpTransferStore, SessionStore,
};

pub(crate) const OPENVPN_FUNCTION_ID: &str = "openvpn-connect";
pub(crate) const CADDY_FUNCTION_ID: &str = "caddy-run";
pub(crate) const CONFIG_FILE_NAME: &str = "config.json";
pub(crate) const SERVERS_FILE_NAME: &str = "servers.json";
pub(crate) const SYNC_SETTINGS_FILE_NAME: &str = "sync-local.json";
#[cfg(target_os = "windows")]
pub(crate) const OPENVPN_HELPER_SCRIPT_NAME: &str = "openvpn-admin-helper.ps1";
pub(crate) const BOUNDARY_AUTH_FUNCTION_ID: &str = "boundary-auth";
pub(crate) const CC_FUNCTION_ID: &str = "cc-connection";
pub(crate) const BOUNDARY_OIDC_WINDOW_LABEL: &str = "boundary-oidc-automation";

pub(crate) fn server_tunnel_function_id(server_id: &str) -> String {
    format!("server-tunnel-{server_id}")
}
pub(crate) fn server_ssh_function_id(server_id: &str) -> String {
    format!("server-ssh-{server_id}")
}
#[cfg(target_os = "windows")]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

mod commands;
use commands::{
    close_pty, close_remote_browser, config_git_pull, config_git_push,
    create_remote_browser_directory, delete_remote_browser_path, delete_server,
    detect_openvpn_executable, enroll_device, get_app_config, get_config_paths, get_sync_settings,
    is_first_run, list_function_statuses, list_remote_browser_children, list_servers, mcp_call_tool_command,
    mcp_get_runtime_status, mcp_list_resources_command, mcp_list_tools_command,
    mcp_read_resource_command, mcp_stop_runtime, mcp_sync_runtime, open_pty, open_remote_browser,
    resize_pty, save_app_config, save_sync_settings, scp_download, scp_upload,
    search_boundary_targets, start_boundary_auth, start_caddy_run, start_openvpn_function,
    start_scp_download, start_scp_upload, start_server_rdp, start_server_ssh, start_server_tunnel,
    stat_remote_browser_path, stop_boundary_auth, stop_caddy_run, stop_openvpn_function,
    stop_server_ssh, stop_server_tunnel, sync_config_pull, sync_config_push, upsert_server,
    write_pty, revoke_my_device,
};

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

fn main() {
    tauri::Builder::default()
        .manage(SessionStore::default())
        .manage(FunctionStore::default())
        .manage(OpenVpnAdminStore::default())
        .manage(OpenVpnReconnectStore::default())
        .manage(BoundaryAutomationStore::default())
        .manage(RemoteBrowserStore::default())
        .manage(RemotePtyStore::default())
        .manage(ScpTransferStore::default())
        .manage(McpRuntimeState::default())
        .manage(CcRuntimeState::default())
        .setup(|app| {
            let state = app.state::<McpRuntimeState>();
            let settings = load_sync_settings_from_disk(app.handle(), SYNC_SETTINGS_FILE_NAME)?;
            let _ = sync_mcp_runtime_from_settings(app.handle(), &state, &settings, now_ms);
            let _ = sync_cc_status_from_settings(app.handle());
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
            enroll_device,
            is_first_run,
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
            config_git_push,
            revoke_my_device,
        ])
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
