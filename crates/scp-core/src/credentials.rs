//! Profile-bound credentials. Hosts retain ownership while using this store; no global cache.

use crate::profile::ProfileOwner;
use std::{collections::HashMap, fmt};
use zeroize::Zeroizing;

#[cfg(feature = "os-keyring")]
mod os;
#[cfg(feature = "os-keyring")]
pub use os::OsSecretBackend;

/// Explicit credential accounts; caller input cannot select arbitrary OS accounts.
pub enum CredentialAccount {
    OpenVpnPassword,
    OpenVpnOtpSecret,
    BoundaryPassword,
    BoundaryOidcPassword,
    BoundaryOidcOtpSecret,
    BoundaryAppToken,
    ServerSshPassword(String),
}

impl CredentialAccount {
    fn account_name(&self) -> Result<String, SecretError> {
        let name = match self {
            Self::OpenVpnPassword => "openvpn_password",
            Self::OpenVpnOtpSecret => "openvpn_otp_secret",
            Self::BoundaryPassword => "boundary_password",
            Self::BoundaryOidcPassword => "boundary_oidc_password",
            Self::BoundaryOidcOtpSecret => "boundary_oidc_otp_secret",
            Self::BoundaryAppToken => "boundary_app_token",
            Self::ServerSshPassword(id) => {
                if id.is_empty()
                    || id.len() > 256
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                {
                    return Err(SecretError::InvalidAccount);
                }
                return Ok(format!("server_ssh_password:{id}"));
            }
        };
        Ok(name.into())
    }
}

/// Owned secret: debug output is redacted and owned storage is zeroized on drop.
#[derive(Clone)]
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    /// Deliberately expose the value only at the point an operation needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}

/// Sanitized failures. Missing credentials are `Ok(None)`, never an error or empty value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretError {
    Unavailable,
    InvalidAccount,
    InvalidData,
    Ambiguous,
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => {
                "OS credential store is locked or unavailable; unlock or enable the store and retry"
            }
            Self::InvalidAccount => "invalid credential account",
            Self::InvalidData => "stored credential has invalid encoding",
            Self::Ambiguous => {
                "multiple matching credentials found; resolve duplicate entries in the OS store"
            }
        })
    }
}

impl std::error::Error for SecretError {}

/// Trusted host adapter, addressed by both namespace (OS service) and account.
pub trait SecretBackend {
    fn read(&mut self, namespace: &str, account: &str) -> Result<Option<SecretValue>, SecretError>;
    /// `None` deletes idempotently; `Some("")` stores an explicitly empty value.
    fn write(
        &mut self,
        namespace: &str,
        account: &str,
        value: Option<&str>,
    ) -> Result<(), SecretError>;
}

/// Successful reads (including missing entries) only. Each key includes its profile namespace.
#[derive(Default)]
pub struct CredentialCache {
    entries: HashMap<(String, String), Option<SecretValue>>,
}

impl CredentialCache {
    /// Discard cached values after external maintenance or before releasing runtime state.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Borrowing the owner prevents releasing the profile lock while credentials are in use.
///
/// ```compile_fail
/// use scp_core::{credentials::*, profile::ProfileOwner};
/// fn release_too_early<B: SecretBackend>(owner: ProfileOwner, backend: &mut B, cache: &mut CredentialCache) {
///     let mut secrets = ProfileSecrets::new(&owner, backend, cache);
///     drop(owner);
///     secrets.read(&CredentialAccount::OpenVpnPassword).unwrap();
/// }
/// ```
pub struct ProfileSecrets<'a, B> {
    owner: &'a ProfileOwner,
    backend: &'a mut B,
    cache: &'a mut CredentialCache,
}

impl<'a, B: SecretBackend> ProfileSecrets<'a, B> {
    pub fn new(
        owner: &'a ProfileOwner,
        backend: &'a mut B,
        cache: &'a mut CredentialCache,
    ) -> Self {
        Self {
            owner,
            backend,
            cache,
        }
    }

    pub fn read(
        &mut self,
        account: &CredentialAccount,
    ) -> Result<Option<SecretValue>, SecretError> {
        let key = self.key(account)?;
        if let Some(value) = self.cache.entries.get(&key) {
            return Ok(value.clone());
        }
        let value = self.backend.read(&key.0, &key.1)?;
        self.cache.entries.insert(key, value.clone());
        Ok(value)
    }

    /// Bypass and evict any stale entry, including when a fresh read fails.
    pub fn read_fresh(
        &mut self,
        account: &CredentialAccount,
    ) -> Result<Option<SecretValue>, SecretError> {
        let key = self.key(account)?;
        self.cache.entries.remove(&key);
        self.read(account)
    }

    /// Writes reach the OS first. Any failure evicts old cache state: a native failure
    /// can have an unknown outcome, so the next read must consult the store again.
    pub fn write(
        &mut self,
        account: &CredentialAccount,
        value: Option<&str>,
    ) -> Result<(), SecretError> {
        let key = self.key(account)?;
        self.cache.entries.remove(&key);
        self.backend.write(&key.0, &key.1, value)?;
        self.cache
            .entries
            .insert(key, value.map(|s| SecretValue::new(s.into())));
        Ok(())
    }

    fn key(&self, account: &CredentialAccount) -> Result<(String, String), SecretError> {
        Ok((
            self.owner.credential_namespace().into(),
            account.account_name()?,
        ))
    }
}
