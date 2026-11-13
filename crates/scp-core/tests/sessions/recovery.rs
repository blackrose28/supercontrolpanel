use super::*;
use scp_core::host::ForegroundOutcome;
use std::{fs, path::Path, sync::atomic::AtomicBool};

const DENIED: &str = "CLI profile recovery evidence is present or cannot be inspected; ordinary runtime operations are unavailable";
const NAMES: [&str; 2] = [
    "settings-restore-journal.enc",
    "scp-cli-restore-journal.enc",
];

fn publish(path: &Path, kind: &str) {
    match kind {
        "bytes" => fs::write(path, b"private evidence").unwrap(),
        "empty" => fs::write(path, b"").unwrap(),
        "done" => fs::write(path, b"SCP-RESTORE-DONE-1").unwrap(),
        "directory" => fs::create_dir(path).unwrap(),
        #[cfg(unix)]
        "link" => std::os::unix::fs::symlink("profile.json", path).unwrap(),
        #[cfg(unix)]
        "dangling" => std::os::unix::fs::symlink("absent-private-target", path).unwrap(),
        _ => panic!("unknown fixture"),
    }
}

fn ordinary_entry(
    runtime: &mut ProfileRuntime,
    pty: &FakePty,
    id: &str,
    entry: &str,
) -> Result<(), String> {
    let command = CommandBuilder::new("unused");
    match entry {
        "spawn" => runtime.spawn(pty, 80, 24, command).map(|_| ()),
        "write" => runtime.write(id, "private input"),
        "empty" => runtime.write(id, ""),
        "unknown" => runtime.write("unknown", ""),
        "foreground" => runtime
            .run_foreground(pty, 80, 24, command, &mut Vec::new())
            .map(|_| ()),
        "cancel" | "precancel" => runtime
            .run_foreground_cancellable(
                pty,
                (80, 24),
                command,
                &mut Vec::new(),
                &AtomicBool::new(entry == "precancel"),
            )
            .map(|_| ()),
        _ => panic!("unknown entry"),
    }
}

fn preserve_then_remove(artifact: &Path, kind: &str) {
    assert!(fs::symlink_metadata(artifact).is_ok());
    if ["bytes", "empty", "done"].contains(&kind) {
        let expected: &[u8] = match kind {
            "bytes" => b"private evidence",
            "done" => b"SCP-RESTORE-DONE-1",
            _ => b"",
        };
        assert_eq!(fs::read(artifact).unwrap(), expected);
    }
    if kind == "directory" {
        fs::remove_dir(artifact).unwrap();
    } else {
        fs::remove_file(artifact).unwrap();
    }
}

fn assert_independent_profile_is_clean(pty: &FakePty) {
    let independent = tempfile::tempdir().unwrap();
    let mut clean = ProfileRuntime::acquire(independent.path()).unwrap();
    assert!(matches!(
        clean
            .run_foreground_cancellable(
                pty,
                (80, 24),
                CommandBuilder::new("unused"),
                &mut Vec::new(),
                &AtomicBool::new(true)
            )
            .unwrap(),
        ForegroundOutcome::Cancelled
    ));
    assert!(pty.calls().is_empty());
}

#[test]
fn late_evidence_denies_every_ordinary_entry_before_any_backend_effect() {
    for name in NAMES {
        for entry in [
            "spawn",
            "write",
            "empty",
            "unknown",
            "foreground",
            "cancel",
            "precancel",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let mut runtime = ProfileRuntime::acquire(temp.path()).unwrap();
            let pty = FakePty::default();
            let launched = runtime
                .spawn(&pty, 80, 24, CommandBuilder::new("unused"))
                .unwrap();
            let before = pty.calls();
            publish(&temp.path().join(name), "bytes");
            assert_eq!(
                ordinary_entry(&mut runtime, &pty, launched.id(), entry),
                Err(DENIED.into()),
                "{name}: {entry}"
            );
            assert_eq!(pty.calls(), before);
            assert!(pty.trace.lock().unwrap().input.is_empty());
        }
    }
}

