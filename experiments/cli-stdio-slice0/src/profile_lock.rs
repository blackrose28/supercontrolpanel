use crate::SpikeError;
use serde_json::json;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub struct OwnerLease {
    _file: File,
    diagnostics_path: PathBuf,
}

impl OwnerLease {
    pub fn diagnostics_path(&self) -> &Path {
        &self.diagnostics_path
    }
}

pub fn validate_profile_name(name: &str) -> Result<&str, SpikeError> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    if !valid {
        return Err(SpikeError::new(
            "INVALID_ARGUMENTS",
            "invalid_profile_name",
            format!("Invalid profile name: {name}"),
            2,
        ));
    }
    Ok(name)
}

/// Canonicalize existing prefix and lexically resolve remaining `.` / `..` segments
/// so non-existent leaf directories under symlinked parents still compare accurately.
fn resolve_with_symlinks(path: &Path) -> Result<PathBuf, SpikeError> {
    let mut existing = path.to_path_buf();
    let mut tail = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        tail.push(name.to_os_string());
        let Some(parent) = existing.parent() else {
            break;
        };
        existing = parent.to_path_buf();
    }
    let mut resolved = if existing.as_os_str().is_empty() {
        std::env::current_dir()
            .map_err(|e| SpikeError::new("OPERATION_FAILED", "cwd_failed", e.to_string(), 1))?
    } else if existing.exists() {
        fs::canonicalize(&existing).map_err(|e| {
            SpikeError::new("OPERATION_FAILED", "canonicalize_failed", e.to_string(), 1)
        })?
    } else {
        existing
    };
    for part in tail.into_iter().rev() {
        match Component::Normal(&part) {
            Component::Normal(seg) if seg == ".." => {
                resolved.pop();
            }
            Component::Normal(seg) if seg != "." => {
                resolved.push(seg);
            }
            _ => {}
        }
    }
    Ok(resolved)
}

fn ensure_private_dir(dir: &Path) -> Result<(), SpikeError> {
    fs::create_dir_all(dir).map_err(|e| {
        SpikeError::new(
            "OPERATION_FAILED",
            "profile_dir_create_failed",
            format!("Failed to create {}: {e}", dir.display()),
            1,
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|e| {
            SpikeError::new(
                "OPERATION_FAILED",
                "profile_dir_chmod_failed",
                format!("Failed to restrict {}: {e}", dir.display()),
                1,
            )
        })?;
    }
    Ok(())
}

pub fn resolve_profile_dir(
    config_dir: Option<&Path>,
    env_config_dir: Option<&Path>,
    xdg_config_home: Option<&Path>,
    home_dir: Option<&Path>,
    desktop_config_dir: Option<&Path>,
    profile: Option<&str>,
) -> Result<PathBuf, SpikeError> {
    let profile = validate_profile_name(profile.unwrap_or("default"))?;
    let raw_root = if let Some(dir) = config_dir {
        dir.to_path_buf()
    } else if let Some(dir) = env_config_dir {
        dir.to_path_buf()
    } else if let Some(xdg) = xdg_config_home {
        xdg.join("scpctl")
    } else if let Some(home) = home_dir {
        home.join(".config").join("scpctl")
    } else {
        return Err(SpikeError::new(
            "CAPABILITY_UNAVAILABLE",
            "config_root_unavailable",
            "Cannot determine CLI config root directory",
            4,
        ));
    };

    let resolved_root = resolve_with_symlinks(&raw_root)?;
    if let Some(desktop) = desktop_config_dir {
        let resolved_desktop = resolve_with_symlinks(desktop)?;
        if resolved_root == resolved_desktop || resolved_root.starts_with(&resolved_desktop) {
            return Err(SpikeError::new(
                "POLICY_DENIED",
                "desktop_profile_rejected",
                "Refusing desktop profile directory; select a standalone CLI profile or use settings import",
                3,
            ));
        }
    }

    ensure_private_dir(&resolved_root)?;
    let profiles_dir = resolved_root.join("profiles");
    ensure_private_dir(&profiles_dir)?;
    let profile_dir = profiles_dir.join(profile);
    ensure_private_dir(&profile_dir)?;
    let canon_profile = fs::canonicalize(&profile_dir).map_err(|e| {
        SpikeError::new("OPERATION_FAILED", "canonicalize_failed", e.to_string(), 1)
    })?;
    if let Some(desktop) = desktop_config_dir {
        let resolved_desktop = resolve_with_symlinks(desktop)?;
        if canon_profile == resolved_desktop || canon_profile.starts_with(&resolved_desktop) {
            return Err(SpikeError::new(
                "POLICY_DENIED",
                "desktop_profile_rejected",
                "Refusing canonicalized profile inside desktop directory",
                3,
            ));
        }
    }
    Ok(canon_profile)
}

/// Acquires an exclusive non-blocking OS file lock without unlinking on drop
/// so concurrent openers always serialize on the same inode.
pub fn acquire_exclusive_lock_file(
    lock_path: &Path,
    runtime_id: &str,
    mode: &str,
    conflict_reason: &'static str,
) -> Result<OwnerLease, SpikeError> {
    if let Some(parent) = lock_path.parent() {
        ensure_private_dir(parent)?;
    }
    if let Ok(meta) = fs::symlink_metadata(lock_path)
        && meta.file_type().is_symlink()
    {
        return Err(SpikeError::new(
            "POLICY_DENIED",
            "unsafe_lock_symlink",
            "Refusing symlink lock file",
            3,
        ));
    }

    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(lock_path).map_err(|e| {
        SpikeError::new(
            "OPERATION_FAILED",
            "lock_open_failed",
            format!("Failed to open lock file {}: {e}", lock_path.display()),
            1,
        )
    })?;

    try_flock_nonblocking(&file, lock_path, conflict_reason)?;

    let diag = json!({
        "runtimeId": runtime_id,
        "pid": std::process::id(),
        "mode": mode,
    })
    .to_string();
    file.set_len(0)
        .map_err(|e| SpikeError::new("OPERATION_FAILED", "lock_write_failed", e.to_string(), 1))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| SpikeError::new("OPERATION_FAILED", "lock_write_failed", e.to_string(), 1))?;
    writeln!(file, "{diag}")
        .map_err(|e| SpikeError::new("OPERATION_FAILED", "lock_write_failed", e.to_string(), 1))?;
    file.sync_data()
        .map_err(|e| SpikeError::new("OPERATION_FAILED", "lock_write_failed", e.to_string(), 1))?;

    Ok(OwnerLease {
        _file: file,
        diagnostics_path: lock_path.to_path_buf(),
    })
}

