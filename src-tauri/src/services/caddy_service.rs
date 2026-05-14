use std::path::PathBuf;

use portable_pty::CommandBuilder;
use tauri::AppHandle;

use crate::{
    models::{PtyExitPayload, StartFunctionResult},
    stores::{FunctionStore, SessionStore},
};

#[allow(clippy::too_many_arguments)]
pub fn start_caddy_run(
    cols: u16,
    rows: u16,
    folder: String,
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    caddy_function_id: &str,
    lock_err: fn(
        &std::sync::Mutex<std::collections::HashMap<String, crate::models::FunctionStatus>>,
    ) -> Result<
        std::sync::MutexGuard<'_, std::collections::HashMap<String, crate::models::FunctionStatus>>,
        String,
    >,
    close_pty_internal: fn(&str, &SessionStore) -> Result<Option<PtyExitPayload>, String>,
    emit_pty_exit: fn(&AppHandle, PtyExitPayload),
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
) -> Result<StartFunctionResult, String> {
    let folder_trim = folder.trim();
    if folder_trim.is_empty() {
        return Err("folder cannot be empty".to_string());
    }
    let folder_path = PathBuf::from(folder_trim);
    if !folder_path.exists() {
        return Err(format!(
            "folder does not exist: {}",
            folder_path.to_string_lossy()
        ));
    }
    let caddyfile = folder_path.join("Caddyfile");
    if !caddyfile.exists() {
        return Err(format!(
            "Caddyfile not found in folder: {}",
            folder_path.to_string_lossy()
        ));
    }

    if let Some(prev) = lock_err(&function_store.statuses)?
        .get(caddy_function_id)
        .and_then(|s| s.session_id.clone())
    {
        if let Some(p) = close_pty_internal(&prev, session_store)? {
            emit_pty_exit(app, p);
        }
        let _ = update_functions_for_session_exit(app, function_store, &prev, None, "replaced");
    }

    let mut cmd = CommandBuilder::new("caddy");
    cmd.arg("run");
    cmd.arg("--config");
    cmd.arg("Caddyfile");
    cmd.cwd(folder_path.clone());

    let session_id = open_pty_with_command_builder(
        cols,
        rows,
        cmd,
        app,
        session_store,
        update_functions_for_session_exit,
        None,
    )?;
    upsert_function_status(
        app,
        function_store,
        caddy_function_id,
        "running",
        &format!("Caddy running in {}", folder_path.to_string_lossy()),
        Some(session_id.clone()),
    )?;

    Ok(StartFunctionResult {
        function_id: caddy_function_id.to_string(),
        session_id,
    })
}

pub fn stop_caddy_run(
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    caddy_function_id: &str,
    lock_err: fn(
        &std::sync::Mutex<std::collections::HashMap<String, crate::models::FunctionStatus>>,
    ) -> Result<
        std::sync::MutexGuard<'_, std::collections::HashMap<String, crate::models::FunctionStatus>>,
        String,
    >,
    close_pty_internal: fn(&str, &SessionStore) -> Result<Option<PtyExitPayload>, String>,
    emit_pty_exit: fn(&AppHandle, PtyExitPayload),
    upsert_function_status: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        &str,
        &str,
        Option<String>,
    ) -> Result<(), String>,
) -> Result<(), String> {
    let sid = match lock_err(&function_store.statuses)?
        .get(caddy_function_id)
        .and_then(|s| s.session_id.clone())
    {
        Some(s) => s,
        None => {
            upsert_function_status(
                app,
                function_store,
                caddy_function_id,
                "idle",
                "No running session",
                None,
            )?;
            return Ok(());
        }
    };
    if let Some(p) = close_pty_internal(&sid, session_store)? {
        emit_pty_exit(app, p);
    }
    upsert_function_status(
        app,
        function_store,
        caddy_function_id,
        "idle",
        "Stopped",
        None,
    )?;
    Ok(())
}
