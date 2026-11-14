//! Shared encrypted backup transaction engine, IO helpers, archives, and primitives.
pub mod archive;
pub mod crypto;
pub mod io;
pub mod references;
pub mod transaction;

pub use archive::{
    compile_restore, describe, fingerprint, is_valid_server_id, known_accounts, snapshot,
    snapshot_with_version, validate_export_target, Archive, ExternalFile, PendingRestore, Preview,
    RestoreOptions, SectionSummary,
};

use crate::models::{
    KEYRING_BOUNDARY_OIDC_OTP_SECRET, KEYRING_BOUNDARY_OIDC_PASSWORD, KEYRING_BOUNDARY_PASSWORD,
    KEYRING_OPENVPN_OTP_SECRET, KEYRING_OPENVPN_PASSWORD,
};
use serde_json::Value;
use std::collections::BTreeMap;

pub const FILES: [&str; 3] = ["config.json", "servers.json", "sync-local.json"];

pub const CONFIGURED: [&str; 5] = [
    KEYRING_OPENVPN_PASSWORD,
    KEYRING_OPENVPN_OTP_SECRET,
    KEYRING_BOUNDARY_PASSWORD,
    KEYRING_BOUNDARY_OIDC_PASSWORD,
    KEYRING_BOUNDARY_OIDC_OTP_SECRET,
];

pub const LOCAL_PATHS: [&str; 5] = [
    "repoRoot",
    "openvpnExecutablePath",
    "openvpnOvpnPath",
    "boundaryCliPath",
    "caddyFolder",
];

pub trait SecretStore {
    fn read(&mut self, account: &str) -> Result<Option<String>, String>;
    fn write(&mut self, account: &str, value: Option<&str>) -> Result<(), String>;
}

pub struct Changes {
    pub files: BTreeMap<String, Option<Value>>,
    pub credentials: BTreeMap<String, Option<String>>,
}

pub fn wipe_value(value: &mut Value) {
    use zeroize::Zeroize;
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(wipe_value),
        Value::Object(fields) => fields.values_mut().for_each(wipe_value),
        _ => {}
    }
}

pub(crate) fn wipe_json_and_credentials(
    files: &mut BTreeMap<String, Option<Value>>,
    credentials: &mut BTreeMap<String, Option<String>>,
) {
    use zeroize::Zeroize;
    files.values_mut().flatten().for_each(wipe_value);
    credentials.values_mut().flatten().for_each(String::zeroize);
}

impl Drop for Changes {
    fn drop(&mut self) {
        wipe_json_and_credentials(&mut self.files, &mut self.credentials);
    }
}
