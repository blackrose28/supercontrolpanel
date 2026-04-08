use std::{
    path::PathBuf, process::Command, sync::atomic::AtomicBool, sync::Arc, thread, time::Duration,
};

use serde_json;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, State};

use super::{
    boundary_token_summary, load_app_config_from_disk, update_functions_for_session_exit,
    upsert_boundary_status, upsert_function_status,
};
use crate::models::{BoundaryConfig, BoundaryTargetSummary, StartFunctionResult};
use crate::services::{
    apply_boundary_env, apply_boundary_env_builder, apply_boundary_token_arg_if_present,
    apply_boundary_token_arg_if_present_builder, boundary_addr_base, boundary_cli_path,
    boundary_http_client, boundary_oidc_init_script as boundary_oidc_init_script_service,
    close_pty_internal,
    complete_boundary_automation_session as complete_boundary_automation_session_service,
    emit_pty_exit, emit_pty_output_line, generate_totp_code, keyring_set_or_delete, lock_err,
    open_pty_with_command_builder, passive_pty_command, prepare_boundary_auth_start,
    run_boundary_oidc_automation as run_boundary_oidc_automation_service,
    stop_boundary_auth as stop_boundary_auth_service,
    stop_boundary_automation_session as stop_boundary_automation_session_service,
    write_pty_internal, BoundaryAuthLaunch, KEYRING_BOUNDARY_APP_TOKEN,
};
use crate::stores::{
    BoundaryAutomationSession, BoundaryAutomationStore, FunctionStore, SessionStore,
};
#[cfg(target_os = "windows")]
use crate::CREATE_NO_WINDOW;
use crate::{BOUNDARY_AUTH_FUNCTION_ID, BOUNDARY_OIDC_WINDOW_LABEL};

fn boundary_oidc_init_script(cfg: &BoundaryConfig) -> String {
    boundary_oidc_init_script_service(cfg, generate_totp_code)
}

/// Search Boundary targets (uses CLI token). Optional text filter; empty = list all.
#[tauri::command]
pub fn search_boundary_targets(
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
pub fn start_boundary_auth(
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
pub fn stop_boundary_auth(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    automation_store: State<'_, BoundaryAutomationStore>,
) -> Result<(), String> {
    stop_boundary_auth_action(app, session_store, function_store, automation_store)
}
