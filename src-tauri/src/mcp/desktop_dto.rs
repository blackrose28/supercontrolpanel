//! Trusted desktop conversions; public contract modules do not import Tauri or runtime models.
use super::public_dto::*;
use crate::{
    models::{
        AppConfig, BoundaryAuthKind, BoundaryOidcLoginMode, FunctionStatus, ScpTransferEntry,
        ScpTransferState,
    },
    services::SyncSettings,
};

impl From<&AppConfig> for SafeAppConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            app: SafeAppRuntime {
                default_terminal_cols: config.app.default_terminal_cols,
                default_terminal_rows: config.app.default_terminal_rows,
            },
            openvpn: SafeOpenVpn {
                username: config.openvpn.username.clone(),
                terminate_open_vpn_when_helper_exits: config
                    .openvpn
                    .terminate_open_vpn_when_helper_exits,
                auto_reconnect_enabled: config.openvpn.auto_reconnect_enabled,
                auto_reconnect_base_seconds: config.openvpn.auto_reconnect_base_seconds,
                auto_reconnect_max_seconds: config.openvpn.auto_reconnect_max_seconds,
            },
            boundary: SafeBoundary {
                addr: config.boundary.addr.clone(),
                auth_method_id: config.boundary.auth_method_id.clone(),
                login_name: config.boundary.login_name.clone(),
                auth_kind: match config.boundary.auth_kind {
                    BoundaryAuthKind::Password => "password",
                    BoundaryAuthKind::Oidc => "oidc",
                }
                .into(),
                oidc_login_mode: match config.boundary.oidc_login_mode {
                    BoundaryOidcLoginMode::Browser => "browser",
                    BoundaryOidcLoginMode::Automated => "automated",
                }
                .into(),
                oidc_username: config.boundary.oidc_username.clone(),
            },
        }
    }
}

impl From<&SyncSettings> for SafeSyncSettings {
    fn from(settings: &SyncSettings) -> Self {
        Self {
            repo_root: settings.repo_root.clone(),
            openvpn_executable_path: settings.openvpn_executable_path.clone(),
            openvpn_ovpn_path: settings.openvpn_ovpn_path.clone(),
            boundary_cli_path: settings.boundary_cli_path.clone(),
            caddy_folder: settings.caddy_folder.clone(),
            mcp_bind_address: settings.mcp_bind_address.clone(),
            mcp_sensitive_action_policy: settings.mcp_sensitive_action_policy.clone(),
            cc_server_url: settings.cc_server_url.clone(),
            cc_auth_mode: settings.cc_auth_mode.clone(),
            cc_device_id: settings.cc_device_id.clone(),
            cc_device_name: settings.cc_device_name.clone(),
            mcp_enabled: settings.mcp_enabled,
            mcp_port: settings.mcp_port,
            mcp_allowed_tool_categories: settings.mcp_allowed_tool_categories.clone(),
            cc_enabled: settings.cc_enabled,
            first_run_completed: settings.first_run_completed,
        }
    }
}

impl From<&FunctionStatus> for SafeFunctionStatus {
    fn from(status: &FunctionStatus) -> Self {
        Self {
            function_id: status.function_id.clone(),
            state: status.state.clone(),
            message: String::new(),
            boundary_token_source: status.boundary_token_source.clone(),
            session_id: status.session_id.clone(),
            updated_at_ms: status.updated_at_ms,
        }
    }
}

impl From<&ScpTransferEntry> for SafeTransfer {
    fn from(entry: &ScpTransferEntry) -> Self {
        Self {
            id: entry.id.clone(),
            title: String::new(),
            detail: String::new(),
            state: match entry.state {
                ScpTransferState::Pending => "pending",
                ScpTransferState::Success => "success",
                ScpTransferState::Error => "error",
            }
            .into(),
            created_at_ms: entry.created_at_ms,
            updated_at_ms: entry.updated_at_ms,
            progress_percent: entry.progress_percent,
            source: None,
            destination: None,
            session_id: entry.session_id.clone(),
        }
    }
}
