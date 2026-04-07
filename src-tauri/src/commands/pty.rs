use portable_pty::{CommandBuilder, PtySize};
use tauri::{AppHandle, State};

use crate::models::FunctionStatus;
use crate::services::{
    close_pty_internal, emit_pty_exit, lock_err, open_pty_with_command_builder,
    write_pty_internal,
};
use crate::stores::{FunctionStore, SessionStore};
use super::{
    boundary_token_summary, load_app_config_from_disk, update_functions_for_session_exit,
};
use crate::BOUNDARY_AUTH_FUNCTION_ID;

pub(crate) fn open_pty_internal(
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
pub(crate) fn adapt_command_for_shell(command: &str) -> String {
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

pub(crate) fn default_shell_command() -> CommandBuilder {
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
pub fn open_pty(
    cols: u16,
    rows: u16,
    app: AppHandle,
    store: State<'_, SessionStore>,
) -> Result<String, String> {
    open_pty_internal(cols, rows, &app, &store)
}

#[tauri::command]
pub fn write_pty(id: String, data: String, store: State<'_, SessionStore>) -> Result<(), String> {
    write_pty_internal(&id, &data, &store)
}

#[tauri::command]
pub fn resize_pty(
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
pub fn close_pty(
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
pub fn list_function_statuses(
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
