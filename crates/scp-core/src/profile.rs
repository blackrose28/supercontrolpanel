//! Profile resolution, desktop alias rejection, exclusive OS owner lease, and namespace metadata.
use crate::backup::{io, transaction};
use crate::error::CoreError;
use crate::models::DESKTOP_APP_IDENTIFIER;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ProfilePaths {
    pub root: PathBuf,
    pub profile_name: String,
    pub profile_dir: PathBuf,
    pub lock_path: PathBuf,
    pub meta_path: PathBuf,
    pub config_path: PathBuf,
    pub servers_path: PathBuf,
    pub sync_local_path: PathBuf,
    pub stdio_policy_path: PathBuf,
    pub journal_path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct ProfileLocationInput<'a> {
    pub config_dir: Option<&'a Path>,
    pub env_config_dir: Option<&'a Path>,
    pub xdg_config_home: Option<&'a Path>,
    pub home_dir: Option<&'a Path>,
    pub desktop_config_dir: Option<&'a Path>,
    pub profile: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct OwnedProfileLocation {
    pub config_dir: Option<PathBuf>,
    pub env_config_dir: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub home_dir: Option<PathBuf>,
    pub desktop_config_dir: Option<PathBuf>,
    pub profile: Option<String>,
}

impl OwnedProfileLocation {
    pub fn from_env(config_dir: Option<PathBuf>, profile: Option<String>) -> Self {
        let env_config_dir = std::env::var_os("SCPCTL_CONFIG_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let xdg_config_home = non_empty_env_path("XDG_CONFIG_HOME");
        let home_dir = non_empty_env_path("HOME").or_else(|| non_empty_env_path("USERPROFILE"));
        let desktop_config_dir =
            non_empty_env_path("SCPCTL_DESKTOP_CONFIG_DIR").or_else(default_desktop_config_dir);
        Self {
            config_dir,
            env_config_dir,
            xdg_config_home,
            home_dir,
            desktop_config_dir,
            profile,
        }
    }

    pub fn as_input(&self) -> ProfileLocationInput<'_> {
        ProfileLocationInput {
            config_dir: self.config_dir.as_deref(),
            env_config_dir: self.env_config_dir.as_deref(),
            xdg_config_home: self.xdg_config_home.as_deref(),
            home_dir: self.home_dir.as_deref(),
            desktop_config_dir: self.desktop_config_dir.as_deref(),
            profile: self.profile.as_deref(),
        }
    }
}

fn non_empty_env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub fn default_desktop_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return non_empty_env_path("HOME").map(|h| {
            h.join("Library")
                .join("Application Support")
                .join(DESKTOP_APP_IDENTIFIER)
        });
    }
    #[cfg(target_os = "windows")]
    {
        return non_empty_env_path("APPDATA").map(|a| a.join(DESKTOP_APP_IDENTIFIER));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        non_empty_env_path("XDG_CONFIG_HOME")
            .or_else(|| non_empty_env_path("HOME").map(|h| h.join(".config")))
            .map(|base| base.join(DESKTOP_APP_IDENTIFIER))
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileMetadata {
    pub schema_version: u32,
    pub credential_namespace: String,
}

pub fn validate_profile_name(name: &str) -> Result<&str, CoreError> {
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
        return Err(CoreError::invalid_arguments(
            "invalid_profile_name",
            format!("Invalid profile name: {name}"),
        ));
    }
    Ok(name)
}

