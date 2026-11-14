//! Server inventory (metadata) persisted in `servers.json`.

use scp_core::models::ServersFile;
#[allow(unused_imports)]
pub use scp_core::models::{new_server_id, ConnectionMethod, Server, ServerOs, ServerService};
use std::fs;
use tauri::AppHandle;
use tauri::Manager;

fn servers_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("app config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
    Ok(dir.join("servers.json"))
}

pub fn load_servers(app: &AppHandle) -> Result<Vec<Server>, String> {
    let _settings_guard = crate::settings_backup::gate::enter()?;
    let path = servers_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("read servers: {e}"))?;
    let file: ServersFile =
        serde_json::from_str(&raw).map_err(|e| format!("parse servers: {e}"))?;
    Ok(file.servers)
}

pub fn save_servers(app: &AppHandle, servers: &[Server]) -> Result<(), String> {
    let _settings_guard = crate::settings_backup::gate::enter()?;
    crate::settings_backup::gate::changed();
    let path = servers_path(app)?;
    let file = ServersFile {
        servers: servers.to_vec(),
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| format!("encode servers: {e}"))?;
    fs::write(path, json).map_err(|e| format!("write servers: {e}"))
}
