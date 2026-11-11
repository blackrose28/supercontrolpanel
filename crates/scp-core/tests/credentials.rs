use scp_core::{
    credentials::{
        CredentialAccount, CredentialCache, ProfileSecrets, SecretBackend, SecretError, SecretValue,
    },
    profile::ProfileOwner,
};
use std::collections::HashMap;

#[derive(Default)]
struct Backend {
    entries: HashMap<(String, String), SecretValue>,
    reads: usize,
    writes: usize,
    error: Option<SecretError>,
}

impl SecretBackend for Backend {
    fn read(&mut self, namespace: &str, account: &str) -> Result<Option<SecretValue>, SecretError> {
        self.reads += 1;
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(self
            .entries
            .get(&(namespace.into(), account.into()))
            .cloned())
    }

    fn write(
        &mut self,
        namespace: &str,
        account: &str,
        value: Option<&str>,
    ) -> Result<(), SecretError> {
        self.writes += 1;
        if let Some(error) = self.error {
            return Err(error);
        }
        let key = (namespace.into(), account.into());
        match value {
            Some(value) => {
                self.entries.insert(key, SecretValue::new(value.into()));
            }
            None => {
                self.entries.remove(&key);
            }
        }
        Ok(())
    }
}

#[test]
fn cache_and_backend_addresses_are_isolated_by_profile_and_account() {
    let temp = tempfile::tempdir().unwrap();
    let first = ProfileOwner::acquire(&temp.path().join("first")).unwrap();
    let second = ProfileOwner::acquire(&temp.path().join("second")).unwrap();
    let mut backend = Backend::default();
    let mut cache = CredentialCache::default();
    let account = CredentialAccount::OpenVpnPassword;
    ProfileSecrets::new(&first, &mut backend, &mut cache)
        .write(&account, Some("first-secret"))
        .unwrap();
    assert!(ProfileSecrets::new(&second, &mut backend, &mut cache)
        .read(&account)
        .unwrap()
        .is_none());
    ProfileSecrets::new(&second, &mut backend, &mut cache)
        .write(&account, Some("second-secret"))
        .unwrap();
    assert_eq!(
        ProfileSecrets::new(&first, &mut backend, &mut cache)
            .read(&account)
            .unwrap()
            .unwrap()
            .expose(),
        "first-secret"
    );
    let ssh = CredentialAccount::ServerSshPassword("server-1".into());
    assert!(ProfileSecrets::new(&first, &mut backend, &mut cache)
        .read(&ssh)
        .unwrap()
        .is_none());
    let fresh = ProfileSecrets::new(&first, &mut backend, &mut cache)
        .read_fresh(&account)
        .unwrap()
        .unwrap();
    assert_eq!(fresh.expose(), "first-secret");
    assert!(backend.entries.contains_key(&(
        first.credential_namespace().into(),
        "openvpn_password".into()
    )));
    assert!(!backend
        .entries
        .keys()
        .any(|(namespace, _)| namespace == "com.example.supercontrolpanel"));
}

#[test]
fn successful_reads_are_cached_and_refresh_observes_external_changes() {
    let temp = tempfile::tempdir().unwrap();
    let owner = ProfileOwner::acquire(temp.path()).unwrap();
    let mut backend = Backend::default();
    let mut cache = CredentialCache::default();
    let account = CredentialAccount::BoundaryPassword;
    for _ in 0..2 {
        assert!(ProfileSecrets::new(&owner, &mut backend, &mut cache)
            .read(&account)
            .unwrap()
            .is_none());
    }
    assert_eq!(backend.reads, 1);
    backend.entries.insert(
        (
            owner.credential_namespace().into(),
            "boundary_password".into(),
        ),
        SecretValue::new("external".into()),
    );
    assert!(ProfileSecrets::new(&owner, &mut backend, &mut cache)
        .read(&account)
        .unwrap()
        .is_none());
    assert_eq!(
        ProfileSecrets::new(&owner, &mut backend, &mut cache)
            .read_fresh(&account)
            .unwrap()
            .unwrap()
            .expose(),
        "external"
    );
    cache.clear();
    ProfileSecrets::new(&owner, &mut backend, &mut cache)
        .read(&account)
        .unwrap();
    assert_eq!(backend.reads, 3);
}

