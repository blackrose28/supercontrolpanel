use super::ProfileOwner;
use std::{
    cell::Cell,
    fs,
    io::{Error, ErrorKind},
};

const DENIED: &str = "CLI profile recovery evidence is present or cannot be inspected; ordinary runtime operations are unavailable";

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