pub fn acquire_owner_lease(
    profile_dir: &Path,
    runtime_id: &str,
    mode: &str,
) -> Result<OwnerLease, SpikeError> {
    acquire_exclusive_lock_file(
        &profile_dir.join("owner.lock"),
        runtime_id,
        mode,
        "profile_owner_conflict",
    )
}

#[cfg(unix)]
fn try_flock_nonblocking(
    file: &File,
    lock_path: &Path,
    conflict_reason: &'static str,
) -> Result<(), SpikeError> {
    use std::os::fd::AsRawFd;
    // SAFETY: `file` owns a valid open descriptor for the duration of this call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        let mut raw = String::new();
        if let Ok(reader) = File::open(lock_path) {
            let _ = reader.take(2048).read_to_string(&mut raw);
        }
        let owner_info = raw.trim();
        let detail = if owner_info.is_empty() {
            "active owner process".to_string()
        } else {
            owner_info.to_string()
        };
        return Err(SpikeError::new(
            "CONFLICT",
            conflict_reason,
            format!(
                "CONFLICT ({conflict_reason}): lock {} held by {detail}",
                lock_path.display()
            ),
            5,
        ));
    }
    Err(SpikeError::new(
        "OPERATION_FAILED",
        "lock_acquire_failed",
        format!("flock failed on {}: {err}", lock_path.display()),
        1,
    ))
}

#[cfg(windows)]
fn try_flock_nonblocking(
    file: &File,
    lock_path: &Path,
    conflict_reason: &'static str,
) -> Result<(), SpikeError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;

    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if ok != 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        return Err(SpikeError::new(
            "CONFLICT",
            conflict_reason,
            format!(
                "CONFLICT ({conflict_reason}): lock {} held by another process",
                lock_path.display()
            ),
            5,
        ));
    }
    Err(SpikeError::new(
        "OPERATION_FAILED",
        "lock_acquire_failed",
        format!("LockFileEx failed on {}: {err}", lock_path.display()),
        1,
    ))
}
