//! CLI-local identity and exclusive runtime ownership. Desktop profiles do not use this module.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};
use uuid::Uuid;

const NAMESPACE_PREFIX: &str = "supercontrolpanel-cli-";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileIdentity {
    version: u32,
    credential_namespace: String,
}

/// Retain this guard for the entire lifetime of a runtime or config write.
/// The persistent lock file must never be deleted: ownership is the OS file lock,
/// not the presence of the file. Closing the guard (including process exit) releases it.
pub struct ProfileOwner {
    _lock: OwnerLock,
    identity: ProfileIdentity,
}

// Explicit unlock prevents a descriptor inherited by concurrent fork/exec from
// extending ownership after this guard is dropped, including initialization errors.
struct OwnerLock(File);

impl Drop for OwnerLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl ProfileOwner {
    /// Initialize a local profile if needed, failing immediately on competing ownership.
    /// Callers must not acquire this for read-only inventory queries.
    pub fn acquire(config_dir: &Path) -> Result<Self, String> {
        let mut directory = fs::DirBuilder::new();
        directory.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory
            .create(config_dir)
            .map_err(|e| format!("create CLI profile directory: {e}"))?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(config_dir.join("runtime-owner.lock"))
            .map_err(|e| format!("open CLI profile owner lock: {e}"))?;
        match lock.try_lock() {
            Ok(()) => {},
            Err(std::fs::TryLockError::WouldBlock) => return Err(
                "CLI profile is already owned by another process; stop that owner or select another --config-dir".into()),
            Err(e) => return Err(format!("lock CLI profile: {e}")),
        }
        let lock = OwnerLock(lock);
        let identity = load_or_create_identity(config_dir)?;
        Ok(Self {
            _lock: lock,
            identity,
        })
    }

    /// Keyring service/cache namespace for this profile; never use desktop's fixed service.
    pub fn credential_namespace(&self) -> &str {
        &self.identity.credential_namespace
    }
}

fn load_or_create_identity(config_dir: &Path) -> Result<ProfileIdentity, String> {
    let path = config_dir.join("profile.json");
    match fs::read(&path) {
        Ok(bytes) => {
            let identity: ProfileIdentity = serde_json::from_slice(&bytes).map_err(|_| {
                "invalid CLI profile metadata; restore the original profile.json".to_string()
            })?;
            let valid_namespace = identity
                .credential_namespace
                .strip_prefix(NAMESPACE_PREFIX)
                .and_then(|id| Uuid::parse_str(id).ok())
                .is_some_and(|id| id.get_version_num() == 4);
            if identity.version != 1 || !valid_namespace {
                return Err(
                    "unsupported CLI profile metadata version or credential namespace".into(),
                );
            }
            Ok(identity)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let identity = ProfileIdentity {
                version: 1,
                credential_namespace: format!("{NAMESPACE_PREFIX}{}", Uuid::new_v4()),
            };
            let mut pending = tempfile::NamedTempFile::new_in(config_dir)
                .map_err(|e| format!("create CLI profile metadata: {e}"))?;
            serde_json::to_writer_pretty(&mut pending, &identity)
                .map_err(|e| format!("encode CLI profile metadata: {e}"))?;
            pending
                .flush()
                .and_then(|_| pending.as_file().sync_all())
                .map_err(|e| format!("flush CLI profile metadata: {e}"))?;
            pending
                .persist_noclobber(path)
                .map_err(|e| format!("save CLI profile metadata: {}", e.error))?;
            Ok(identity)
        }
        Err(e) => Err(format!("read CLI profile metadata: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::ProfileOwner;

    #[test]
    fn dropping_owner_unlocks_even_while_a_duplicate_file_description_exists() {
        let temp = tempfile::tempdir().unwrap();
        let owner = ProfileOwner::acquire(temp.path()).unwrap();
        // Models the descriptor briefly inherited by a concurrent fork before exec.
        let inherited = owner._lock.0.try_clone().unwrap();
        assert!(ProfileOwner::acquire(temp.path()).is_err());
        drop(owner);
        let replacement =
            ProfileOwner::acquire(temp.path()).expect("owner explicitly releases lock");
        drop(inherited);
        assert!(ProfileOwner::acquire(temp.path()).is_err());
        drop(replacement);
        assert!(ProfileOwner::acquire(temp.path()).is_ok());
    }
}
