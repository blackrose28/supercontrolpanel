use tauri::{AppHandle, Manager, State};

use super::{update_functions_for_session_exit, upsert_function_status};
use crate::commands::close_remote_browser_sessions_for_server;
use crate::commands::load_app_config_from_disk;
use crate::models::StartFunctionResult;
use crate::server_mgmt::{load_servers, new_server_id, save_servers, Server};
use crate::services::{
    apply_boundary_env_builder, apply_boundary_token_arg_if_present_builder, boundary_cli_path,
    close_pty_internal, emit_pty_exit, open_pty_with_command_builder,
    start_server_rdp as start_server_rdp_service, start_server_ssh as start_server_ssh_service,
    start_server_tunnel as start_server_tunnel_service, stop_server_ssh as stop_server_ssh_service,
    stop_server_tunnel as stop_server_tunnel_service,
};
use crate::stores::{FunctionStore, RemoteBrowserStore, SessionStore};
#[cfg(target_os = "windows")]
use crate::CREATE_NO_WINDOW;
use crate::{server_ssh_function_id, server_tunnel_function_id};

#[tauri::command]
pub fn list_servers(app: AppHandle) -> Result<Vec<Server>, String> {
    load_servers(&app)
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
pub fn upsert_server(
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
pub fn delete_server(
    app: AppHandle,
    id: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<Server>, String> {
    delete_server_action(app, id, browser_store)
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
pub fn start_server_tunnel(
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
        load_servers,
        server_tunnel_function_id,
    )
}

async fn run_stop_server_tunnel_blocking<F>(action: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(action)
        .await
        .map_err(|err| format!("failed to join stop server tunnel task: {err}"))?
}

pub(crate) fn restart_server_tunnel_action(
    server_id: String,
    app: AppHandle,
) -> Result<(), String> {
    let cfg = load_app_config_from_disk(&app)?;
    let cols = cfg.app.default_terminal_cols;
    let rows = cfg.app.default_terminal_rows;
    let is_running = {
        let function_store = app.state::<FunctionStore>();
        let fid = server_tunnel_function_id(&server_id);
        let running = crate::services::lock_err(&function_store.statuses)?
            .get(&fid)
            .and_then(|status| status.session_id.clone())
            .is_some();
        running
    };

    if is_running {
        let session_store = app.state::<SessionStore>();
        let function_store = app.state::<FunctionStore>();
        let browser_store = app.state::<RemoteBrowserStore>();
        stop_server_tunnel_action(
            server_id.clone(),
            app.clone(),
            session_store,
            function_store,
            browser_store,
        )?;
    } else {
        let session_store = app.state::<SessionStore>();
        let function_store = app.state::<FunctionStore>();
        let browser_store = app.state::<RemoteBrowserStore>();
        return start_server_tunnel_action(
            server_id,
            cols,
            rows,
            app.clone(),
            session_store,
            function_store,
            browser_store,
        )
        .map(|_| ());
    }

    let restart_app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let session_store = restart_app.state::<SessionStore>();
        let function_store = restart_app.state::<FunctionStore>();
        let browser_store = restart_app.state::<RemoteBrowserStore>();
        if let Err(err) = start_server_tunnel_action(
            server_id.clone(),
            cols,
            rows,
            restart_app.clone(),
            session_store,
            function_store,
            browser_store,
        ) {
            let function_store = restart_app.state::<FunctionStore>();
            let fid = server_tunnel_function_id(&server_id);
            let _ = upsert_function_status(
                &restart_app,
                &function_store,
                &fid,
                "error",
                &format!("CC server tunnel restart failed: {err}"),
                None,
            );
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn stop_server_tunnel(server_id: String, app: AppHandle) -> Result<(), String> {
    run_stop_server_tunnel_blocking(move || {
        let session_store = app.state::<SessionStore>();
        let function_store = app.state::<FunctionStore>();
        let browser_store = app.state::<RemoteBrowserStore>();
        stop_server_tunnel_action(
            server_id,
            app.clone(),
            session_store,
            function_store,
            browser_store,
        )
    })
    .await
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
pub fn start_server_ssh(
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

pub(crate) fn restart_server_ssh_action(server_id: String, app: AppHandle) -> Result<(), String> {
    let cfg = load_app_config_from_disk(&app)?;
    let cols = cfg.app.default_terminal_cols;
    let rows = cfg.app.default_terminal_rows;
    let is_running = {
        let function_store = app.state::<FunctionStore>();
        let fid = server_ssh_function_id(&server_id);
        let running = crate::services::lock_err(&function_store.statuses)?
            .get(&fid)
            .and_then(|status| status.session_id.clone())
            .is_some();
        running
    };

    if is_running {
        let session_store = app.state::<SessionStore>();
        let function_store = app.state::<FunctionStore>();
        stop_server_ssh_action(
            server_id.clone(),
            app.clone(),
            session_store,
            function_store,
        )?;
    } else {
        let session_store = app.state::<SessionStore>();
        let function_store = app.state::<FunctionStore>();
        return start_server_ssh_action(
            server_id,
            cols,
            rows,
            app.clone(),
            session_store,
            function_store,
        )
        .map(|_| ());
    }

    let restart_app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let session_store = restart_app.state::<SessionStore>();
        let function_store = restart_app.state::<FunctionStore>();
        if let Err(err) = start_server_ssh_action(
            server_id.clone(),
            cols,
            rows,
            restart_app.clone(),
            session_store,
            function_store,
        ) {
            let function_store = restart_app.state::<FunctionStore>();
            let fid = server_ssh_function_id(&server_id);
            let _ = upsert_function_status(
                &restart_app,
                &function_store,
                &fid,
                "error",
                &format!("CC server SSH restart failed: {err}"),
                None,
            );
        }
    });

    Ok(())
}

#[tauri::command]
pub fn start_server_rdp(app: AppHandle, server_id: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        start_server_rdp_service(&app, server_id, CREATE_NO_WINDOW)
    }
    #[cfg(not(target_os = "windows"))]
    {
        start_server_rdp_service(&app, server_id)
    }
}

#[tauri::command]
pub fn stop_server_ssh(
    server_id: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<(), String> {
    stop_server_ssh_action(server_id, app, session_store, function_store)
}

#[cfg(test)]
mod tests {
    use super::run_stop_server_tunnel_blocking;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn stop_server_tunnel_blocking_helper_runs_action_off_async_task() {
        let thread_id = std::thread::current().id();
        let action_thread_id = Arc::new(Mutex::new(None));
        let action_thread_id_clone = action_thread_id.clone();

        run_stop_server_tunnel_blocking(move || {
            *action_thread_id_clone
                .lock()
                .expect("action thread id lock") = Some(std::thread::current().id());
            Ok(())
        })
        .await
        .expect("blocking action should succeed");

        assert_ne!(
            action_thread_id
                .lock()
                .expect("action thread id lock")
                .unwrap(),
            thread_id
        );
    }
}
