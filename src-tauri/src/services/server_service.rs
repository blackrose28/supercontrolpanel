#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::process::Command;

use portable_pty::CommandBuilder;

use crate::{
    models::SshEndpoint,
    server_mgmt::{load_servers, ConnectionMethod, Server},
    services::lock_err,
    stores::FunctionStore,
};
use tauri::AppHandle;

pub fn load_server_by_id(app: &AppHandle, server_id: &str) -> Result<Server, String> {
    load_servers(app)?
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| "server not found".to_string())
}

pub fn resolve_server_ssh_endpoint(
    server: &Server,
    operation: &str,
) -> Result<SshEndpoint, String> {
    if server.ssh_user.trim().is_empty() {
        return Err(format!("SSH user is required for {operation}"));
    }
    match &server.connection_method {
        ConnectionMethod::Ssh => {
            if server.host.trim().is_empty() {
                return Err(format!("Host is required for {operation}"));
            }
            Ok(SshEndpoint {
                user: server.ssh_user.trim().to_string(),
                host: server.host.trim().to_string(),
                port: server.ssh_port,
            })
        }
        ConnectionMethod::Boundary => Ok(SshEndpoint {
            user: server.ssh_user.trim().to_string(),
            host: "127.0.0.1".to_string(),
            port: if server.boundary_listen_port == 0 {
                6222
            } else {
                server.boundary_listen_port
            },
        }),
    }
}

pub fn add_common_ssh_options(cmd: &mut Command, port_flag: &str, port: u16) {
    cmd.arg(port_flag)
        .arg(port.to_string())
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("MACs=hmac-sha2-512");
}

pub fn ssh_target(endpoint: &SshEndpoint) -> String {
    format!("{}@{}", endpoint.user, endpoint.host)
}

pub fn ensure_boundary_tunnel_running(
    function_store: &FunctionStore,
    server: &Server,
    server_tunnel_function_id: fn(&str) -> String,
) -> Result<(), String> {
    if server.connection_method != ConnectionMethod::Boundary {
        return Ok(());
    }
    let tunnel_id = server_tunnel_function_id(&server.id);
    let statuses = lock_err(&function_store.statuses)?;
    let running = statuses
        .get(&tunnel_id)
        .map(|status| status.state == "running" && status.session_id.is_some())
        .unwrap_or(false);
    if running {
        Ok(())
    } else {
        Err("Boundary tunnel is not running for this server.".to_string())
    }
}

pub fn build_scp_command(
    endpoint: &SshEndpoint,
    local_path: &str,
    remote_path: &str,
    recursive: bool,
    download: bool,
) -> CommandBuilder {
    let remote_spec = format!("{}:{}", ssh_target(endpoint), remote_path);
    let mut cmd = CommandBuilder::new("scp");
    cmd.arg("-P");
    cmd.arg(endpoint.port.to_string());
    cmd.arg("-o");
    cmd.arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-o");
    cmd.arg("MACs=hmac-sha2-512");
    if recursive {
        cmd.arg("-r");
    }
    if download {
        cmd.arg(remote_spec);
        cmd.arg(local_path);
    } else {
        cmd.arg(local_path);
        cmd.arg(remote_spec);
    }
    cmd
}

use crate::{
    models::{AppConfig, StartFunctionResult},
    server_mgmt::ServerOs,
    stores::{RemoteBrowserStore, SessionStore},
};

