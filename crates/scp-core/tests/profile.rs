use scp_core::profile::ProfileOwner;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};

#[test]
fn initialization_is_stable_and_profiles_have_distinct_namespaces() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let owner = ProfileOwner::acquire(&first).unwrap();
    let namespace = owner.credential_namespace().to_owned();
    assert!(namespace.starts_with("supercontrolpanel-cli-"));
    let original = fs::read(first.join("profile.json")).unwrap();
    drop(owner);
    let owner = ProfileOwner::acquire(&first).unwrap();
    assert_eq!(owner.credential_namespace(), namespace);
    assert_eq!(fs::read(first.join("profile.json")).unwrap(), original);
    let second = ProfileOwner::acquire(&temp.path().join("second")).unwrap();
    assert_ne!(second.credential_namespace(), namespace);
    assert!(!first.join("config.json").exists());
    assert!(!first.join("servers.json").exists());
}

#[test]
fn malformed_or_unsupported_metadata_fails_closed_without_replacement() {
    let temp = tempfile::tempdir().unwrap();
    for raw in [
        "private-invalid-data",
        r#"{"version":99,"credentialNamespace":"x"}"#,
        r#"{"version":1,"credentialNamespace":"supercontrolpanel"}"#,
    ] {
        fs::write(temp.path().join("profile.json"), raw).unwrap();
        let error = ProfileOwner::acquire(temp.path())
            .err()
            .expect("must reject metadata");
        assert!(error.contains("profile"));
        assert!(!error.contains("private-invalid-data"));
        assert_eq!(
            fs::read_to_string(temp.path().join("profile.json")).unwrap(),
            raw
        );
    }
    fs::remove_file(temp.path().join("profile.json")).unwrap();
    assert!(ProfileOwner::acquire(temp.path()).is_ok());
}

#[test]
fn owner_lock_conflicts_across_processes_and_releases_after_exit_or_kill() {
    let temp = tempfile::tempdir().unwrap();
    for kill in [false, true] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "owner_fixture", "--nocapture"])
            .env("SCP_TEST_PROFILE", temp.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            assert!(
                output.read_line(&mut line).unwrap() > 0,
                "fixture exited before locking"
            );
            if line.contains("OWNER_READY") {
                break;
            }
            line.clear();
        }
        let error = ProfileOwner::acquire(&temp.path().join("."))
            .err()
            .expect("second owner must fail");
        assert!(error.contains("already owned"), "{error}");
        if kill {
            child.kill().unwrap();
        } else {
            drop(child.stdin.take());
        }
        child.wait().unwrap();
        assert!(ProfileOwner::acquire(temp.path()).is_ok());
    }
}

#[test]
#[ignore = "subprocess fixture for owner lifetime tests"]
fn owner_fixture() {
    let root = std::env::var_os("SCP_TEST_PROFILE").unwrap();
    let _owner = ProfileOwner::acquire(std::path::Path::new(&root)).unwrap();
    println!("OWNER_READY");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
}

#[test]
#[cfg(unix)]
fn new_profile_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("profile");
    let _owner = ProfileOwner::acquire(&root).unwrap();
    for path in [
        root.clone(),
        root.join("profile.json"),
        root.join("runtime-owner.lock"),
    ] {
        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
    }
}

const RECOVERY_ERROR: &str = "CLI profile recovery evidence is present or cannot be inspected; ordinary ownership is unavailable";
const JOURNALS: [&str; 2] = [
    "settings-restore-journal.enc",
    "scp-cli-restore-journal.enc",
];

fn assert_recovery_denied(root: &std::path::Path) {
    // Exercise the ordinary core file boundaries before owner acquisition creates a lock.
    let before_lock = fs::read(root.join("runtime-owner.lock")).ok();
    assert_eq!(scp_core::config::read_shared_config(root).err().unwrap(), "CLI profile recovery evidence is present or cannot be inspected; ordinary reads are unavailable");
    assert_eq!(scp_core::servers::read_servers(root).unwrap_err(), "CLI profile recovery evidence is present or cannot be inspected; ordinary reads are unavailable");
    assert_eq!(fs::read(root.join("runtime-owner.lock")).ok(), before_lock);
    assert_eq!(
        scp_core::host::ProfileRuntime::acquire(root)
            .err()
            .expect("runtime must deny recovery evidence"),
        RECOVERY_ERROR
    );
    assert_eq!(
        ProfileOwner::acquire(root)
            .err()
            .expect("owner must deny recovery evidence"),
        RECOVERY_ERROR
    );
    // Denial releases the OS lock but never removes its persistent file.
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("runtime-owner.lock"))
        .unwrap();
    lock.try_lock().expect("denial must release ownership");
    lock.unlock().unwrap();
}

#[test]
fn recovery_artifact_objects_deny_owner_and_runtime_without_changing_profile() {
    for name in JOURNALS {
        for kind in ["bytes", "empty", "directory", "tombstone"] {
            for existing_identity in [false, true] {
                assert_artifact_case(name, kind, existing_identity);
            }
        }
    }
}

fn assert_artifact_case(name: &str, kind: &str, existing_identity: bool) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let identity = existing_identity.then(|| {
        drop(ProfileOwner::acquire(root).unwrap());
        fs::read(root.join("profile.json")).unwrap()
    });
    for file in ["config.json", "servers.json", "sync-local.json"] {
        fs::write(root.join(file), b"unchanged private settings").unwrap();
    }
    let artifact = root.join(name);
    if kind == "directory" {
        fs::create_dir(&artifact).unwrap();
    } else {
        let contents: &[u8] = match kind {
            "bytes" => b"private recovery contents",
            "tombstone" => b"SCP-RESTORE-DONE-1",
            _ => b"",
        };
        fs::write(&artifact, contents).unwrap();
    }
    let before = fs::read(&artifact).ok();
    assert_recovery_denied(root);
    assert_eq!(fs::read(root.join("profile.json")).ok(), identity);
    assert_eq!(fs::read(&artifact).ok(), before);
    assert_eq!(
        fs::symlink_metadata(&artifact).unwrap().is_dir(),
        kind == "directory"
    );
    for file in ["config.json", "servers.json", "sync-local.json"] {
        assert_eq!(
            fs::read(root.join(file)).unwrap(),
            b"unchanged private settings"
        );
    }
}

#[test]
#[cfg(unix)]
fn recovery_symlinks_including_dangling_targets_are_denied_without_following() {
    for name in JOURNALS {
        for dangling in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let target = temp.path().join("private-target-name");
            if !dangling {
                fs::write(&target, b"private target contents").unwrap();
            }
            let artifact = temp.path().join(name);
            std::os::unix::fs::symlink(&target, &artifact).unwrap();
            assert_recovery_denied(temp.path());
            assert!(!temp.path().join("profile.json").exists());
            assert_eq!(fs::read_link(&artifact).unwrap(), target);
            assert_eq!(
                fs::read(&target).ok(),
                (!dangling).then(|| b"private target contents".to_vec())
            );
        }
    }
}

#[test]
fn competing_owner_conflict_precedes_recovery_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let owner = ProfileOwner::acquire(temp.path()).unwrap();
    for name in JOURNALS {
        fs::write(temp.path().join(name), b"private journal").unwrap();
    }
    let error = ProfileOwner::acquire(temp.path()).err().unwrap();
    assert!(error.contains("already owned"), "{error}");
    drop(owner);
    assert_recovery_denied(temp.path());
}