#[test]
fn all_evidence_objects_latch_denial_without_changing_profile_or_identity() {
    #[cfg(not(unix))]
    let kinds = ["bytes", "empty", "directory", "done"];
    #[cfg(unix)]
    let kinds = ["bytes", "empty", "directory", "done", "link", "dangling"];
    for name in NAMES {
        for kind in kinds {
            let temp = tempfile::tempdir().unwrap();
            let mut runtime = ProfileRuntime::acquire(temp.path()).unwrap();
            let namespace = runtime.credential_namespace().to_owned();
            let identity = fs::read(temp.path().join("profile.json")).unwrap();
            fs::write(temp.path().join("config.json"), b"original config").unwrap();
            fs::write(temp.path().join("servers.json"), b"original servers").unwrap();
            let artifact = temp.path().join(name);
            publish(&artifact, kind);
            let pty = FakePty::default();
            assert_eq!(runtime.write("unknown", ""), Err(DENIED.into()));
            preserve_then_remove(&artifact, kind);
            for entry in [
                "spawn",
                "write",
                "empty",
                "unknown",
                "foreground",
                "cancel",
                "precancel",
            ] {
                assert_eq!(
                    ordinary_entry(&mut runtime, &pty, "unknown", entry),
                    Err(DENIED.into())
                );
            }
            assert!(pty.calls().is_empty());
            assert_eq!(runtime.credential_namespace(), namespace);
            assert_eq!(
                fs::read(temp.path().join("profile.json")).unwrap(),
                identity
            );
            assert_eq!(
                fs::read(temp.path().join("config.json")).unwrap(),
                b"original config"
            );
            assert_eq!(
                fs::read(temp.path().join("servers.json")).unwrap(),
                b"original servers"
            );
            assert_independent_profile_is_clean(&pty);
        }
    }
}

#[test]
fn observed_denial_keeps_cleanup_and_drop_available_under_ownership() {
    for name in NAMES {
        let temp = tempfile::tempdir().unwrap();
        let mut runtime = ProfileRuntime::acquire(temp.path()).unwrap();
        let pty = FakePty {
            owned_profile: Some(temp.path().to_path_buf()),
            ..FakePty::default()
        };
        let first = runtime
            .spawn(&pty, 80, 24, CommandBuilder::new("unused"))
            .unwrap();
        let second = runtime
            .spawn(&pty, 80, 24, CommandBuilder::new("unused"))
            .unwrap();
        publish(&temp.path().join(name), "bytes");
        assert_eq!(runtime.write(first.id(), ""), Err(DENIED.into()));
        assert!(matches!(
            runtime.poll(&first).unwrap(),
            SessionPoll::Running
        ));
        let exit = runtime.close(first.id()).unwrap().unwrap();
        assert_eq!(exit.id, first.id());
        assert_eq!(exit.reason, "closed_by_command");
        assert_eq!(exit.exit_code, Some(0));
        assert!(runtime.close(first.id()).unwrap().is_none());
        assert!(runtime.close("unknown").unwrap().is_none());
        assert!(matches!(
            runtime.poll(&first).unwrap(),
            SessionPoll::Detached
        ));
        fs::remove_file(temp.path().join(name)).unwrap();
        drop(runtime);
        assert!(pty.calls().ends_with(&["kill", "wait"]));
        let replacement = ProfileRuntime::acquire(temp.path()).unwrap();
        assert!(matches!(
            replacement.poll(&second).unwrap(),
            SessionPoll::Detached
        ));
    }
}

#[test]
fn cleanup_native_errors_keep_their_existing_payloads_after_denial() {
    for failure in ["poll", "wait"] {
        let temp = tempfile::tempdir().unwrap();
        let mut runtime = ProfileRuntime::acquire(temp.path()).unwrap();
        let pty = FakePty {
            fail: failure,
            owned_profile: Some(temp.path().to_path_buf()),
            ..FakePty::default()
        };
        let launched = runtime
            .spawn(&pty, 80, 24, CommandBuilder::new("unused"))
            .unwrap();
        publish(&temp.path().join(NAMES[0]), "bytes");
        assert_eq!(runtime.write(launched.id(), ""), Err(DENIED.into()));
        if failure == "poll" {
            let SessionPoll::Exited(exit) = runtime.poll(&launched).unwrap() else {
                panic!("expected exit")
            };
            assert_eq!(exit.reason, "wait_error: injected poll");
            assert_eq!(exit.exit_code, None);
        } else {
            assert!(runtime
                .close(launched.id())
                .err()
                .unwrap()
                .contains("injected wait"));
            assert!(runtime.close(launched.id()).unwrap().is_none());
        }
        assert!(matches!(
            runtime.poll(&launched).unwrap(),
            SessionPoll::Detached
        ));
        assert!(pty.calls().contains(&"kill"));
        assert!(pty.calls().contains(&"wait"));
    }
}

#[test]
fn relative_profile_selection_survives_later_working_directory_change() {
    let temp = tempfile::tempdir().unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "recovery::relative_profile_fixture",
            "--nocapture",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("RELATIVE_PROFILE_CHECKED"));
}

#[test]
#[ignore = "isolated cwd fixture"]
fn relative_profile_fixture() {
    let selected = std::env::current_dir().unwrap().join("selected");
    let runtime = ProfileRuntime::acquire(Path::new("selected")).unwrap();
    fs::create_dir("other").unwrap();
    std::env::set_current_dir("other").unwrap();
    assert!(runtime
        .write("unknown", "")
        .unwrap_err()
        .contains("session not found"));
    publish(&selected.join(NAMES[1]), "bytes");
    assert_eq!(runtime.write("unknown", ""), Err(DENIED.into()));
    println!("RELATIVE_PROFILE_CHECKED");
}
