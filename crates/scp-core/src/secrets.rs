//! Namespaced OS keyring store with non-prompting Linux lock preflight and isolated cache.
use crate::backup::SecretStore;
use crate::error::CoreError;
use keyring::{Entry, Error as KeyringError};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyringHealth {
    Ready,
    Locked,
    Unavailable(String),
}

pub trait KeyringBackend: Send {
    fn preflight(&self) -> KeyringHealth;
    fn read_raw(&mut self, service: &str, account: &str) -> Result<Option<String>, CoreError>;
    fn write_raw(
        &mut self,
        service: &str,
        account: &str,
        value: Option<&str>,
    ) -> Result<(), CoreError>;
}

#[derive(Debug, Default, Clone)]
pub struct OsKeyringBackend;

impl KeyringBackend for OsKeyringBackend {
    fn preflight(&self) -> KeyringHealth {
        preflight_keyring()
    }

    fn read_raw(&mut self, service: &str, account: &str) -> Result<Option<String>, CoreError> {
        ensure_keyring_ready()?;
        let entry = Entry::new(service, account).map_err(map_keyring_err)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(err) => Err(map_keyring_err(err)),
        }
    }

    fn write_raw(
        &mut self,
        service: &str,
        account: &str,
        value: Option<&str>,
    ) -> Result<(), CoreError> {
        ensure_keyring_ready()?;
        let entry = Entry::new(service, account).map_err(map_keyring_err)?;
        match value {
            Some(secret) => entry.set_password(secret).map_err(map_keyring_err),
            None => match entry.delete_credential() {
                Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
                Err(err) => Err(map_keyring_err(err)),
            },
        }
    }
}

#[derive(Debug)]
struct MemoryKeyringState {
    health: KeyringHealth,
    entries: HashMap<(String, String), String>,
}

#[derive(Debug, Clone)]
pub struct MemoryKeyringBackend {
    state: Arc<Mutex<MemoryKeyringState>>,
}

impl Default for MemoryKeyringBackend {
    fn default() -> Self {
        Self::new(KeyringHealth::Ready)
    }
}

impl MemoryKeyringBackend {
    pub fn new(health: KeyringHealth) -> Self {
        Self {
            state: Arc::new(Mutex::new(MemoryKeyringState {
                health,
                entries: HashMap::new(),
            })),
        }
    }

    pub fn snapshot_entry(&self, service: &str, account: &str) -> Option<String> {
        self.state
            .lock()
            .entries
            .get(&(service.to_string(), account.to_string()))
            .cloned()
    }

    pub fn seed_entry(&self, service: &str, account: &str, value: &str) {
        self.state.lock().entries.insert(
            (service.to_string(), account.to_string()),
            value.to_string(),
        );
    }
}

impl KeyringBackend for MemoryKeyringBackend {
    fn preflight(&self) -> KeyringHealth {
        self.state.lock().health.clone()
    }

    fn read_raw(&mut self, service: &str, account: &str) -> Result<Option<String>, CoreError> {
        check_health(&self.state.lock().health)?;
        Ok(self
            .state
            .lock()
            .entries
            .get(&(service.to_string(), account.to_string()))
            .cloned())
    }

    fn write_raw(
        &mut self,
        service: &str,
        account: &str,
        value: Option<&str>,
    ) -> Result<(), CoreError> {
        let mut guard = self.state.lock();
        check_health(&guard.health)?;
        let key = (service.to_string(), account.to_string());
        match value {
            Some(secret) => {
                guard.entries.insert(key, secret.to_string());
            }
            None => {
                guard.entries.remove(&key);
            }
        }
        Ok(())
    }
}

pub struct NamespacedSecretStore<B: KeyringBackend = OsKeyringBackend> {
    service: String,
    namespace: String,
    entry_prefix: String,
    cache: HashMap<(String, String), String>,
    backend: B,
}

impl<B: KeyringBackend> NamespacedSecretStore<B> {
    pub fn new(service: impl Into<String>, namespace: impl Into<String>, backend: B) -> Self {
        let namespace = namespace.into();
        let entry_prefix = format!("profile:{namespace}:");
        Self {
            service: service.into(),
            namespace,
            entry_prefix,
            cache: HashMap::new(),
            backend,
        }
    }

    fn scoped_account(&self, logical_account: &str) -> String {
        format!("{}{logical_account}", self.entry_prefix)
    }

    fn cache_key(&self, logical_account: &str) -> (String, String) {
        (self.namespace.clone(), logical_account.to_string())
    }

    pub fn preflight(&self) -> KeyringHealth {
        self.backend.preflight()
    }

    pub fn cached_get(&self, logical_account: &str) -> Option<&str> {
        self.cache
            .get(&self.cache_key(logical_account))
            .map(String::as_str)
    }

    pub fn get(&mut self, logical_account: &str) -> Result<Option<String>, CoreError> {
        let scoped = self.scoped_account(logical_account);
        let value = self.backend.read_raw(&self.service, &scoped)?;
        match &value {
            Some(secret) => {
                self.cache
                    .insert(self.cache_key(logical_account), secret.clone());
            }
            None => {
                self.cache.remove(&self.cache_key(logical_account));
            }
        }
        Ok(value)
    }

    pub fn set(&mut self, logical_account: &str, value: &str) -> Result<(), CoreError> {
        let scoped = self.scoped_account(logical_account);
        self.backend
            .write_raw(&self.service, &scoped, Some(value))?;
        self.cache
            .insert(self.cache_key(logical_account), value.to_string());
        Ok(())
    }

    pub fn delete(&mut self, logical_account: &str) -> Result<(), CoreError> {
        let scoped = self.scoped_account(logical_account);
        self.backend.write_raw(&self.service, &scoped, None)?;
        self.cache.remove(&self.cache_key(logical_account));
        Ok(())
    }
}

impl<B: KeyringBackend> SecretStore for NamespacedSecretStore<B> {
    fn read(&mut self, account: &str) -> Result<Option<String>, String> {
        self.get(account).map_err(|e| e.to_string())
    }

    fn write(&mut self, account: &str, value: Option<&str>) -> Result<(), String> {
        match value {
            Some(secret) => self.set(account, secret),
            None => self.delete(account),
        }
        .map_err(|e| e.to_string())
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

fn check_health(health: &KeyringHealth) -> Result<(), CoreError> {
    match health {
        KeyringHealth::Ready => Ok(()),
        KeyringHealth::Locked => Err(CoreError::capability_unavailable(
            "keyring_locked",
            "OS keyring is locked; unlock it before noninteractive CLI execution",
            "unlock_keyring_and_retry",
        )),
        KeyringHealth::Unavailable(detail) => Err(CoreError::capability_unavailable(
            "keyring_unavailable",
            detail.clone(),
            "configure_credentials",
        )),
    }
}

pub fn ensure_keyring_ready() -> Result<(), CoreError> {
    check_health(&preflight_keyring())
}

fn map_keyring_err(err: KeyringError) -> CoreError {
    match err {
        KeyringError::NoStorageAccess(_) => CoreError::capability_unavailable(
            "keyring_unavailable",
            format!("OS keyring access denied or unavailable: {err}"),
            "configure_credentials",
        ),
        other => CoreError::operation_failed(
            "keyring_error",
            format!("OS keyring failure: {other}"),
            false,
            "unlock_keyring_and_retry",
        ),
    }
}
