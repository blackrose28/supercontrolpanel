// Preserve the desktop import path while sharing the exact schema with headless hosts.
pub use scp_core::config::*;

#[cfg(target_os = "windows")]
use serde::{Deserialize, Serialize};

#[cfg(target_os = "windows")]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnLaunchConfig {
    pub executable_path: String,
    pub args: String,
    pub username: String,
    pub password: String,
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    pub terminate_open_vpn_when_helper_exits: bool,
    #[serde(default)]
    pub auto_reconnect_enabled: bool,
    #[serde(default = "default_openvpn_reconnect_base_seconds")]
    pub auto_reconnect_base_seconds: u64,
    #[serde(default = "default_openvpn_reconnect_max_seconds")]
    pub auto_reconnect_max_seconds: u64,
}

#[cfg(target_os = "windows")]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperStatusFile {
    pub state: String,
    pub message: String,
    pub updated_at_ms: Option<u64>,
}
