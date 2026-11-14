use crate::SpikeError;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryAuthKind {
    Password,
    BrowserOidc,
    AutomatedOidc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundaryTokenMode {
    InjectedEnv(String),
    NamespacedKeyring { namespace: String },
    NoKeyring,
}

pub fn check_auth_capability(kind: BoundaryAuthKind) -> Result<(), SpikeError> {
    match kind {
        BoundaryAuthKind::Password => Ok(()),
        BoundaryAuthKind::AutomatedOidc => Err(SpikeError::new(
            "CAPABILITY_UNAVAILABLE",
            "webview_oidc_desktop_only",
            "Automated OIDC requires the desktop hidden-webview runtime and is unavailable in headless CLI",
            4,
        )),
        BoundaryAuthKind::BrowserOidc => Err(SpikeError::new(
            "CAPABILITY_UNAVAILABLE",
            "browser_oidc_unverified",
            "Browser OIDC requires an interactive host browser session and remains gated",
            4,
        )),
    }
}

pub fn build_get_token_command(
    cli_path: &Path,
    addr: Option<&str>,
    mode: &BoundaryTokenMode,
) -> Command {
    let mut cmd = Command::new(cli_path);
    cmd.args(["config", "get-token"])
        .stdin(Stdio::null())
        .env("BOUNDARY_SKIP_CACHE_DAEMON", "true")
        .env_remove("BOUNDARY_TOKEN")
        .env_remove("BOUNDARY_TOKEN_NAME")
        .env_remove("BOUNDARY_KEYRING_TYPE");
    if let Some(addr_val) = addr {
        cmd.env("BOUNDARY_ADDR", addr_val);
    }
    match mode {
        BoundaryTokenMode::InjectedEnv(token) => {
            cmd.env("BOUNDARY_KEYRING_TYPE", "none")
                .env("BOUNDARY_TOKEN", token);
        }
        BoundaryTokenMode::NamespacedKeyring { namespace } => {
            cmd.env("BOUNDARY_TOKEN_NAME", format!("scpctl-{namespace}"));
        }
        BoundaryTokenMode::NoKeyring => {
            cmd.env("BOUNDARY_KEYRING_TYPE", "none");
        }
    }
    cmd
}

pub fn run_get_token(
    cli_path: &Path,
    addr: Option<&str>,
    mode: &BoundaryTokenMode,
) -> Result<String, SpikeError> {
    let output = build_get_token_command(cli_path, addr, mode)
        .output()
        .map_err(|e| {
            SpikeError::new(
                "CAPABILITY_UNAVAILABLE",
                "boundary_cli_unavailable",
                format!("Failed to launch {}: {e}", cli_path.display()),
                4,
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let lower = stderr.to_ascii_lowercase();
        let reason = if matches!(mode, BoundaryTokenMode::NoKeyring)
            || lower.contains("could not be found in the keyring")
            || lower.contains("no token")
        {
            "boundary_token_missing"
        } else if lower.contains("failed to get token from keyring") || lower.contains("keyring") {
            "boundary_keyring_error"
        } else {
            "boundary_token_missing"
        };
        return Err(SpikeError::new(
            "CAPABILITY_UNAVAILABLE",
            reason,
            if stderr.is_empty() {
                format!(
                    "boundary config get-token exited {:?}",
                    output.status.code()
                )
            } else {
                stderr
            },
            4,
        ));
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(SpikeError::new(
            "CAPABILITY_UNAVAILABLE",
            "boundary_token_missing",
            "boundary config get-token returned an empty token",
            4,
        ));
    }
    Ok(token)
}
