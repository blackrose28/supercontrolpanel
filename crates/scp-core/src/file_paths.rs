//! Shared local and remote file-path validation helpers for SCP and SFTP operations.
use crate::error::CoreError;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use std::path::{Component, Path, Prefix};

/// Structured path validation failure with a stable machine reason code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOpPathError {
    pub reason_code: &'static str,
    pub detail: String,
}

impl FileOpPathError {
    /// Builds a path error with the provided reason code and user-facing detail.
    pub fn new(reason_code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason_code,
            detail: detail.into(),
        }
    }

    /// Formats the error in desktop legacy string form.
    pub fn into_string(self) -> String {
        format!("{}: {}", self.reason_code, self.detail)
    }

    /// Converts this validation error into a headless core `INVALID_ARGUMENTS` error.
    pub fn into_core_error(self) -> CoreError {
        CoreError::invalid_arguments(self.reason_code, self.detail)
    }
}

/// Validates a local source path for upload operations.
pub fn validate_local_upload_source(path: &str) -> Result<String, FileOpPathError> {
    let path = parse_local_path(
        path,
        "invalid_source_path",
        "Local upload source path is required.",
    )?;
    if !path.exists() {
        return Err(FileOpPathError::new(
            "invalid_source_path",
            "Local upload source path does not exist.",
        ));
    }
    Ok(path.to_string_lossy().to_string())
}

/// Validates a local destination path for download operations.
pub fn validate_local_download_target(path: &str) -> Result<String, FileOpPathError> {
    let path = parse_local_path(
        path,
        "invalid_target_path",
        "Local download target path is required.",
    )?;
    if path.parent().is_none() {
        return Err(FileOpPathError::new(
            "invalid_target_path",
            "Local download target cannot be the filesystem root.",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        FileOpPathError::new(
            "invalid_target_path",
            "Local download target must have an existing parent directory.",
        )
    })?;
    if !parent.exists() || !parent.is_dir() {
        return Err(FileOpPathError::new(
            "invalid_target_path",
            "Local download target parent directory does not exist.",
        ));
    }
    Ok(path.to_string_lossy().to_string())
}

/// Normalizes a remote POSIX transfer path and rejects parent-directory escapes.
pub fn normalize_remote_transfer_path(
    path: &str,
    fallback: Option<&str>,
    reason_code: &'static str,
    allow_root: bool,
) -> Result<String, FileOpPathError> {
    let absolute = resolve_remote_raw(path, fallback, reason_code)?;
    let mut parts = Vec::new();
    for segment in absolute.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            if parts.pop().is_none() {
                return Err(FileOpPathError::new(
                    reason_code,
                    "Remote path cannot escape above the remote root.",
                ));
            }
            continue;
        }
        parts.push(segment);
    }

    let normalized = if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    };
    if !allow_root && normalized == "/" {
        return Err(FileOpPathError::new(
            reason_code,
            "Remote path cannot be the remote root.",
        ));
    }
    Ok(normalized)
}

fn resolve_remote_raw(
    path: &str,
    fallback: Option<&str>,
    reason_code: &'static str,
) -> Result<String, FileOpPathError> {
    let trimmed = path.trim();
    let raw = if trimmed.is_empty() {
        fallback.unwrap_or("")
    } else {
        trimmed
    };
    if raw.trim().is_empty() {
        return Err(FileOpPathError::new(
            reason_code,
            "Remote path is required.",
        ));
    }
    if contains_forbidden_chars(raw) {
        return Err(FileOpPathError::new(
            reason_code,
            "Remote path contains unsupported control characters.",
        ));
    }
    let normalized_slashes = raw.replace('\\', "/");
    if normalized_slashes.starts_with('/') {
        return Ok(normalized_slashes);
    }
    if let Some(base) = fallback {
        return Ok(join_remote_base(base, &normalized_slashes));
    }
    Err(FileOpPathError::new(
        reason_code,
        "Remote path must be absolute.",
    ))
}

fn parse_local_path(
    raw: &str,
    reason_code: &'static str,
    empty_message: &'static str,
) -> Result<PathBuf, FileOpPathError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FileOpPathError::new(reason_code, empty_message));
    }
    if contains_forbidden_chars(trimmed) {
        return Err(FileOpPathError::new(
            reason_code,
            "Path contains unsupported control characters.",
        ));
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(FileOpPathError::new(reason_code, "Path must be absolute."));
    }
    #[cfg(target_os = "windows")]
    validate_windows_components(&path, reason_code)?;
    Ok(path)
}

fn contains_forbidden_chars(value: &str) -> bool {
    value.chars().any(|ch| ch == '\0' || ch.is_control())
}

fn join_remote_base(base: &str, child: &str) -> String {
    let clean_base = if base.trim().is_empty() {
        "/"
    } else {
        base.trim()
    };
    if clean_base == "/" {
        format!("/{}", child.trim_start_matches('/'))
    } else {
        format!(
            "{}/{}",
            clean_base.trim_end_matches('/'),
            child.trim_start_matches('/')
        )
    }
}

#[cfg(target_os = "windows")]
fn validate_windows_components(
    path: &Path,
    reason_code: &'static str,
) -> Result<(), FileOpPathError> {
    for component in path.components() {
        if let Component::Normal(value) = component {
            validate_windows_normal_segment(&value.to_string_lossy(), reason_code)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn validate_windows_normal_segment(
    segment: &str,
    reason_code: &'static str,
) -> Result<(), FileOpPathError> {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if segment.ends_with(' ') || segment.ends_with('.') {
        return Err(FileOpPathError::new(
            reason_code,
            "Windows path segments cannot end with a space or period.",
        ));
    }
    let upper = segment
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if RESERVED.contains(&upper.as_str()) {
        return Err(FileOpPathError::new(
            reason_code,
            "Windows path uses a reserved device name.",
        ));
    }
    if segment
        .chars()
        .any(|ch| matches!(ch, '<' | '>' | '"' | '|' | '?' | '*'))
    {
        return Err(FileOpPathError::new(
            reason_code,
            "Windows path contains invalid characters.",
        ));
    }
    Ok(())
}
