use super::*;
use std::{fs, path::Path};

const DENIED: &str = "CLI profile recovery evidence is present or cannot be inspected; ordinary credential operations are unavailable";
const NAMES: [&str; 2] = [
    "settings-restore-journal.enc",
    "scp-cli-restore-journal.enc",
];

#[test]
fn recovery_error_is_distinct_and_fixed() {
    let error = SecretError::RecoveryRequired;
    assert_eq!(error.to_string(), DENIED);
    assert_eq!(format!("{error:?}"), "RecoveryRequired");
    for native in [
        SecretError::Unavailable,
        SecretError::InvalidAccount,
        SecretError::InvalidData,
        SecretError::Ambiguous,
    ] {
        assert_ne!(error, native);
    }
}

fn attempt(store: &mut ProfileSecrets<'_, Backend>, entry: &str) -> Result<(), SecretError> {
    let account = if entry.starts_with("invalid") {
        CredentialAccount::ServerSshPassword("bad:account".into())
    } else {
        CredentialAccount::OpenVpnPassword
    };
    match entry {
        "read" | "invalid-read" => store.read(&account).map(|_| ()),
        "fresh" | "invalid-fresh" => store.read_fresh(&account).map(|_| ()),
        "set" | "invalid-set" => store.write(&account, Some("replacement")),
        "delete" | "invalid-delete" => store.write(&account, None),
        _ => panic!("unknown entry"),
    }
}

#[test]
fn late_evidence_denies_cached_values_and_every_entry_before_backend_or_validation() {
    for name in NAMES {
        for value in [Some("cached"), Some(""), None] {
            for entry in [
                "read",
                "fresh",
                "set",
                "delete",
                "invalid-read",
                "invalid-fresh",
                "invalid-set",
                "invalid-delete",
            ] {
                let temp = tempfile::tempdir().unwrap();
                let owner = ProfileOwner::acquire(temp.path()).unwrap();
                let mut backend = Backend::default();
                let mut cache = CredentialCache::default();
                let mut store = ProfileSecrets::new(&owner, &mut backend, &mut cache);
                store
                    .write(&CredentialAccount::OpenVpnPassword, value)
                    .unwrap();
                fs::write(temp.path().join(name), b"private evidence").unwrap();
                assert_eq!(
                    attempt(&mut store, entry).map_err(|e| e.to_string()),
                    Err(DENIED.into()),
                    "{name} {entry}"
                );
                fs::remove_file(temp.path().join(name)).unwrap();
                assert_eq!(
                    attempt(&mut store, entry).map_err(|e| e.to_string()),
                    Err(DENIED.into())
                );
                assert_eq!((backend.reads, backend.writes), (0, 1));
                cache.clear();
                assert_eq!(
                    ProfileSecrets::new(&owner, &mut backend, &mut cache)
                        .read(&CredentialAccount::OpenVpnPassword)
                        .unwrap_err()
                        .to_string(),
                    DENIED
                );
                assert_eq!((backend.reads, backend.writes), (0, 1));
            }
        }
    }
}

fn publish(path: &Path, kind: &str) {
    match kind {
        "empty" => fs::write(path, b"").unwrap(),
        "done" => fs::write(path, b"SCP-RESTORE-DONE-1").unwrap(),
        "directory" => fs::create_dir(path).unwrap(),
        #[cfg(unix)]
        "link" => std::os::unix::fs::symlink("profile.json", path).unwrap(),
        #[cfg(unix)]
        "dangling" => std::os::unix::fs::symlink("missing-private-target", path).unwrap(),
        _ => panic!("unknown fixture"),
    }
}

#[test]
fn every_recovery_object_denies_without_reading_or_changing_it() {
    let mut kinds = vec!["empty", "done", "directory"];
    if cfg!(unix) {
        kinds.extend(["link", "dangling"]);
    }
    for name in NAMES {
        for kind in &kinds {
            let temp = tempfile::tempdir().unwrap();
            let owner = ProfileOwner::acquire(temp.path()).unwrap();
            let identity = fs::read(temp.path().join("profile.json")).unwrap();
            let artifact = temp.path().join(name);
            publish(&artifact, kind);
            let mut backend = Backend::default();
            let mut cache = CredentialCache::default();
            assert_eq!(
                ProfileSecrets::new(&owner, &mut backend, &mut cache)
                    .read(&CredentialAccount::OpenVpnPassword)
                    .unwrap_err()
                    .to_string(),
                DENIED
            );
            assert_eq!((backend.reads, backend.writes), (0, 0));
            assert!(fs::symlink_metadata(&artifact).is_ok());
            assert_eq!(
                fs::read(temp.path().join("profile.json")).unwrap(),
                identity
            );
            if *kind == "done" {
                assert_eq!(fs::read(&artifact).unwrap(), b"SCP-RESTORE-DONE-1");
            }
            if *kind == "empty" {
                assert!(fs::read(&artifact).unwrap().is_empty());
            }
            if ["link", "dangling"].contains(kind) {
                assert!(fs::read_link(&artifact).is_ok());
            }
        }
    }
}

