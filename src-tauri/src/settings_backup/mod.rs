//! Local-only encrypted settings backups. Never expose Archive or Changes over IPC/MCP.
pub mod desktop;
pub mod gate;

#[allow(unused_imports)]
pub(crate) use scp_core::backup::{
    compile_restore, crypto, io, known_accounts, snapshot_with_version, transaction,
    validate_export_target, wipe_value, ExternalFile, PendingRestore, CONFIGURED, FILES,
    LOCAL_PATHS,
};
pub use scp_core::backup::{Archive, Changes, RestoreOptions, SecretStore};
use std::path::Path;

pub fn snapshot(dir: &Path, store: &mut impl SecretStore) -> Result<Archive, String> {
    snapshot_with_version(dir, env!("CARGO_PKG_VERSION"), store)
}

#[cfg(test)]
mod tests;
