use crate::services::{load_sync_settings_from_disk, upsert_function_status};
use crate::stores::{CcRuntimeState, FunctionStore};
use tauri::{AppHandle, Manager};

use super::connection::CcConnection;
use crate::CC_FUNCTION_ID;

pub(crate) fn set_cc_status(app: &AppHandle, state: &str, message: &str) -> Result<(), String> {
    let function_store = app.state::<FunctionStore>();
    upsert_function_status(app, &function_store, CC_FUNCTION_ID, state, message, None)
}

pub(crate) fn sync_cc_status_from_settings(app: &AppHandle) -> Result<(), String> {
    let settings =
        load_sync_settings_from_disk(app, "sync-local.json").map_err(|e| e.to_string())?;
    let runtime = app.state::<CcRuntimeState>();
    let (state, message) = if !settings.cc_enabled {
        ("disabled", "Disabled in Config.".to_string())
    } else if settings.cc_server_url.is_empty()
        || settings.cc_agent_token.is_empty()
        || settings.cc_device_id.is_empty()
    {
        (
            "error",
            "CC config incomplete. Fill server URL, agent token, and device ID in Config."
                .to_string(),
        )
    } else if runtime.is_connected() {
        let label = if settings.cc_device_name.trim().is_empty() {
            settings.cc_device_id.clone()
        } else {
            settings.cc_device_name.clone()
        };
        (
            "connected",
            format!("Connected as {} to {}.", label, settings.cc_server_url),
        )
    } else {
        let label = if settings.cc_device_name.trim().is_empty() {
            settings.cc_device_id.clone()
        } else {
            settings.cc_device_name.clone()
        };
        (
            "idle",
            format!(
                "Configured for {} at {}. Use Re-init CC from Config to connect.",
                label, settings.cc_server_url
            ),
        )
    };
    set_cc_status(app, state, &message)
}

#[tauri::command]
pub async fn init_cc_connection(app: AppHandle) -> Result<String, String> {
    let settings =
        load_sync_settings_from_disk(&app, "sync-local.json").map_err(|e| e.to_string())?;
    let generation = app.state::<CcRuntimeState>().begin_generation();

    if !settings.cc_enabled {
        let _ = sync_cc_status_from_settings(&app);
        return Ok("CC disabled".to_string());
    }

    if settings.cc_server_url.is_empty()
        || settings.cc_agent_token.is_empty()
        || settings.cc_device_id.is_empty()
    {
        let _ = sync_cc_status_from_settings(&app);
        return Err("CC config incomplete".to_string());
    }

    let _ = set_cc_status(
        &app,
        "connecting",
        &format!(
            "Connecting to {} as {}.",
            settings.cc_server_url, settings.cc_device_id
        ),
    );

    let connection = CcConnection::new(
        settings.cc_server_url.clone(),
        settings.cc_agent_token.clone(),
        settings.cc_device_id.clone(),
        app.clone(),
        generation,
    );

    // Spawn connection in background
    tokio::spawn(async move {
        if let Err(e) = connection.connect_with_retry().await {
            eprintln!("CC connection error: {}", e);
        }
    });

    Ok(format!(
        "CC connection started for device {}",
        settings.cc_device_id
    ))
}

#[tauri::command]
pub fn get_cc_status(app: AppHandle) -> Result<serde_json::Value, String> {
    let settings =
        load_sync_settings_from_disk(&app, "sync-local.json").map_err(|e| e.to_string())?;
    let runtime = app.state::<CcRuntimeState>();
    Ok(serde_json::json!({
        "enabled": settings.cc_enabled,
        "serverUrl": settings.cc_server_url,
        "authMode": settings.cc_auth_mode,
        "deviceId": settings.cc_device_id,
        "connected": runtime.is_connected()
    }))
}
