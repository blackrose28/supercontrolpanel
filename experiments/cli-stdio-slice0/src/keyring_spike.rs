use crate::SpikeError;
use keyring::{Entry, Error as KeyringError};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyringHealth {
    Ready,
    Locked,
    Unavailable(String),
}

pub struct NamespacedSecretStore {
    service: String,
    namespace: String,
    entry_prefix: String,
    cache: HashMap<(String, String), String>,
}

impl NamespacedSecretStore {
    pub fn new(service: impl Into<String>, namespace: impl Into<String>) -> Self {
        let namespace = namespace.into();
        let entry_prefix = format!("profile:{namespace}:");
        Self {
            service: service.into(),
            namespace,
            entry_prefix,
            cache: HashMap::new(),
        }
    }

    fn scoped_account(&self, logical_account: &str) -> String {
        format!("{}{logical_account}", self.entry_prefix)
    }

    fn cache_key(&self, logical_account: &str) -> (String, String) {
        (self.namespace.clone(), logical_account.to_string())
    }

    pub fn cached_get(&self, logical_account: &str) -> Option<&str> {
        self.cache
            .get(&self.cache_key(logical_account))
            .map(String::as_str)
    }

    pub fn get(&mut self, logical_account: &str) -> Result<Option<String>, SpikeError> {
        ensure_keyring_ready()?;
        let account = self.scoped_account(logical_account);
        let entry = Entry::new(&self.service, &account).map_err(map_keyring_err)?;
        match entry.get_password() {
            Ok(secret) => {
                self.cache
                    .insert(self.cache_key(logical_account), secret.clone());
                Ok(Some(secret))
            }
            Err(KeyringError::NoEntry) => {
                self.cache.remove(&self.cache_key(logical_account));
                Ok(None)
            }
            Err(err) => Err(map_keyring_err(err)),
        }
    }

    pub fn set(&mut self, logical_account: &str, value: &str) -> Result<(), SpikeError> {
        ensure_keyring_ready()?;
        let account = self.scoped_account(logical_account);
        let entry = Entry::new(&self.service, &account).map_err(map_keyring_err)?;
        entry.set_password(value).map_err(map_keyring_err)?;
        self.cache
            .insert(self.cache_key(logical_account), value.to_string());
        Ok(())
    }

    pub fn delete(&mut self, logical_account: &str) -> Result<(), SpikeError> {
        ensure_keyring_ready()?;
        let account = self.scoped_account(logical_account);
        let entry = Entry::new(&self.service, &account).map_err(map_keyring_err)?;
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => {
                self.cache.remove(&self.cache_key(logical_account));
                Ok(())
            }
            Err(err) => Err(map_keyring_err(err)),
        }
    }
}

/// Checks Secret Service lock state via D-Bus properties without triggering an interactive prompt.
pub fn preflight_keyring() -> KeyringHealth {
    #[cfg(target_os = "linux")]
    {
        use dbus_secret_service::{EncryptionType, SecretService};
        let ss = match SecretService::connect(EncryptionType::Plain) {
            Ok(ss) => ss,
            Err(err) => {
                return KeyringHealth::Unavailable(format!("Secret Service connect failed: {err}"));
            }
        };
        let collection = match ss.get_default_collection() {
            Ok(col) => col,
            Err(err) => {
                return KeyringHealth::Unavailable(format!(
                    "Secret Service default collection failed: {err}"
                ));
            }
        };
        match collection.is_locked() {
            Ok(true) => KeyringHealth::Locked,
            Ok(false) => KeyringHealth::Ready,
            Err(err) => {
                KeyringHealth::Unavailable(format!("Secret Service lock check failed: {err}"))
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        KeyringHealth::Ready
    }
}

pub fn ensure_keyring_ready() -> Result<(), SpikeError> {
    match preflight_keyring() {
        KeyringHealth::Ready => Ok(()),
        KeyringHealth::Locked => Err(SpikeError::new(
            "CAPABILITY_UNAVAILABLE",
            "keyring_locked",
            "OS keyring is locked; unlock it before noninteractive CLI execution",
            4,
        )),
        KeyringHealth::Unavailable(detail) => Err(SpikeError::new(
            "CAPABILITY_UNAVAILABLE",
            "keyring_unavailable",
            detail,
            4,
        )),
    }
}

fn map_keyring_err(err: KeyringError) -> SpikeError {
    match err {
        KeyringError::NoStorageAccess(_) => SpikeError::new(
            "CAPABILITY_UNAVAILABLE",
            "keyring_unavailable",
            format!("OS keyring access denied or unavailable: {err}"),
            4,
        ),
        other => SpikeError::new(
            "OPERATION_FAILED",
            "keyring_error",
            format!("OS keyring failure: {other}"),
            1,
        ),
    }
}
