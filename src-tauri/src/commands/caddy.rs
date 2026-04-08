use tauri::{AppHandle, State};

use super::{update_functions_for_session_exit, upsert_function_status};
use crate::models::StartFunctionResult;
use crate::services::{
    close_pty_internal, emit_pty_exit, lock_err, open_pty_with_command_builder,
    start_caddy_run as start_caddy_run_service, stop_caddy_run as stop_caddy_run_service,
};
use crate::stores::{FunctionStore, SessionStore};
use crate::CADDY_FUNCTION_ID;

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
pub fn start_caddy_run(
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
pub fn stop_caddy_run(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<(), String> {
    stop_caddy_action(app, session_store, function_store)
}
