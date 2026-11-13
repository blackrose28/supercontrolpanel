use super::ProfileOwner;
use crate::credentials::*;
use std::{
    cell::Cell,
    fs,
    io::{Error, ErrorKind},
};

const DENIED: &str = "CLI profile recovery evidence is present or cannot be inspected; ordinary runtime operations are unavailable";

struct NoBackendAccess;

impl SecretBackend for NoBackendAccess {
    fn read(&mut self, _: &str, _: &str) -> Result<Option<SecretValue>, SecretError> {
        panic!("revoked owner must not read native credentials")
    }
    fn write(&mut self, _: &str, _: &str, _: Option<&str>) -> Result<(), SecretError> {
        panic!("revoked owner must not mutate native credentials")
    }
}

#[test]
fn inspection_denial_is_ordered_fixed_and_permanently_latched() {
    for (index, name) in [
        "settings-restore-journal.enc",
        "scp-cli-restore-journal.enc",
    ]
    .iter()
    .enumerate()
    {
        for kind in [ErrorKind::PermissionDenied, ErrorKind::Other] {
            let temp = tempfile::tempdir().unwrap();
            let owner = ProfileOwner::acquire(temp.path()).unwrap();
            let inspected = std::cell::RefCell::new(Vec::new());
            assert_eq!(
                owner.ensure_runtime_with_inspector(|path| {
                    inspected.borrow_mut().push(path.to_path_buf());
                    let fault = if path.ends_with(name) {
                        kind
                    } else {
                        ErrorKind::NotFound
                    };
                    Err(Error::new(fault, "private detail"))
                }),
                Err(DENIED.into())
            );
            let expected: Vec<_> = [
                "settings-restore-journal.enc",
                "scp-cli-restore-journal.enc",
            ]
            .iter()
            .take(index + 1)
            .map(|name| temp.path().join(name))
            .collect();
            assert_eq!(*inspected.borrow(), expected);
            assert_eq!(
                owner.ensure_runtime_with_inspector(|_| panic!(
                    "revoked owner must not inspect again"
                )),
                Err(DENIED.into())
            );
            assert_eq!(owner.ensure_runtime_operations(), Err(DENIED.into()));
        }
    }
}

#[test]
fn metadata_success_denies_at_either_position_and_clean_checks_do_not_latch() {
    for denied_at in [1, 2] {
        let temp = tempfile::tempdir().unwrap();
        let owner = ProfileOwner::acquire(temp.path()).unwrap();
        let calls = Cell::new(0);
        owner
            .ensure_runtime_with_inspector(|_| {
                calls.set(calls.get() + 1);
                Err(Error::from(ErrorKind::NotFound))
            })
            .unwrap();
        assert_eq!(calls.get(), 2);
        calls.set(0);
        assert_eq!(
            owner.ensure_runtime_with_inspector(|path| {
                calls.set(calls.get() + 1);
                assert_eq!(
                    path.file_name().unwrap(),
                    [
                        "settings-restore-journal.enc",
                        "scp-cli-restore-journal.enc"
                    ][calls.get() - 1]
                );
                if calls.get() == denied_at {
                    fs::symlink_metadata(temp.path())
                } else {
                    Err(Error::from(ErrorKind::NotFound))
                }
            }),
            Err(DENIED.into())
        );
        assert_eq!(calls.get(), denied_at);
        assert_eq!(owner.ensure_runtime_operations(), Err(DENIED.into()));
    }
}

#[test]
fn credential_entries_share_inspection_error_and_runtime_revocation_latch() {
    for name in [
        "settings-restore-journal.enc",
        "scp-cli-restore-journal.enc",
    ] {
        for kind in [ErrorKind::PermissionDenied, ErrorKind::Other] {
            let temp = tempfile::tempdir().unwrap();
            let owner = ProfileOwner::acquire(temp.path()).unwrap();
            assert!(owner
                .ensure_runtime_with_inspector(|path| {
                    Err(Error::new(
                        if path.ends_with(name) {
                            kind
                        } else {
                            ErrorKind::NotFound
                        },
                        "private native detail",
                    ))
                })
                .is_err());
            let mut backend = NoBackendAccess;
            let mut cache = CredentialCache::default();
            let mut store = ProfileSecrets::new(&owner, &mut backend, &mut cache);
            let invalid = CredentialAccount::ServerSshPassword("bad:account".into());
            assert_eq!(
                store.read(&invalid).unwrap_err(),
                SecretError::RecoveryRequired
            );
            assert_eq!(
                store.read_fresh(&invalid).unwrap_err(),
                SecretError::RecoveryRequired
            );
            for value in [Some("private value"), None] {
                assert_eq!(
                    store.write(&invalid, value),
                    Err(SecretError::RecoveryRequired)
                );
            }
        }
    }
}

#[test]
fn credential_observation_revokes_runtime_even_after_evidence_removal() {
    let temp = tempfile::tempdir().unwrap();
    let owner = ProfileOwner::acquire(temp.path()).unwrap();
    let artifact = temp.path().join("scp-cli-restore-journal.enc");
    fs::write(&artifact, b"evidence").unwrap();
    // Invalid input still observes recovery before account validation, with no native adapter.
    let mut backend = NoBackendAccess;
    let mut cache = CredentialCache::default();
    assert_eq!(
        ProfileSecrets::new(&owner, &mut backend, &mut cache)
            .read(&CredentialAccount::ServerSshPassword("".into()))
            .unwrap_err(),
        SecretError::RecoveryRequired
    );
    fs::remove_file(artifact).unwrap();
    assert_eq!(
        owner.ensure_runtime_with_inspector(|_| panic!("latch must skip inspection")),
        Err(DENIED.into())
    );
}
