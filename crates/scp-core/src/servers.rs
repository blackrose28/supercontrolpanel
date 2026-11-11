//! Server inventory schema shared with the desktop; reads never create or migrate files.

use serde::{Deserialize, Serialize};
use std::{fs, io::ErrorKind, path::Path};

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

/// Read only the selected profile's inventory, without keyring or host initialization.
pub fn read_servers(config_dir: &Path) -> Result<Vec<Server>, String> {
    let raw = match fs::read_to_string(config_dir.join("servers.json")) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("read servers: {error}")),
    };
    let file: ServersFile = serde_json::from_str(&raw).map_err(|error| {
        // Serde's detail can include field values. Keep diagnostics free of file contents.
        format!(
            "parse servers: invalid inventory at line {}, column {}",
            error.line(),
            error.column()
        )
    })?;
    Ok(file.servers)
}
