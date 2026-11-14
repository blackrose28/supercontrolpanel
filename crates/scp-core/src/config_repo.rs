//! Profile-scoped configuration, server inventory, and local stdio policy repositories.
use crate::backup::io;
use crate::error::CoreError;
use crate::gate::MaintenanceGate;
use crate::models::{
    decode_sync_settings, default_app_config, AppConfig, AppConfigForDisk, Server, ServersFile,
    StdioPolicy, SyncSettings, KEYRING_BOUNDARY_OIDC_OTP_SECRET, KEYRING_BOUNDARY_OIDC_PASSWORD,
    KEYRING_BOUNDARY_PASSWORD, KEYRING_OPENVPN_OTP_SECRET, KEYRING_OPENVPN_PASSWORD,
};
use crate::profile::ProfilePaths;
use crate::secrets::{KeyringBackend, NamespacedSecretStore};
use zeroize::Zeroizing;

pub fn load_servers(
    paths: &ProfilePaths,
    gate: &MaintenanceGate,
) -> Result<Vec<Server>, CoreError> {
    let _guard = gate.enter()?;
    let Some(bytes) = io::read_bounded(&paths.servers_path).map_err(|e| {
        CoreError::operation_failed("servers_read_failed", e, false, "fix_configuration")
    })?
    else {
        return Ok(Vec::new());
    };
    let parsed: ServersFile = serde_json::from_slice(&bytes).map_err(|e| {
        CoreError::operation_failed(
            "servers_corrupted",
            format!("Invalid servers.json: {e}"),
            false,
            "fix_configuration",
        )
    })?;
    Ok(parsed.servers)
}

pub fn load_sync_settings(
    paths: &ProfilePaths,
    gate: &MaintenanceGate,
) -> Result<SyncSettings, CoreError> {
    let _guard = gate.enter()?;
    let Some(bytes) = io::read_bounded(&paths.sync_local_path).map_err(|e| {
        CoreError::operation_failed("sync_settings_read_failed", e, false, "fix_configuration")
    })?
    else {
        return Ok(SyncSettings::default());
    };
    let raw = std::str::from_utf8(&bytes).map_err(|_| {
        CoreError::operation_failed(
            "sync_settings_corrupted",
            "Invalid UTF-8 in sync-local.json",
            false,
            "fix_configuration",
        )
    })?;
    decode_sync_settings(raw).map_err(|e| {
        CoreError::operation_failed("sync_settings_corrupted", e, false, "fix_configuration")
    })
}

pub fn load_stdio_policy(
    paths: &ProfilePaths,
    gate: &MaintenanceGate,
) -> Result<StdioPolicy, CoreError> {
    let _guard = gate.enter()?;
    let Some(bytes) = io::read_bounded(&paths.stdio_policy_path).map_err(|e| {
        CoreError::policy_denied("stdio_policy_read_failed", e, "fix_configuration")
    })?
    else {
        return Ok(StdioPolicy::default());
    };
    serde_json::from_slice(&bytes).map_err(|e| {
        CoreError::policy_denied(
            "stdio_policy_corrupted",
            format!("Invalid stdio-policy.json: {e}"),
            "fix_configuration",
        )
    })
}

