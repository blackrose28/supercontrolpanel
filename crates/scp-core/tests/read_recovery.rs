use scp_core::{config::read_shared_config, servers::read_servers};
use std::{fs, path::Path};

const ERROR: &str = "CLI profile recovery evidence is present or cannot be inspected; ordinary reads are unavailable";
const NAMES: [&str; 2] = [
    "settings-restore-journal.enc",
    "scp-cli-restore-journal.enc",
];

fn denied(root: &Path) {
    assert_eq!(
        read_shared_config(root).err().expect("read must deny"),
        ERROR
    );
    assert_eq!(read_servers(root).unwrap_err(), ERROR);
    assert!(!root.join("runtime-owner.lock").exists());
}

fn settings(root: &Path, state: &str) {
    for name in ["config.json", "servers.json"] {
        let path = root.join(name);
        match state {
            "missing" => {}
            "directory" => fs::create_dir(path).unwrap(),
            "valid" => {
                let raw = if name == "config.json" {
                    r#"{"app":{"defaultTerminalCols":80,"defaultTerminalRows":24}}"#
                } else {
                    r#"{"servers":[]}"#
                };
                fs::write(path, raw).unwrap();
            }
            _ => fs::write(path, state).unwrap(),
        }
    }
    fs::write(root.join("sync-local.json"), "private local settings").unwrap();
}

fn snapshot(root: &Path) -> Vec<(std::ffi::OsString, Option<Vec<u8>>)> {
    let mut entries: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).ok())
        })
        .collect();
    entries.sort();
    entries
}

#[test]
fn recovery_objects_precede_missing_malformed_and_unreadable_settings_without_mutation() {
    for name in NAMES {
        for state in ["missing", "private invalid settings", "directory", "valid"] {
            assert_object_case(name, state, false);
            assert_object_case(name, state, true);
        }
    }
}

fn assert_object_case(name: &str, state: &str, identity: bool) {
    let temp = tempfile::tempdir().unwrap();
    settings(temp.path(), state);
    if identity {
        fs::write(temp.path().join("profile.json"), "private invalid identity").unwrap();
    }
    let path = temp.path().join(name);
    fs::write(&path, "private evidence").unwrap();
    let before = snapshot(temp.path());
    denied(temp.path());
    assert_eq!(snapshot(temp.path()), before);
    assert_eq!(temp.path().join("profile.json").exists(), identity);
}

#[test]
fn similar_names_do_not_block_clean_missing_reads_or_initialize_profiles() {
    let temp = tempfile::tempdir().unwrap();
    for name in NAMES {
        fs::write(temp.path().join(format!("{name}.old")), "retained").unwrap();
    }
    assert!(read_shared_config(temp.path()).unwrap().is_none());
    assert!(read_servers(temp.path()).unwrap().is_empty());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}

#[test]
fn inventory_direct_reader_denies_recovery_before_missing_file_default() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join(NAMES[1]), "private bytes").unwrap();
    assert_eq!(read_servers(temp.path()).unwrap_err(), ERROR);
}
