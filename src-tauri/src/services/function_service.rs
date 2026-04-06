use std::{collections::HashMap, sync::Mutex};

use tauri::{AppHandle, Emitter};

use crate::{cc::telemetry::forward_log_event, models::FunctionStatus, stores::FunctionStore};

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn lock_err<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
    mutex.lock().map_err(|_| "mutex poisoned".to_string())
}

pub fn default_function_statuses(
    openvpn_function_id: &str,
    caddy_function_id: &str,
    boundary_auth_function_id: &str,
) -> HashMap<String, FunctionStatus> {
    let mut statuses = HashMap::new();
    for (id, msg) in [
        (openvpn_function_id, "Ready"),
        (caddy_function_id, "Ready"),
        (boundary_auth_function_id, "Not authenticated"),
    ] {
        statuses.insert(
            id.to_string(),
            FunctionStatus {
                function_id: id.to_string(),
                state: "idle".to_string(),
                message: msg.to_string(),
                boundary_token_source: None,
                session_id: None,
                updated_at_ms: now_ms(),
            },
        );
    }
    statuses
}

pub fn upsert_function_status(
    app: &AppHandle,
    function_store: &FunctionStore,
    function_id: &str,
    state: &str,
    message: &str,
    session_id: Option<String>,
) -> Result<(), String> {
    let status = FunctionStatus {
        function_id: function_id.to_string(),
        state: state.to_string(),
        message: message.to_string(),
        boundary_token_source: None,
        session_id,
        updated_at_ms: now_ms(),
    };

    {
        let mut statuses = lock_err(&function_store.statuses)?;
        statuses.insert(function_id.to_string(), status.clone());
    }

    let _ = app.emit("function-status", status.clone());
    forward_log_event(
        app,
        "function-status",
        status.session_id.as_deref(),
        Some(&status.function_id),
        &status,
    );
    Ok(())
}

pub fn upsert_boundary_status(
    app: &AppHandle,
    function_store: &FunctionStore,
    state: &str,
    message: &str,
    boundary_token_source: Option<&str>,
    session_id: Option<String>,
    boundary_auth_function_id: &str,
) -> Result<(), String> {
    let status = FunctionStatus {
        function_id: boundary_auth_function_id.to_string(),
        state: state.to_string(),
        message: message.to_string(),
        boundary_token_source: boundary_token_source.map(|s| s.to_string()),
        session_id,
        updated_at_ms: now_ms(),
    };

    {
        let mut statuses = lock_err(&function_store.statuses)?;
        statuses.insert(boundary_auth_function_id.to_string(), status.clone());
    }

    let _ = app.emit("function-status", status.clone());
    forward_log_event(
        app,
        "function-status",
        status.session_id.as_deref(),
        Some(&status.function_id),
        &status,
    );
    Ok(())
}