#[allow(clippy::too_many_arguments)]
pub fn start_server_tunnel(
    server_id: String,
    cols: u16,
    rows: u16,
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    browser_store: &RemoteBrowserStore,
    close_remote_browser_sessions_for_server: fn(
        &str,
        &RemoteBrowserStore,
    ) -> Result<usize, String>,
    close_pty_internal: fn(
        &str,
        &SessionStore,
    ) -> Result<Option<crate::models::PtyExitPayload>, String>,
    emit_pty_exit: fn(&AppHandle, crate::models::PtyExitPayload),
    update_functions_for_session_exit: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        Option<i32>,
        &str,
    ) -> Result<(), String>,
    load_app_config_from_disk: fn(&AppHandle) -> Result<AppConfig, String>,
    boundary_cli_path: fn(&AppConfig) -> String,
    apply_boundary_token_arg_if_present_builder: fn(&mut CommandBuilder) -> Result<(), String>,
    apply_boundary_env_builder: fn(&mut CommandBuilder, &AppConfig) -> Result<(), String>,
    open_pty_with_command_builder: fn(
        u16,
        u16,
        CommandBuilder,
        &AppHandle,
        &SessionStore,
        fn(&AppHandle, &FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
    ) -> Result<String, String>,
    upsert_function_status: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        &str,
        &str,
        Option<String>,
    ) -> Result<(), String>,
    server_tunnel_function_id: fn(&str) -> String,
) -> Result<StartFunctionResult, String> {
    close_remote_browser_sessions_for_server(&server_id, browser_store)?;
    let server = load_server_by_id(app, &server_id)?;
    if server.connection_method != ConnectionMethod::Boundary {
        return Err("Tunnel is only for Boundary servers".to_string());
    }
    if server.boundary_target_id.trim().is_empty() {
        return Err("Boundary target ID is required".to_string());
    }
    let fid = server_tunnel_function_id(&server_id);
    if let Some(prev) = lock_err(&function_store.statuses)?
        .get(&fid)
        .and_then(|s| s.session_id.clone())
    {
        if let Some(p) = close_pty_internal(&prev, session_store)? {
            emit_pty_exit(app, p);
        }
        let _ = update_functions_for_session_exit(app, function_store, &prev, None, "replaced");
    }
    let port = if server.boundary_listen_port == 0 {
        6222u16
    } else {
        server.boundary_listen_port
    };
    let cfg = load_app_config_from_disk(app)?;
    let exe = boundary_cli_path(&cfg);
    let mut c = CommandBuilder::new(exe);
    c.arg("connect");
    apply_boundary_token_arg_if_present_builder(&mut c)?;
    apply_boundary_env_builder(&mut c, &cfg)?;
    c.arg("-target-id");
    c.arg(server.boundary_target_id.trim());
    c.arg("-listen-port");
    c.arg(port.to_string());
    let session_id = open_pty_with_command_builder(
        cols,
        rows,
        c,
        app,
        session_store,
        update_functions_for_session_exit,
    )?;
    upsert_function_status(
        app,
        function_store,
        &fid,
        "running",
        &format!("Tunnel: {} (port {})", server.name, port),
        Some(session_id.clone()),
    )?;
    Ok(StartFunctionResult {
        function_id: fid,
        session_id,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn stop_server_tunnel(
    server_id: String,
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    browser_store: &RemoteBrowserStore,
    close_remote_browser_sessions_for_server: fn(
        &str,
        &RemoteBrowserStore,
    ) -> Result<usize, String>,
    close_pty_internal: fn(
        &str,
        &SessionStore,
    ) -> Result<Option<crate::models::PtyExitPayload>, String>,
    emit_pty_exit: fn(&AppHandle, crate::models::PtyExitPayload),
    upsert_function_status: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        &str,
        &str,
        Option<String>,
    ) -> Result<(), String>,
    server_tunnel_function_id: fn(&str) -> String,
) -> Result<(), String> {
    close_remote_browser_sessions_for_server(&server_id, browser_store)?;
    let fid = server_tunnel_function_id(&server_id);
    let sid = match lock_err(&function_store.statuses)?
        .get(&fid)
        .and_then(|s| s.session_id.clone())
    {
        Some(s) => s,
        None => return Ok(()),
    };
    if let Some(p) = close_pty_internal(&sid, session_store)? {
        emit_pty_exit(app, p);
    }
    let message = load_servers(app)
        .ok()
        .and_then(|servers| servers.into_iter().find(|s| s.id == server_id))
        .map(|s| {
            let port = if s.boundary_listen_port == 0 {
                6222
            } else {
                s.boundary_listen_port
            };
            format!("Tunnel: {} (port {}) — stopped", s.name, port)
        })
        .unwrap_or_else(|| "Stopped".to_string());
    upsert_function_status(app, function_store, &fid, "idle", &message, None)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn start_server_ssh(
    server_id: String,
    cols: u16,
    rows: u16,
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    close_pty_internal: fn(
        &str,
        &SessionStore,
    ) -> Result<Option<crate::models::PtyExitPayload>, String>,
    emit_pty_exit: fn(&AppHandle, crate::models::PtyExitPayload),
    update_functions_for_session_exit: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        Option<i32>,
        &str,
    ) -> Result<(), String>,
    open_pty_with_command_builder: fn(
        u16,
        u16,
        CommandBuilder,
        &AppHandle,
        &SessionStore,
        fn(&AppHandle, &FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
    ) -> Result<String, String>,
    upsert_function_status: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        &str,
        &str,
        Option<String>,
    ) -> Result<(), String>,
    server_ssh_function_id: fn(&str) -> String,
) -> Result<StartFunctionResult, String> {
    let server = load_server_by_id(app, &server_id)?;
    let fid = server_ssh_function_id(&server_id);
    if let Some(prev) = lock_err(&function_store.statuses)?
        .get(&fid)
        .and_then(|s| s.session_id.clone())
    {
        if let Some(p) = close_pty_internal(&prev, session_store)? {
            emit_pty_exit(app, p);
        }
        let _ = update_functions_for_session_exit(app, function_store, &prev, None, "replaced");
    }
    let cmd = match &server.connection_method {
        ConnectionMethod::Ssh => {
            if server.ssh_user.trim().is_empty() || server.host.trim().is_empty() {
                return Err("host and SSH user are required for direct SSH".to_string());
            }
            let mut c = CommandBuilder::new("ssh");
            c.arg("-p");
            c.arg(server.ssh_port.to_string());
            c.arg("-o");
            c.arg("StrictHostKeyChecking=accept-new");
            c.arg("-o");
            c.arg("MACs=hmac-sha2-512");
            c.arg(format!("{}@{}", server.ssh_user.trim(), server.host.trim()));
            c
        }
        ConnectionMethod::Boundary => {
            let port = if server.boundary_listen_port == 0 {
                6222u16
            } else {
                server.boundary_listen_port
            };
            let user = if server.ssh_user.trim().is_empty() {
                return Err("SSH username required for Boundary (connect to localhost)".to_string());
            } else {
                server.ssh_user.trim()
            };
            let mut c = CommandBuilder::new("ssh");
            c.arg("-p");
            c.arg(port.to_string());
            c.arg("-o");
            c.arg("StrictHostKeyChecking=accept-new");
            c.arg("-o");
            c.arg("MACs=hmac-sha2-512");
            c.arg(format!("{}@127.0.0.1", user));
            c
        }
    };
    let session_id = open_pty_with_command_builder(
        cols,
        rows,
        cmd,
        app,
        session_store,
        update_functions_for_session_exit,
    )?;
    upsert_function_status(
        app,
        function_store,
        &fid,
        "running",
        &format!("SSH: {}", server.name),
        Some(session_id.clone()),
    )?;
    Ok(StartFunctionResult {
        function_id: fid,
        session_id,
    })
}

pub fn stop_server_ssh(
    server_id: String,
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    close_pty_internal: fn(
        &str,
        &SessionStore,
    ) -> Result<Option<crate::models::PtyExitPayload>, String>,
    emit_pty_exit: fn(&AppHandle, crate::models::PtyExitPayload),
    upsert_function_status: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        &str,
        &str,
        Option<String>,
    ) -> Result<(), String>,
    server_ssh_function_id: fn(&str) -> String,
) -> Result<(), String> {
    let fid = server_ssh_function_id(&server_id);
    let sid = match lock_err(&function_store.statuses)?
        .get(&fid)
        .and_then(|s| s.session_id.clone())
    {
        Some(s) => s,
        None => return Ok(()),
    };
    if let Some(p) = close_pty_internal(&sid, session_store)? {
        emit_pty_exit(app, p);
    }
    let message = load_servers(app)
        .ok()
        .and_then(|servers| servers.into_iter().find(|s| s.id == server_id))
        .map(|s| format!("SSH: {} — stopped", s.name))
        .unwrap_or_else(|| "Stopped".to_string());
    upsert_function_status(app, function_store, &fid, "idle", &message, None)?;
    Ok(())
}

pub fn start_server_rdp(
    app: &AppHandle,
    server_id: String,
    #[cfg(target_os = "windows")] create_no_window: u32,
) -> Result<(), String> {
    let servers = load_servers(app)?;
    let server = servers
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| "server not found".to_string())?;
    if server.os != ServerOs::Windows {
        return Err(
            "RDP is only available for Windows servers. Edit the server and set OS to Windows."
                .to_string(),
        );
    }
    let host = server.host.trim();
    if host.is_empty() {
        return Err(
            "Host is required for RDP — set a DNS name or IP address on the server.".to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("mstsc.exe");
        cmd.arg(format!("/v:{host}"));
        cmd.creation_flags(create_no_window);
        cmd.spawn()
            .map_err(|e| format!("failed to launch mstsc.exe: {e}"))?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let url = format!("rdp://{host}");
        let status = Command::new("open")
            .arg(&url)
            .status()
            .map_err(|e| format!("failed to launch RDP URL via `open`: {e}"))?;
        if !status.success() {
            return Err("failed to launch RDP client via `open`".to_string());
        }
        Ok(())
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let url = format!("rdp://{host}");
        let status = Command::new("xdg-open")
            .arg(&url)
            .status()
            .map_err(|e| format!("failed to launch RDP URL via `xdg-open`: {e}"))?;
        if !status.success() {
            return Err("failed to launch RDP client via `xdg-open`".to_string());
        }
        Ok(())
    }
}