#[test]
fn denial_evicts_entire_namespace_but_preserves_independent_cached_profile() {
    let temp = tempfile::tempdir().unwrap();
    let first_dir = temp.path().join("first");
    let first = ProfileOwner::acquire(&first_dir).unwrap();
    let second = ProfileOwner::acquire(&temp.path().join("second")).unwrap();
    let namespace = first.credential_namespace().to_string();
    let mut backend = Backend::default();
    let mut cache = CredentialCache::default();
    let accounts = [
        CredentialAccount::OpenVpnPassword,
        CredentialAccount::BoundaryPassword,
        CredentialAccount::ServerSshPassword("srv-1".into()),
    ];
    for (account, value) in accounts.iter().zip([Some("cached"), Some(""), None]) {
        ProfileSecrets::new(&first, &mut backend, &mut cache)
            .write(account, value)
            .unwrap();
    }
    ProfileSecrets::new(&second, &mut backend, &mut cache)
        .write(&accounts[0], Some("independent"))
        .unwrap();
    fs::write(first_dir.join(NAMES[0]), b"evidence").unwrap();
    assert_eq!(
        attempt(
            &mut ProfileSecrets::new(&first, &mut backend, &mut cache),
            "invalid-delete"
        )
        .unwrap_err()
        .to_string(),
        DENIED
    );
    fs::remove_file(first_dir.join(NAMES[0])).unwrap();
    assert!(ProfileOwner::acquire(&first_dir).is_err());
    drop(first);
    let replacement = ProfileOwner::acquire(&first_dir).unwrap();
    assert_eq!(replacement.credential_namespace(), namespace);
    backend.error = Some(SecretError::Unavailable);
    for account in &accounts {
        assert_eq!(
            ProfileSecrets::new(&replacement, &mut backend, &mut cache)
                .read(account)
                .unwrap_err(),
            SecretError::Unavailable
        );
    }
    assert_eq!(
        ProfileSecrets::new(&second, &mut backend, &mut cache)
            .read(&accounts[0])
            .unwrap()
            .unwrap()
            .expose(),
        "independent"
    );
    assert_eq!((backend.reads, backend.writes), (3, 4));
}

struct OwnershipCheckingBackend<'a> {
    directory: &'a Path,
    inner: Backend,
}

impl SecretBackend for OwnershipCheckingBackend<'_> {
    fn read(&mut self, namespace: &str, account: &str) -> Result<Option<SecretValue>, SecretError> {
        assert!(
            ProfileOwner::acquire(self.directory).is_err(),
            "owner retained during native read"
        );
        self.inner.read(namespace, account)
    }

    fn write(
        &mut self,
        namespace: &str,
        account: &str,
        value: Option<&str>,
    ) -> Result<(), SecretError> {
        assert!(
            ProfileOwner::acquire(self.directory).is_err(),
            "owner retained during native mutation"
        );
        self.inner.write(namespace, account, value)
    }
}

#[test]
fn owner_lock_is_retained_through_native_reads_sets_deletes_and_failures() {
    let temp = tempfile::tempdir().unwrap();
    let owner = ProfileOwner::acquire(temp.path()).unwrap();
    let mut backend = OwnershipCheckingBackend {
        directory: temp.path(),
        inner: Backend::default(),
    };
    let mut cache = CredentialCache::default();
    let account = CredentialAccount::OpenVpnPassword;
    for error in [None, Some(SecretError::Unavailable)] {
        backend.inner.error = error;
        let mut store = ProfileSecrets::new(&owner, &mut backend, &mut cache);
        assert_eq!(
            store.read_fresh(&account).map(|_| ()),
            error.map_or(Ok(()), Err)
        );
        for value in [Some(""), None] {
            assert_eq!(store.write(&account, value), error.map_or(Ok(()), Err));
        }
    }
    assert_eq!((backend.inner.reads, backend.inner.writes), (2, 4));
    drop(owner);
    assert!(ProfileOwner::acquire(temp.path()).is_ok());
}
