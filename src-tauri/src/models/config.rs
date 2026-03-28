use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub app: AppRuntimeConfig,
    #[serde(default)]
    pub openvpn: OpenVpnConfig,
    #[serde(default)]
    pub boundary: BoundaryConfig,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoundaryAuthKind {
    Password,
    #[default]
    Oidc,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoundaryOidcLoginMode {
    #[default]
    Browser,
    Automated,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryConfig {
    #[serde(default)]
    pub cli_path: String,
    #[serde(default)]
    pub addr: String,
    #[serde(default)]
    pub auth_method_id: String,
    #[serde(default)]
    pub login_name: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub auth_kind: BoundaryAuthKind,
    #[serde(default)]
    pub oidc_login_mode: BoundaryOidcLoginMode,
    #[serde(default)]
    pub oidc_username: String,
    #[serde(default)]
    pub oidc_password: String,
    #[serde(default)]
    pub oidc_otp_secret: String,
    /// Passed to SSH when using boundary connect ssh (after --). e.g. "-o" "MACs=hmac-sha2-256" to fix MAC errors.
    #[serde(default)]
    pub connect_ssh_options: String,
}

impl Default for BoundaryConfig {
    fn default() -> Self {
        Self {
            cli_path: String::new(),
            addr: String::new(),
            auth_method_id: String::new(),
            login_name: String::new(),
            password: String::new(),
            auth_kind: BoundaryAuthKind::Oidc,
            oidc_login_mode: BoundaryOidcLoginMode::Browser,
            oidc_username: String::new(),
            oidc_password: String::new(),
            oidc_otp_secret: String::new(),
            connect_ssh_options: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryConfigForDisk {
    #[serde(default)]
    pub addr: String,
    #[serde(default)]
    pub auth_method_id: String,
    #[serde(default)]
    pub login_name: String,
    #[serde(default)]
    pub auth_kind: BoundaryAuthKind,
    #[serde(default)]
    pub oidc_login_mode: BoundaryOidcLoginMode,
    #[serde(default)]
    pub oidc_username: String,
    #[serde(default)]
    pub connect_ssh_options: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppRuntimeConfig {
    pub default_terminal_cols: u16,
    pub default_terminal_rows: u16,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub executable_path: String,
    #[serde(default)]
    pub ovpn_path: String,
    #[serde(default)]
    pub extra_args: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub otp_secret: String,
    /// Windows: if true, closing/killing the elevated helper also terminates OpenVPN (job object).
    /// Field name uses `open_vpn` so camelCase matches UI (`terminateOpenVpnWhenHelperExits`).
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    pub terminate_open_vpn_when_helper_exits: bool,
}

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
}

#[cfg(target_os = "windows")]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperStatusFile {
    pub state: String,
    pub message: String,
    pub updated_at_ms: Option<u64>,
}

/// Written to `config.json` only (no secrets).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnConfigForDisk {
    pub command: String,
    pub extra_args: String,
    pub username: String,
    #[serde(default, alias = "terminateOpenvpnWhenHelperExits")]
    pub terminate_open_vpn_when_helper_exits: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfigForDisk {
    pub app: AppRuntimeConfig,
    pub openvpn: OpenVpnConfigForDisk,
    #[serde(default)]
    pub boundary: BoundaryConfigForDisk,
}

impl From<&AppConfig> for AppConfigForDisk {
    fn from(c: &AppConfig) -> Self {
        Self {
            app: c.app.clone(),
            openvpn: OpenVpnConfigForDisk {
                command: String::new(),
                extra_args: c.openvpn.extra_args.clone(),
                username: c.openvpn.username.clone(),
                terminate_open_vpn_when_helper_exits: c.openvpn.terminate_open_vpn_when_helper_exits,
            },
            boundary: BoundaryConfigForDisk {
                addr: c.boundary.addr.clone(),
                auth_method_id: c.boundary.auth_method_id.clone(),
                login_name: c.boundary.login_name.clone(),
                auth_kind: c.boundary.auth_kind,
                oidc_login_mode: c.boundary.oidc_login_mode,
                oidc_username: c.boundary.oidc_username.clone(),
                connect_ssh_options: c.boundary.connect_ssh_options.clone(),
            },
        }
    }
}
