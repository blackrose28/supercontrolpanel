//! Server inventory (metadata) persisted in `servers.json`.

use serde::{Deserialize, Serialize};
use std::fs;
use tauri::AppHandle;
use tauri::Manager;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServerOs {
    Linux,
    Windows,
}

impl Default for ServerOs {
    fn default() -> Self {
        ServerOs::Linux
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionMethod {
    Ssh,
    Boundary,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ServerService {
    pub name: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub os: ServerOs,
    pub connection_method: ConnectionMethod,
    pub ssh_port: u16,
    pub ssh_user: String,
    #[serde(default)]
    pub boundary_target_id: String,
    /// Listen port for Boundary tunnel (boundary connect -listen-port). Used when connection_method is Boundary.
    #[serde(default)]
    pub boundary_listen_port: u16,
    #[serde(default)]
    pub services: Vec<ServerService>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ServersFile {
    #[serde(default)]
    servers: Vec<Server>,
}

fn servers_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("app config dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
    Ok(dir.join("servers.json"))
}

pub fn load_servers(app: &AppHandle) -> Result<Vec<Server>, String> {
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
    let path = servers_path(app)?;
    let file = ServersFile {
        servers: servers.to_vec(),
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| format!("encode servers: {e}"))?;
    fs::write(path, json).map_err(|e| format!("write servers: {e}"))
}

pub fn new_server_id() -> String {
    format!("srv-{}", Uuid::new_v4())
}