fn resolve_with_symlinks(path: &Path) -> Result<PathBuf, CoreError> {
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
        std::env::current_dir().map_err(|e| {
            CoreError::operation_failed("cwd_failed", e.to_string(), false, "fix_configuration")
        })?
    } else if existing.exists() {
        fs::canonicalize(&existing).map_err(|e| {
            CoreError::operation_failed(
                "canonicalize_failed",
                e.to_string(),
                false,
                "fix_configuration",
            )
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

pub fn ensure_private_dir(dir: &Path) -> Result<(), CoreError> {
    fs::create_dir_all(dir).map_err(|e| {
        CoreError::operation_failed(
            "profile_dir_create_failed",
            format!("Failed to create {}: {e}", dir.display()),
            false,
            "fix_configuration",
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|e| {
            CoreError::operation_failed(
                "profile_dir_chmod_failed",
                format!("Failed to restrict {}: {e}", dir.display()),
                false,
                "fix_configuration",
            )
        })?;
    }
    Ok(())
}

fn pick_config_root(input: &ProfileLocationInput<'_>) -> Result<PathBuf, CoreError> {
    if let Some(dir) = input.config_dir {
        if dir.as_os_str().is_empty() {
            return Err(CoreError::invalid_arguments(
                "invalid_config_dir",
                "Config directory path must not be empty",
            ));
        }
        return Ok(dir.to_path_buf());
    }
    if let Some(dir) = input.env_config_dir {
        return Ok(dir.to_path_buf());
    }
    if let Some(xdg) = input.xdg_config_home {
        return Ok(xdg.join("scpctl"));
    }
    if let Some(home) = input.home_dir {
        #[cfg(target_os = "macos")]
        return Ok(home
            .join("Library")
            .join("Application Support")
            .join("scpctl"));
        #[cfg(not(target_os = "macos"))]
        return Ok(home.join(".config").join("scpctl"));
    }
    Err(CoreError::capability_unavailable(
        "config_root_unavailable",
        "Cannot determine CLI config root directory",
        "fix_configuration",
    ))
}

fn reject_if_desktop(candidate: &Path, desktop: Option<&Path>) -> Result<(), CoreError> {
    let Some(resolved_desktop) = desktop else {
        return Ok(());
    };
    if candidate == resolved_desktop || candidate.starts_with(resolved_desktop) {
        return Err(CoreError::policy_denied(
            "desktop_profile_rejected",
            "Refusing desktop profile directory; select a standalone CLI profile or use settings import",
            "select_standalone_profile",
        ));
    }
    Ok(())
}

pub fn resolve_profile_paths(input: &ProfileLocationInput<'_>) -> Result<ProfilePaths, CoreError> {
    let profile = validate_profile_name(input.profile.unwrap_or("default"))?;
    let raw_root = pick_config_root(input)?;
    let resolved_desktop = input
        .desktop_config_dir
        .map(resolve_with_symlinks)
        .transpose()?;
    let resolved_root = resolve_with_symlinks(&raw_root)?;
    reject_if_desktop(&resolved_root, resolved_desktop.as_deref())?;

    let precheck_profile = resolve_with_symlinks(&resolved_root.join("profiles").join(profile))?;
    reject_if_desktop(&precheck_profile, resolved_desktop.as_deref())?;

    ensure_private_dir(&resolved_root)?;
    let profiles_dir = resolved_root.join("profiles");
    ensure_private_dir(&profiles_dir)?;
    let profile_dir = profiles_dir.join(profile);
    ensure_private_dir(&profile_dir)?;

    let canon_root = fs::canonicalize(&resolved_root).unwrap_or(resolved_root);
    let canon_profile = fs::canonicalize(&profile_dir).map_err(|e| {
        CoreError::operation_failed(
            "canonicalize_failed",
            e.to_string(),
            false,
            "fix_configuration",
        )
    })?;
    reject_if_desktop(&canon_profile, resolved_desktop.as_deref())?;

    Ok(ProfilePaths {
        root: canon_root,
        profile_name: profile.to_string(),
        lock_path: canon_profile.join("owner.lock"),
        meta_path: canon_profile.join("profile-meta.json"),
        config_path: canon_profile.join("config.json"),
        servers_path: canon_profile.join("servers.json"),
        sync_local_path: canon_profile.join("sync-local.json"),
        stdio_policy_path: canon_profile.join("stdio-policy.json"),
        journal_path: canon_profile.join(transaction::JOURNAL_FILE),
        profile_dir: canon_profile,
    })
}

pub fn acquire_owner_lease(
    paths: &ProfilePaths,
    runtime_id: &str,
    mode: &str,
) -> Result<OwnerLease, CoreError> {
    acquire_exclusive_lock_file(&paths.lock_path, runtime_id, mode, "profile_owner_conflict")
}

pub fn acquire_exclusive_lock_file(
    lock_path: &Path,
    runtime_id: &str,
    mode: &str,
    conflict_reason: &'static str,
) -> Result<OwnerLease, CoreError> {
    if let Some(parent) = lock_path.parent() {
        ensure_private_dir(parent)?;
    }
    if fs::symlink_metadata(lock_path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(CoreError::policy_denied(
            "unsafe_lock_symlink",
            "Refusing symlink lock file",
            "fix_configuration",
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
        CoreError::operation_failed(
            "lock_open_failed",
            format!("Failed to open lock file {}: {e}", lock_path.display()),
            false,
            "fix_configuration",
        )
    })?;

    try_flock_nonblocking(&file, lock_path, conflict_reason)?;

    let diag = json!({
        "runtimeId": runtime_id,
        "pid": std::process::id(),
        "mode": mode,
    })
    .to_string();
    write_lock_diagnostics(&mut file, &diag)?;

    Ok(OwnerLease {
        _file: file,
        diagnostics_path: lock_path.to_path_buf(),
    })
}

fn write_lock_diagnostics(file: &mut File, diag: &str) -> Result<(), CoreError> {
    let map_err = |e: std::io::Error| {
        CoreError::operation_failed(
            "lock_write_failed",
            e.to_string(),
            false,
            "fix_configuration",
        )
    };
    file.set_len(0).map_err(map_err)?;
    file.seek(SeekFrom::Start(0)).map_err(map_err)?;
    writeln!(file, "{diag}").map_err(map_err)?;
    file.sync_data().map_err(map_err)
}

#[cfg(unix)]
fn try_flock_nonblocking(
    file: &File,
    lock_path: &Path,
    conflict_reason: &'static str,
) -> Result<(), CoreError> {
    use std::os::fd::AsRawFd;
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
            "active owner process"
        } else {
            owner_info
        };
        return Err(CoreError::conflict(
            conflict_reason,
            format!(
                "Profile owner lock {} held by {detail}; query or stop the owning process",
                lock_path.display()
            ),
            true,
            "use_owner_connection",
        ));
    }
    Err(CoreError::operation_failed(
        "lock_acquire_failed",
        format!("flock failed on {}: {err}", lock_path.display()),
        false,
        "fix_configuration",
    ))
}

#[cfg(windows)]
fn try_flock_nonblocking(
    file: &File,
    lock_path: &Path,
    conflict_reason: &'static str,
) -> Result<(), CoreError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
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
        return Err(CoreError::conflict(
            conflict_reason,
            format!(
                "Profile owner lock {} held by another process",
                lock_path.display()
            ),
            true,
            "use_owner_connection",
        ));
    }
    Err(CoreError::operation_failed(
        "lock_acquire_failed",
        format!("LockFileEx failed on {}: {err}", lock_path.display()),
        false,
        "fix_configuration",
    ))
}

pub fn ensure_or_load_metadata(paths: &ProfilePaths) -> Result<ProfileMetadata, CoreError> {
    if let Some(bytes) = io::read_bounded(&paths.meta_path).map_err(|e| {
        CoreError::operation_failed("profile_meta_read_failed", e, false, "fix_configuration")
    })? {
        let meta: ProfileMetadata = serde_json::from_slice(&bytes).map_err(|_| {
            CoreError::operation_failed(
                "profile_meta_corrupted",
                "Invalid profile-meta.json",
                false,
                "fix_configuration",
            )
        })?;
        if meta.schema_version != 1
            || meta.credential_namespace.is_empty()
            || meta.credential_namespace.len() > 64
            || !meta
                .credential_namespace
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(CoreError::operation_failed(
                "profile_meta_invalid",
                "Unsupported or invalid profile-meta.json namespace",
                false,
                "fix_configuration",
            ));
        }
        return Ok(meta);
    }

    let pending_journal = io::read_bounded(&paths.journal_path)
        .map_err(|e| CoreError::conflict("recovery_blocked", e, false, "unlock_keyring_and_retry"))?
        .is_some_and(|b| b != transaction::DONE);
    if pending_journal {
        return Err(CoreError::conflict(
            "missing_credential_namespace",
            "Pending recovery journal exists without profile-meta.json; restore original profile metadata before retrying startup",
            false,
            "fix_configuration",
        ));
    }

    let meta = ProfileMetadata {
        schema_version: 1,
        credential_namespace: Uuid::new_v4().simple().to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&meta).map_err(|e| {
        CoreError::operation_failed(
            "profile_meta_encode_failed",
            e.to_string(),
            false,
            "fix_configuration",
        )
    })?;
    io::atomic_write(&paths.meta_path, &bytes, false).map_err(|e| {
        CoreError::operation_failed("profile_meta_write_failed", e, false, "fix_configuration")
    })?;
    Ok(meta)
}
