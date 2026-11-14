//! Shared encrypted backup transaction engine, IO helpers, and primitives.
pub mod crypto;
pub mod io;
pub mod transaction;

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

impl Drop for Changes {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.files.values_mut().flatten().for_each(wipe_value);
        self.credentials
            .values_mut()
            .flatten()
            .for_each(|value| value.zeroize());
    }
}
