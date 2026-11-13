//! Pure backup manifest primitives; no archive, filesystem, keyring or restore ownership.
//! External references may contain sensitive paths and are not public IPC/MCP DTOs.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// A reference only: never file contents, path normalization or an existence check.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ExternalFile {
    pub kind: String,
    pub path: String,
}

/// Validate the desktop v1 manifest prefix in its original order: byte/count limits,
/// then schema support. Call before validating sections, server IDs or credentials.
/// Success does not establish archive validity or authorize a restore.
pub fn validate_manifest(
    schema: u32,
    app_version: &str,
    platform: &str,
    external_files: &[ExternalFile],
) -> Result<(), String> {
    if app_version.len() > 64
        || platform.len() > 64
        || external_files.len() > 10000
        || external_files
            .iter()
            .any(|file| file.kind.len() > 128 || file.path.len() > 4096)
    {
        return Err("Backup manifest metadata exceeds size limits".into());
    }
    if schema != 1 {
        return Err("Unsupported settings schema; update the application".into());
    }
    Ok(())
}

/// Parse a present desktop backup servers section using the full shared Server schema.
/// `None` means an absent file; present null/missing/non-array lists are invalid.
/// Returns sorted unique IDs. This does not validate credentials or authorize restore;
/// desktop IDs deliberately permit `:`, unlike CLI credential account validation.
pub fn server_ids(value: Option<&serde_json::Value>) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(vec![]);
    };
    let servers = value
        .get("servers")
        .and_then(Value::as_array)
        .ok_or("Invalid backup server list")?;
    if servers.len() > 10000 {
        return Err("Too many servers in backup".into());
    }
    let mut ids = BTreeSet::new();
    for value in servers {
        let server: crate::servers::Server =
            serde_json::from_value(value.clone()).map_err(|_| "Invalid backup server record")?;
        if server.id.is_empty()
            || server.id.len() > 256
            || !server
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
            || !ids.insert(server.id)
        {
            return Err("Invalid or duplicate backup server ID".into());
        }
    }
    Ok(ids.into_iter().collect())
}

/// Desktop v1 settings declarations and validation order.
pub const FILES: [&str; 3] = ["config.json", "servers.json", "sync-local.json"];

/// Validate exact declarations, present objects and typed app/local settings.
/// `None` declares an absent file; `Some(Value::Null)` is an invalid present section.
/// Typed parsing only checks clones: unknown fields and omitted defaults stay intact.
/// Call after manifest validation and before server/credential validation. Success
/// neither validates server records/credentials nor authorizes restore or activation.
pub fn validate_sections(files: &BTreeMap<String, Option<Value>>) -> Result<(), String> {
    if files.len() != FILES.len() || FILES.iter().any(|name| !files.contains_key(*name)) {
        return Err("Backup must declare all settings sections".into());
    }
    for name in FILES {
        if let Some(value) = &files[name] {
            if !value.is_object() {
                return Err("Invalid settings section".into());
            }
            if name == "config.json" {
                serde_json::from_value::<crate::config::AppConfig>(value.clone())
                    .map_err(|_| "Invalid app settings")?;
            }
            if name == "sync-local.json" {
                serde_json::from_value::<crate::local_settings::SyncSettings>(value.clone())
                    .map_err(|_| "Invalid local settings")?;
            }
        }
    }
    Ok(())
}
