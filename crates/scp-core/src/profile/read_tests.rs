use super::ensure_readable_with_inspector;
use std::{
    cell::Cell,
    io::{Error, ErrorKind},
};

#[test]
fn inspection_failures_are_fixed_and_only_not_found_allows_reads() {
    for denied_at in [1, 2] {
        for kind in [ErrorKind::PermissionDenied, ErrorKind::Other] {
            let temp = tempfile::tempdir().unwrap();
            let calls = Cell::new(0);
            let result = ensure_readable_with_inspector(temp.path(), |path| {
                calls.set(calls.get() + 1);
                let name = [
                    "settings-restore-journal.enc",
                    "scp-cli-restore-journal.enc",
                ][calls.get() - 1];
                assert_eq!(path, temp.path().join(name));
                let fault = if calls.get() == denied_at {
                    kind
                } else {
                    ErrorKind::NotFound
                };
                Err(Error::new(fault, "private native detail"))
            });
            assert_eq!(result.unwrap_err(), "CLI profile recovery evidence is present or cannot be inspected; ordinary reads are unavailable");
            assert_eq!(calls.get(), denied_at);
            assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        }
    }
}
