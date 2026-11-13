//! Server inventory schema shared with the desktop; reads never create or migrate files.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServerOs {
    #[default]
    Linux,
    Windows,
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

#[derive(Deserialize, Default)]
struct ServersFile {
    #[serde(default)]
    servers: Vec<Server>,
}

/// Read only the selected CLI profile's inventory, without keyring or host initialization.
/// Recovery evidence or inspection failure denies before reading settings.
pub fn read_servers(config_dir: &Path) -> Result<Vec<Server>, String> {
    crate::profile::ensure_readable(config_dir)?;
    let file: Option<ServersFile> =
        crate::config_file::read_json(&config_dir.join("servers.json"), "servers", "inventory")?;
    Ok(file.unwrap_or_default().servers)
}
