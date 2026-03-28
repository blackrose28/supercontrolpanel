use std::sync::OnceLock;

use reqwest::blocking::Client;
use totp_rs::{Algorithm, Secret, TOTP};

use crate::models::OpenVpnConfig;

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn quote_posix_arg(raw: &str) -> String {
    if raw.is_empty() {
        "''".to_string()
    } else if raw
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_./:".contains(&b))
    {
        raw.to_string()
    } else {
        format!("'{}'", raw.replace('\'', "'\"'\"'"))
    }
}

#[cfg(target_os = "windows")]
pub fn quote_windows_arg(raw: &str) -> String {
    if raw.contains(' ') || raw.contains('\t') || raw.contains('"') {
        format!("\"{}\"", raw.replace('"', "\\\""))
    } else {
        raw.to_string()
    }
}

pub fn normalized_openvpn_config(config: &OpenVpnConfig) -> OpenVpnConfig {
    let mut cfg = config.clone();
    if cfg.executable_path.trim().is_empty() && !cfg.command.trim().is_empty() {
        cfg.executable_path = cfg.command.trim().to_string();
    }
    cfg
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

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn build_openvpn_command_unix(config: &OpenVpnConfig) -> Result<String, String> {
    if config.executable_path.trim().is_empty() {
        return Err("openvpn executable path cannot be empty".to_string());
    }
    if config.ovpn_path.trim().is_empty() {
        return Err("ovpn file path cannot be empty".to_string());
    }
    let mut command = format!(
        "sudo {} --config {}",
        quote_posix_arg(config.executable_path.trim()),
        quote_posix_arg(config.ovpn_path.trim())
    );
    if !config.extra_args.trim().is_empty() {
        command.push(' ');
        command.push_str(config.extra_args.trim());
    }
    Ok(command)
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
    let secret = otp_secret.trim();
    if secret.is_empty() {
        return Err("OTP secret is empty".to_string());
    }
    let secret_bytes = Secret::Encoded(secret.to_string())
        .to_bytes()
        .map_err(|e| format!("invalid OTP secret: {e}"))?;
    let local_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("system time error: {e}"))?
        .as_secs();
    let offset = fetch_time_offset();
    let corrected = (local_epoch as i64 + offset) as u64;
    let totp = TOTP::new(Algorithm::SHA1, 6, 1, 30, secret_bytes)
        .map_err(|e| format!("failed to initialize OTP generator: {e}"))?;
    Ok(totp.generate(corrected))
}

pub fn generate_openvpn_password(password: &str, otp_secret: &str) -> Result<String, String> {
    let secret = otp_secret.trim();
    if secret.is_empty() {
        return Ok(password.to_string());
    }
    let otp = generate_totp_code(secret)?;
    Ok(format!("{otp}{password}"))
}

pub fn boundary_http_client() -> Result<Client, String> {
    Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(20))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}
