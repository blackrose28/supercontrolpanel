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
use tauri::{AppHandle, Runtime};

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

fn previous_function_session_id(
    function_store: &FunctionStore,
    function_id: &str,
) -> Result<Option<String>, String> {
    let statuses = lock_err(&function_store.statuses)?;
    Ok(statuses
        .get(function_id)
        .and_then(|status| status.session_id.clone()))
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
        Option<&str>,
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
    if let Some(prev) = previous_function_session_id(function_store, &fid)? {
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
        None,
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
pub fn stop_server_tunnel<R: Runtime>(
    server_id: String,
    app: &AppHandle<R>,
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
    emit_pty_exit: fn(&AppHandle<R>, crate::models::PtyExitPayload),
    upsert_function_status: fn(
        &AppHandle<R>,
        &FunctionStore,
        &str,
        &str,
        &str,
        Option<String>,
    ) -> Result<(), String>,
    load_servers: fn(&AppHandle<R>) -> Result<Vec<crate::server_mgmt::Server>, String>,
    server_tunnel_function_id: fn(&str) -> String,
) -> Result<(), String> {
    close_remote_browser_sessions_for_server(&server_id, browser_store)?;
    let fid = server_tunnel_function_id(&server_id);
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
    let sid = lock_err(&function_store.statuses)?
        .get(&fid)
        .and_then(|s| s.session_id.clone());

    if let Some(sid) = sid {
        if let Some(p) = close_pty_internal(&sid, session_store)? {
            emit_pty_exit(app, p);
        }
    }

    upsert_function_status(app, function_store, &fid, "idle", &message, None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{previous_function_session_id, stop_server_tunnel};
    use crate::{
        models::{FunctionStatus, PtyExitPayload},
        stores::{FunctionStore, RemoteBrowserStore, SessionStore},
    };
    use std::{collections::HashMap, sync::Mutex};

    fn close_no_remote_browsers(
        _server_id: &str,
        _browser_store: &RemoteBrowserStore,
    ) -> Result<usize, String> {
        Ok(0)
    }

    fn close_pty_should_not_be_called(
        _session_id: &str,
        _session_store: &SessionStore,
    ) -> Result<Option<PtyExitPayload>, String> {
        panic!("close_pty_internal should not be called without an active session id")
    }

    fn emit_noop<R: tauri::Runtime>(_app: &tauri::AppHandle<R>, _payload: PtyExitPayload) {}

    fn upsert_test_status<R: tauri::Runtime>(
        _app: &tauri::AppHandle<R>,
        function_store: &FunctionStore,
        function_id: &str,
        state: &str,
        message: &str,
        session_id: Option<String>,
    ) -> Result<(), String> {
        function_store
            .statuses
            .lock()
            .map_err(|err| err.to_string())?
            .insert(
                function_id.to_string(),
                FunctionStatus {
                    function_id: function_id.to_string(),
                    state: state.to_string(),
                    message: message.to_string(),
                    boundary_token_source: None,
                    session_id,
                    updated_at_ms: 1,
                },
            );
        Ok(())
    }

    fn load_no_servers<R: tauri::Runtime>(
        _app: &tauri::AppHandle<R>,
    ) -> Result<Vec<crate::server_mgmt::Server>, String> {
        Ok(Vec::new())
    }

    fn tunnel_function_id(server_id: &str) -> String {
        format!("server-tunnel-{server_id}")
    }

    #[test]
    fn previous_function_session_id_releases_status_lock() {
        let function_store = FunctionStore {
            statuses: Mutex::new(HashMap::from([(
                "server-ssh-srv-1".to_string(),
                FunctionStatus {
                    function_id: "server-ssh-srv-1".to_string(),
                    state: "stopped".to_string(),
                    message: "PTY exited".to_string(),
                    boundary_token_source: None,
                    session_id: Some("pty-old".to_string()),
                    updated_at_ms: 1,
                },
            )])),
        };

        let previous = previous_function_session_id(&function_store, "server-ssh-srv-1")
            .expect("previous session lookup should succeed");

        assert_eq!(previous.as_deref(), Some("pty-old"));
        assert!(
            function_store.statuses.try_lock().is_ok(),
            "previous session lookup must release the status lock before callers do cleanup"
        );
    }

    #[test]
    fn stop_server_tunnel_marks_idle_even_when_status_has_no_session_id() {
        let app = tauri::test::mock_app();
        let app_handle = app.handle().clone();
        let session_store = SessionStore::default();
        let function_store = FunctionStore {
            statuses: Mutex::new(HashMap::from([(
                "server-tunnel-srv-1".to_string(),
                FunctionStatus {
                    function_id: "server-tunnel-srv-1".to_string(),
                    state: "running".to_string(),
                    message: "Tunnel: Demo".to_string(),
                    boundary_token_source: None,
                    session_id: None,
                    updated_at_ms: 1,
                },
            )])),
        };
        let browser_store = RemoteBrowserStore::default();

        stop_server_tunnel(
            "srv-1".to_string(),
            &app_handle,
            &session_store,
            &function_store,
            &browser_store,
            close_no_remote_browsers,
            close_pty_should_not_be_called,
            emit_noop,
            upsert_test_status,
            load_no_servers,
            tunnel_function_id,
        )
        .expect("stop tunnel should succeed without a live PTY session id");

        let statuses = function_store
            .statuses
            .lock()
            .expect("function statuses lock");
        let status = statuses
            .get("server-tunnel-srv-1")
            .expect("server tunnel status should be preserved");
        assert_eq!(status.state, "idle");
        assert_eq!(status.session_id, None);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn start_server_ssh(
    server_id: String,
    cols: u16,
    rows: u16,
    forced_local_session_id: Option<&str>,
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
        Option<&str>,
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
    if let Some(prev) = previous_function_session_id(function_store, &fid)? {
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
        forced_local_session_id,
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
