use std::sync::OnceLock;

use reqwest::blocking::Client;
use scp_core::openvpn_ops::generate_openvpn_password_with_offset;
#[cfg_attr(target_os = "windows", allow(unused_imports))]
pub use scp_core::openvpn_ops::{
    build_openvpn_command_unix, normalized_openvpn_config, quote_posix_arg,
};

#[cfg(target_os = "windows")]
use crate::models::OpenVpnConfig;

#[cfg(target_os = "windows")]
pub fn quote_windows_arg(raw: &str) -> String {
    if raw.contains(' ') || raw.contains('\t') || raw.contains('"') {
        format!("\"{}\"", raw.replace('"', "\\\""))
    } else {
        raw.to_string()
    }
}

#[cfg(target_os = "windows")]
pub fn build_openvpn_args_windows(config: &OpenVpnConfig) -> Result<String, String> {
    if config.ovpn_path.trim().is_empty() {
        return Err("ovpn file path cannot be empty".to_string());
    }
    let mut args = format!("--config {}", quote_windows_arg(config.ovpn_path.trim()));
    if !config.extra_args.trim().is_empty() {
        args.push(' ');
        args.push_str(config.extra_args.trim());
    }
    Ok(args)
}

/// Cached clock offset (seconds) = remote_epoch − local_epoch.
/// Positive means local clock is behind; negative means ahead.
static NTP_OFFSET: OnceLock<i64> = OnceLock::new();

pub fn fetch_time_offset() -> i64 {
    *NTP_OFFSET.get_or_init(|| {
        let offset = (|| -> Result<i64, String> {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|e| format!("{e}"))?;
            let resp = client
                .head("https://www.google.com")
                .send()
                .map_err(|e| format!("{e}"))?;
            let date_str = resp
                .headers()
                .get("date")
                .and_then(|v| v.to_str().ok())
                .ok_or("no Date header")?;
            let remote = httpdate::parse_http_date(date_str).map_err(|e| format!("{e}"))?;
            let remote_epoch = remote
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| format!("{e}"))?
                .as_secs() as i64;
            let local_epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| format!("{e}"))?
                .as_secs() as i64;
            Ok(remote_epoch - local_epoch)
        })();
        offset.unwrap_or(0)
    })
}

pub fn generate_totp_code(otp_secret: &str) -> Result<String, String> {
    scp_core::openvpn_ops::generate_totp_code_with_offset(otp_secret, fetch_time_offset())
}

pub fn generate_openvpn_password(password: &str, otp_secret: &str) -> Result<String, String> {
    let offset = fetch_time_offset();
    generate_openvpn_password_with_offset(password, otp_secret, offset)
}

pub fn boundary_http_client() -> Result<Client, String> {
    Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(20))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}