#[test]
fn failures_are_not_missing_or_cached_and_failed_mutations_evict_stale_values() {
    let temp = tempfile::tempdir().unwrap();
    let owner = ProfileOwner::acquire(temp.path()).unwrap();
    let mut backend = Backend::default();
    let mut cache = CredentialCache::default();
    let account = CredentialAccount::BoundaryAppToken;
    ProfileSecrets::new(&owner, &mut backend, &mut cache)
        .write(&account, Some("cached-token"))
        .unwrap();
    backend.error = Some(SecretError::Unavailable);
    assert_eq!(
        ProfileSecrets::new(&owner, &mut backend, &mut cache)
            .read_fresh(&account)
            .unwrap_err(),
        SecretError::Unavailable
    );
    assert_eq!(
        ProfileSecrets::new(&owner, &mut backend, &mut cache)
            .read(&account)
            .unwrap_err(),
        SecretError::Unavailable
    );
    backend.error = None;
    ProfileSecrets::new(&owner, &mut backend, &mut cache)
        .read(&account)
        .unwrap();
    for value in [Some("replacement"), None] {
        backend.error = Some(SecretError::Unavailable);
        assert_eq!(
            ProfileSecrets::new(&owner, &mut backend, &mut cache).write(&account, value),
            Err(SecretError::Unavailable)
        );
        assert_eq!(
            ProfileSecrets::new(&owner, &mut backend, &mut cache)
                .read(&account)
                .unwrap_err(),
            SecretError::Unavailable
        );
        backend.error = None;
        assert_eq!(
            ProfileSecrets::new(&owner, &mut backend, &mut cache)
                .read(&account)
                .unwrap()
                .unwrap()
                .expose(),
            "cached-token"
        );
    }
}

#[test]
fn deletion_is_idempotent_and_empty_value_is_distinct_from_missing() {
    let temp = tempfile::tempdir().unwrap();
    let owner = ProfileOwner::acquire(temp.path()).unwrap();
    let mut backend = Backend::default();
    let mut cache = CredentialCache::default();
    let mut store = ProfileSecrets::new(&owner, &mut backend, &mut cache);
    let account = CredentialAccount::ServerSshPassword("srv-one".into());
    store.write(&account, Some("")).unwrap();
    assert_eq!(store.read(&account).unwrap().unwrap().expose(), "");
    for _ in 0..2 {
        store.write(&account, None).unwrap();
        assert!(store.read(&account).unwrap().is_none());
    }
}

#[test]
fn account_mapping_is_explicit_and_invalid_server_accounts_never_reach_backend() {
    use CredentialAccount::*;
    let temp = tempfile::tempdir().unwrap();
    let owner = ProfileOwner::acquire(temp.path()).unwrap();
    let mut backend = Backend::default();
    let mut cache = CredentialCache::default();
    for (account, expected) in [
        (OpenVpnPassword, "openvpn_password"),
        (OpenVpnOtpSecret, "openvpn_otp_secret"),
        (BoundaryPassword, "boundary_password"),
        (BoundaryOidcPassword, "boundary_oidc_password"),
        (BoundaryOidcOtpSecret, "boundary_oidc_otp_secret"),
        (BoundaryAppToken, "boundary_app_token"),
        (
            ServerSshPassword("srv-1".into()),
            "server_ssh_password:srv-1",
        ),
    ] {
        ProfileSecrets::new(&owner, &mut backend, &mut cache)
            .write(&account, Some("test-value"))
            .unwrap();
        assert!(backend
            .entries
            .contains_key(&(owner.credential_namespace().into(), expected.into())));
    }
    for id in [
        "".to_string(),
        "bad\naccount".into(),
        "bad:account".into(),
        "x".repeat(257),
    ] {
        let mut store = ProfileSecrets::new(&owner, &mut backend, &mut cache);
        let account = ServerSshPassword(id);
        assert_eq!(
            store.read(&account).unwrap_err(),
            SecretError::InvalidAccount
        );
        assert_eq!(
            store.write(&account, Some("value")),
            Err(SecretError::InvalidAccount)
        );
    }
    assert_eq!(backend.writes, 7);
    assert_eq!(backend.reads, 0);
}

#[test]
fn secret_debug_output_is_redacted() {
    let value = SecretValue::new("private-value".into());
    assert!(!format!("{value:?}").contains("private-value"));
    assert_eq!(value.clone().expose(), "private-value");
}

#[cfg(feature = "os-keyring")]
#[test]
fn native_errors_are_classified_without_printing_native_error_contents() {
    use keyring::Error;
    for (native, expected) in [
        (
            Error::NoStorageAccess(Box::new(std::io::Error::other("private-native-data"))),
            SecretError::Unavailable,
        ),
        (
            Error::PlatformFailure(Box::new(std::io::Error::other("private-native-data"))),
            SecretError::Unavailable,
        ),
        (
            Error::BadEncoding(b"private-native-data".to_vec()),
            SecretError::InvalidData,
        ),
        (Error::Ambiguous(vec![]), SecretError::Ambiguous),
        (
            Error::Invalid("private-native-data".into(), "private-native-data".into()),
            SecretError::InvalidAccount,
        ),
        (
            Error::TooLong("private-native-data".into(), 10),
            SecretError::InvalidAccount,
        ),
    ] {
        let error = SecretError::from(native);
        assert_eq!(error, expected);
        assert!(!format!("{error} {error:?}").contains("private-native-data"));
    }
}
