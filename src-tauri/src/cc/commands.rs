use tauri::AppHandle;

use crate::services::load_sync_settings_from_disk;

use super::connection::CcConnection;

#[tauri::command]
pub async fn init_cc_connection(app: AppHandle) -> Result<String, String> {
    let settings =
        load_sync_settings_from_disk(&app, "sync-local.json").map_err(|e| e.to_string())?;

    if !settings.cc_enabled {
        return Ok("CC disabled".to_string());
    }

    if settings.cc_server_url.is_empty()
        || settings.cc_agent_token.is_empty()
        || settings.cc_device_id.is_empty()
    {
        return Err("CC config incomplete".to_string());
    }

    let connection = CcConnection::new(
        settings.cc_server_url.clone(),
        settings.cc_agent_token.clone(),
        settings.cc_device_id.clone(),
        app.clone(),
    );

    // Spawn connection in background
    tokio::spawn(async move {
        if let Err(e) = connection.connect_with_retry().await {
            eprintln!("CC connection error: {}", e);
        }
    });

    Ok(format!("CC connection started for device {}", settings.cc_device_id))
}

#[tauri::command]
pub fn get_cc_status(app: AppHandle) -> Result<serde_json::Value, String> {
    let settings =
        load_sync_settings_from_disk(&app, "sync-local.json").map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "enabled": settings.cc_enabled,
        "serverUrl": settings.cc_server_url,
        "authMode": settings.cc_auth_mode,
        "deviceId": settings.cc_device_id,
        "connected": false // TODO: track actual connection state
    }))
}
