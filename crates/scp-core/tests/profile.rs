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