pub fn load_app_config_and_migrate<B: KeyringBackend>(
    paths: &ProfilePaths,
    gate: &MaintenanceGate,
    secrets: &mut NamespacedSecretStore<B>,
) -> Result<AppConfig, CoreError> {
    let _guard = gate.enter()?;
    let mut config = match io::read_bounded(&paths.config_path).map_err(|e| {
        CoreError::operation_failed("config_read_failed", e, false, "fix_configuration")
    })? {
        None => default_app_config(),
        Some(bytes) => {
            let bytes = Zeroizing::new(bytes);
            serde_json::from_slice::<AppConfig>(&bytes).map_err(|e| {
                CoreError::operation_failed(
                    "config_corrupted",
                    format!("Invalid config.json: {e}"),
                    false,
                    "fix_configuration",
                )
            })?
        }
    };

    let mut sync_settings = load_sync_settings(paths, gate)?;
    let (mut needs_disk_rewrite, needs_sync_rewrite) =
        merge_local_paths(&mut config, &mut sync_settings);

    if has_plaintext_legacy_secrets(&config) && migrate_plaintext_secrets(&mut config, secrets)? {
        needs_disk_rewrite = true;
    }

    if needs_sync_rewrite {
        gate.changed();
        let encoded = serde_json::to_vec_pretty(&sync_settings).map_err(|e| {
            CoreError::operation_failed(
                "sync_settings_encode_failed",
                e.to_string(),
                false,
                "fix_configuration",
            )
        })?;
        io::atomic_write(&paths.sync_local_path, &encoded, true).map_err(|e| {
            CoreError::operation_failed("sync_settings_write_failed", e, false, "fix_configuration")
        })?;
    }

    if needs_disk_rewrite {
        gate.changed();
        let disk_cfg = AppConfigForDisk::from(&config);
        let encoded = serde_json::to_vec_pretty(&disk_cfg).map_err(|e| {
            CoreError::operation_failed(
                "config_encode_failed",
                e.to_string(),
                false,
                "fix_configuration",
            )
        })?;
        io::atomic_write(&paths.config_path, &encoded, true).map_err(|e| {
            CoreError::operation_failed("config_write_failed", e, false, "fix_configuration")
        })?;
    }

    Ok(config)
}

fn merge_local_paths(config: &mut AppConfig, sync: &mut SyncSettings) -> (bool, bool) {
    let mut rewrite_cfg = false;
    let mut rewrite_sync = false;
    for (sync_slot, cfg_val) in [
        (
            &mut sync.openvpn_executable_path,
            &config.openvpn.executable_path,
        ),
        (&mut sync.openvpn_ovpn_path, &config.openvpn.ovpn_path),
        (&mut sync.boundary_cli_path, &config.boundary.cli_path),
    ] {
        if sync_slot.trim().is_empty() && !cfg_val.trim().is_empty() {
            *sync_slot = cfg_val.clone();
            rewrite_cfg = true;
            rewrite_sync = true;
        }
    }
    config.openvpn.executable_path = sync.openvpn_executable_path.clone();
    config.openvpn.ovpn_path = sync.openvpn_ovpn_path.clone();
    config.boundary.cli_path = sync.boundary_cli_path.clone();
    if config.openvpn.command.trim().is_empty() && !config.openvpn.executable_path.trim().is_empty()
    {
        config.openvpn.command = config.openvpn.executable_path.clone();
    }
    (rewrite_cfg, rewrite_sync)
}

fn has_plaintext_legacy_secrets(config: &AppConfig) -> bool {
    !config.openvpn.password.is_empty()
        || !config.openvpn.otp_secret.is_empty()
        || !config.boundary.password.is_empty()
        || !config.boundary.oidc_password.is_empty()
        || !config.boundary.oidc_otp_secret.is_empty()
}

fn migrate_plaintext_secrets<B: KeyringBackend>(
    config: &mut AppConfig,
    secrets: &mut NamespacedSecretStore<B>,
) -> Result<bool, CoreError> {
    let mut dirty = false;
    let slots = [
        (KEYRING_OPENVPN_PASSWORD, &mut config.openvpn.password),
        (KEYRING_OPENVPN_OTP_SECRET, &mut config.openvpn.otp_secret),
        (KEYRING_BOUNDARY_PASSWORD, &mut config.boundary.password),
        (
            KEYRING_BOUNDARY_OIDC_PASSWORD,
            &mut config.boundary.oidc_password,
        ),
        (
            KEYRING_BOUNDARY_OIDC_OTP_SECRET,
            &mut config.boundary.oidc_otp_secret,
        ),
    ];
    for (account, field) in slots {
        if field.is_empty() {
            continue;
        }
        if secrets.get(account)?.is_none() {
            secrets.set(account, field)?;
        }
        field.clear();
        dirty = true;
    }
    Ok(dirty)
}
