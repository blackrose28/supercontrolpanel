use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use std::path::{Component, Prefix};

#[derive(Debug, Clone)]
pub struct FileOpPathError {
    pub reason_code: &'static str,
    pub detail: String,
}

impl FileOpPathError {
    pub fn new(reason_code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason_code,
            detail: detail.into(),
        }
    }

    pub fn into_string(self) -> String {
        format!("{}: {}", self.reason_code, self.detail)
    }
}

pub fn validate_local_upload_source(path: &str) -> Result<String, FileOpPathError> {
    let path = parse_local_path(path, "invalid_source_path", "Local upload source path is required.")?;
    if !path.exists() {
        return Err(FileOpPathError::new(
            "invalid_source_path",
            "Local upload source path does not exist.",
        ));
    }
    Ok(path.to_string_lossy().to_string())
}

pub fn validate_local_download_target(path: &str) -> Result<String, FileOpPathError> {
    let path = parse_local_path(path, "invalid_target_path", "Local download target path is required.")?;
    if is_root_like(&path) {
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

pub fn normalize_remote_transfer_path(
    path: &str,
    fallback: Option<&str>,
    reason_code: &'static str,
    allow_root: bool,
) -> Result<String, FileOpPathError> {
    let trimmed = path.trim();
    let raw = if trimmed.is_empty() {
        fallback.unwrap_or("")
    } else {
        trimmed
    };
    if raw.trim().is_empty() {
        return Err(FileOpPathError::new(reason_code, "Remote path is required."));
    }
    if contains_forbidden_chars(raw) {
        return Err(FileOpPathError::new(
            reason_code,
            "Remote path contains unsupported control characters.",
        ));
    }
    let normalized_slashes = raw.replace('\\', "/");
    let absolute = if normalized_slashes.starts_with('/') {
        normalized_slashes
    } else if let Some(base) = fallback {
        join_remote_base(base, &normalized_slashes)
    } else {
        return Err(FileOpPathError::new(
            reason_code,
            "Remote path must be absolute.",
        ));
    };

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
        return Err(FileOpPathError::new(
            reason_code,
            "Path must be absolute.",
        ));
    }
    #[cfg(target_os = "windows")]
    validate_windows_components(&path, reason_code)?;
    Ok(path)
}

fn contains_forbidden_chars(value: &str) -> bool {
    value.chars().any(|ch| ch == '\0' || ch.is_control())
}

fn join_remote_base(base: &str, child: &str) -> String {
    let clean_base = if base.trim().is_empty() { "/" } else { base.trim() };
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

fn is_root_like(path: &Path) -> bool {
    path.parent().is_none()
}

#[cfg(target_os = "windows")]
fn validate_windows_components(
    path: &Path,
    reason_code: &'static str,
) -> Result<(), FileOpPathError> {
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
        "LPT9",
    ];
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) {
                    continue;
                }
            }
            Component::RootDir => continue,
            Component::Normal(value) => {
                let segment = value.to_string_lossy();
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
                if reserved.contains(&upper.as_str()) {
                    return Err(FileOpPathError::new(
                        reason_code,
                        "Windows path uses a reserved device name.",
                    ));
                }
                if segment.chars().any(|ch| matches!(ch, '<' | '>' | '"' | '|' | '?' | '*')) {
                    return Err(FileOpPathError::new(
                        reason_code,
                        "Windows path contains invalid characters.",
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}
