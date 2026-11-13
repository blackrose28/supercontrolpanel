//! Pure backup manifest primitives; no archive, filesystem, keyring or restore ownership.
//! External references may contain sensitive paths and are not public IPC/MCP DTOs.

use serde::{Deserialize, Serialize};

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
