use crate::SpikeError;
use crate::profile_lock::{OwnerLease, acquire_exclusive_lock_file};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub struct VpnPreflightConfig {
    pub sudo_path: PathBuf,
    pub openvpn_path: PathBuf,
    pub ovpn_path: PathBuf,
    pub vpn_lock_path: PathBuf,
    pub terminate_when_helper_exits: bool,
}

#[derive(Debug)]
pub struct VpnLease {
    _lease: OwnerLease,
}

pub fn preflight_and_acquire_vpn(cfg: &VpnPreflightConfig) -> Result<VpnLease, SpikeError> {
    if cfg.terminate_when_helper_exits {
        return Err(SpikeError::new(
            "INVALID_CONFIG",
            "incompatible_vpn_ownership",
            "terminateOpenVpnWhenHelperExits is incompatible with headless/stdio CLI ownership",
            2,
        ));
    }

    if !cfg.openvpn_path.is_file() || !cfg.ovpn_path.is_file() {
        return Err(SpikeError::new(
            "PRECONDITION_FAILED",
            "vpn_prerequisite_missing",
            format!(
                "OpenVPN binary ({}) and profile ({}) must exist before acquisition",
                cfg.openvpn_path.display(),
                cfg.ovpn_path.display()
            ),
            3,
        ));
    }

    #[cfg(unix)]
    verify_noninteractive_sudo(&cfg.sudo_path)?;

    let lease = acquire_exclusive_lock_file(
        &cfg.vpn_lock_path,
        "cli-vpn",
        "openvpn",
        "vpn_owner_conflict",
    )?;
    Ok(VpnLease { _lease: lease })
}

#[cfg(unix)]
fn verify_noninteractive_sudo(sudo_path: &std::path::Path) -> Result<(), SpikeError> {
    if !sudo_path.is_file() {
        return Err(SpikeError::new(
            "CAPABILITY_UNAVAILABLE",
            "elevation_unavailable",
            format!("Elevation helper missing at {}", sudo_path.display()),
            4,
        ));
    }
    let output = Command::new(sudo_path)
        .arg("-n")
        .arg("true")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            SpikeError::new(
                "CAPABILITY_UNAVAILABLE",
                "elevation_unavailable",
                format!("Failed to execute {} -n true: {e}", sudo_path.display()),
                4,
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(SpikeError::new(
            "CAPABILITY_UNAVAILABLE",
            "elevation_interaction_required",
            format!(
                "Noninteractive sudo failed (exit {:?}): {stderr}",
                output.status.code()
            ),
            4,
        ));
    }
    Ok(())
}
