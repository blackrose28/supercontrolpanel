use std::process::Command;

use tauri::AppHandle;

use crate::{
    services::{
        add_common_ssh_options, build_scp_command, load_server_by_id, resolve_server_ssh_endpoint,
        ssh_target,
    },
    stores::SessionStore,
};

pub fn scp_upload_service(
    app: &AppHandle,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
) -> Result<String, String> {
    let server = load_server_by_id(app, &server_id)?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let remote_spec = format!("{}:{}", ssh_target(&endpoint), remote_path);
    let mut scp = Command::new("scp");
    add_common_ssh_options(&mut scp, "-P", endpoint.port);
    if recursive.unwrap_or(false) {
        scp.arg("-r");
    }
    let o = scp
        .arg(&local_path)
        .arg(&remote_spec)
        .output()
        .map_err(|e| format!("scp failed to run: {e}"))?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok("Upload finished.".into())
}

pub fn start_scp_upload_service(
    app: &AppHandle,
    session_store: &SessionStore,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    open_pty_with_command_builder: fn(u16, u16, portable_pty::CommandBuilder, &AppHandle, &SessionStore, fn(&AppHandle, &crate::stores::FunctionStore, &str, Option<i32>, &str) -> Result<(), String>) -> Result<String, String>,
    update_functions_for_session_exit: fn(&AppHandle, &crate::stores::FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
) -> Result<String, String> {
    let server = load_server_by_id(app, &server_id)?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let cmd = build_scp_command(
        &endpoint,
        &local_path,
        &remote_path,
        recursive.unwrap_or(false),
        false,
    );
    open_pty_with_command_builder(120, 24, cmd, app, session_store, update_functions_for_session_exit)
}

pub fn scp_download_service(
    app: &AppHandle,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
) -> Result<String, String> {
    let server = load_server_by_id(app, &server_id)?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let remote_spec = format!("{}:{}", ssh_target(&endpoint), remote_path);
    let mut scp = Command::new("scp");
    add_common_ssh_options(&mut scp, "-P", endpoint.port);
    if recursive.unwrap_or(false) {
        scp.arg("-r");
    }
    let o = scp
        .arg(&remote_spec)
        .arg(&local_path)
        .output()
        .map_err(|e| format!("scp failed to run: {e}"))?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok("Download finished.".into())
}

pub fn start_scp_download_service(
    app: &AppHandle,
    session_store: &SessionStore,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    open_pty_with_command_builder: fn(u16, u16, portable_pty::CommandBuilder, &AppHandle, &SessionStore, fn(&AppHandle, &crate::stores::FunctionStore, &str, Option<i32>, &str) -> Result<(), String>) -> Result<String, String>,
    update_functions_for_session_exit: fn(&AppHandle, &crate::stores::FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
) -> Result<String, String> {
    let server = load_server_by_id(app, &server_id)?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP")?;
    let cmd = build_scp_command(
        &endpoint,
        &local_path,
        &remote_path,
        recursive.unwrap_or(false),
        true,
    );
    open_pty_with_command_builder(120, 24, cmd, app, session_store, update_functions_for_session_exit)
}
