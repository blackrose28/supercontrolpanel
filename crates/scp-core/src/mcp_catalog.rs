//! Discovery metadata for the existing desktop MCP tools.
//!
//! This catalog is not an execution allowlist, sensitivity classification or
//! authorization check. Hosts must validate policy and capabilities before dispatch.
//! Listing an operation does not implement or enable it in a standalone host.

use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDescriptor {
    pub name: String,
    pub title: String,
    pub category: String,
    pub read_only: bool,
}

pub fn list_mcp_tools() -> Vec<McpToolDescriptor> {
    vec![
        McpToolDescriptor {
            name: "app_get_overview".to_string(),
            title: "App overview".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "function_status_list".to_string(),
            title: "List function statuses".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "function_status_get".to_string(),
            title: "Get one function status".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "openvpn_get_status".to_string(),
            title: "OpenVPN status".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "boundary_get_status".to_string(),
            title: "Boundary status".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "servers_list".to_string(),
            title: "List servers".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "server_get".to_string(),
            title: "Get server".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "config_get_paths".to_string(),
            title: "Get config paths".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "openvpn_start".to_string(),
            title: "Start OpenVPN".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "openvpn_stop".to_string(),
            title: "Stop OpenVPN".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "boundary_auth_start".to_string(),
            title: "Start Boundary auth".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "boundary_auth_stop".to_string(),
            title: "Stop Boundary auth".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_tunnel_start".to_string(),
            title: "Start server tunnel".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_tunnel_stop".to_string(),
            title: "Stop server tunnel".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_ssh_start".to_string(),
            title: "Start server SSH".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_ssh_stop".to_string(),
            title: "Stop server SSH".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "caddy_start".to_string(),
            title: "Start Caddy".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "caddy_stop".to_string(),
            title: "Stop Caddy".to_string(),
            category: "runtime-control".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_upsert".to_string(),
            title: "Create or update server".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "server_delete".to_string(),
            title: "Delete server".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "config_save".to_string(),
            title: "Save full app config".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "config_save_partial".to_string(),
            title: "Patch app config".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "sync_settings_get".to_string(),
            title: "Get device-local sync settings".to_string(),
            category: "read-only".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "sync_settings_save_partial".to_string(),
            title: "Patch device-local sync settings".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_open".to_string(),
            title: "Open remote browser session".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_close".to_string(),
            title: "Close remote browser session".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_list_children".to_string(),
            title: "List remote browser children".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_stat_path".to_string(),
            title: "Stat remote browser path".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_create_directory".to_string(),
            title: "Create remote directory".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "remote_browser_delete_path".to_string(),
            title: "Delete remote path".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "scp_upload".to_string(),
            title: "Run SCP upload".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "scp_download".to_string(),
            title: "Run SCP download".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "scp_start_upload".to_string(),
            title: "Start PTY-backed SCP upload".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "scp_start_download".to_string(),
            title: "Start PTY-backed SCP download".to_string(),
            category: "file-transfer".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "scp_transfer_list".to_string(),
            title: "List SCP transfers".to_string(),
            category: "file-transfer".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "scp_transfer_get".to_string(),
            title: "Get one SCP transfer".to_string(),
            category: "file-transfer".to_string(),
            read_only: true,
        },
        McpToolDescriptor {
            name: "config_sync_pull".to_string(),
            title: "Pull synced config from repo".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "config_sync_push".to_string(),
            title: "Push synced config to repo".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "config_git_pull".to_string(),
            title: "Run git pull for config repo".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
        McpToolDescriptor {
            name: "config_git_push".to_string(),
            title: "Run git add/commit/push for config repo".to_string(),
            category: "config-mutation".to_string(),
            read_only: false,
        },
    ]
}
