use super::*;
use serde_json::json;
use std::collections::BTreeMap;
static GATE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn fixture() -> Archive {
    Archive {
        schema: 1,
        app_version: "test".into(),
        platform: "foreign".into(),
        created_at_ms: 1,
        files: BTreeMap::from([
            (
                "config.json".into(),
                Some(
                    json!({"app":{"defaultTerminalCols":120,"defaultTerminalRows":36},"openvpn":{"extraArgs":"--verb 3"}}),
                ),
            ),
            ("servers.json".into(), Some(json!({"servers":[]}))),
            (
                "sync-local.json".into(),
                Some(
                    json!({"mcpEnabled":true,"ccEnabled":true,"ccAgentToken":"source-token","ccDeviceId":"source-id","mcpAuthToken":"source-mcp","repoRoot":"/source"}),
                ),
            ),
        ]),
        credentials: known_accounts(&[]).into_iter().map(|a| (a, None)).collect(),
        external_files: vec![],
    }
}

#[test]
fn backup_round_trip_is_randomized_and_never_contains_plaintext() {
    let mut archive = fixture();
    archive.credentials.insert(
        "openvpn_password".into(),
        Some("very-private-test-password".into()),
    );
    let first = crypto::encrypt(&archive, "backup passphrase").unwrap();
    let second = crypto::encrypt(&archive, "backup passphrase").unwrap();
    assert_ne!(first, second);
    assert!(!String::from_utf8_lossy(&first).contains("very-private-test-password"));
    let decoded: Archive = crypto::decrypt(&first, "backup passphrase").unwrap();
    assert_eq!(
        decoded.credentials["openvpn_password"].as_deref(),
        Some("very-private-test-password")
    );
    assert!(crypto::decrypt::<Archive>(&first, "wrong password").is_err());
    for index in [0, 12, first.len() - 1] {
        let mut tampered = first.clone();
        tampered[index] ^= 1;
        assert!(crypto::decrypt::<Archive>(&tampered, "backup passphrase").is_err());
    }
}

#[test]
fn backup_rejects_unbounded_headers_before_kdf() {
    assert!(crypto::decrypt::<Archive>(&[255; 200], "pass").is_err());
    assert!(crypto::validate_cost(1024 * 1024, 1, 1).is_err());
    assert!(crypto::validate_cost(65536, 100, 1).is_err());
    assert!(crypto::validate_cost(65536, 3, 200).is_err());
}

#[test]
fn backup_validation_rejects_unknown_accounts_duplicate_ids_and_new_schema() {
    let mut archive = fixture();
    archive
        .credentials
        .insert("arbitrary-other-service".into(), Some("bad".into()));
    assert!(archive.validate().is_err());
    let mut archive = fixture();
    archive.schema = 2;
    assert!(archive.validate().is_err());
    let mut archive = fixture();
    archive.files.insert("../escape".into(), None);
    assert!(archive.validate().is_err());
    let mut archive = fixture();
    let server = json!({"id":"same","name":"server","host":"host","connectionMethod":"ssh","sshPort":22,"sshUser":"user"});
    archive.files.insert(
        "servers.json".into(),
        Some(json!({"servers":[server.clone(),server]})),
    );
    assert!(archive.validate().is_err());
}

#[test]
fn backup_portable_restore_disables_network_rotates_access_and_preserves_paths() {
    let archive = fixture();
    let mut destination = fixture();
    destination
        .files
        .get_mut("sync-local.json")
        .unwrap()
        .as_mut()
        .unwrap()["repoRoot"] = json!("/destination");
    let restore = compile_restore(&archive, &destination, &RestoreOptions::default()).unwrap();
    let local = restore.files["sync-local.json"].as_ref().unwrap();
    assert_eq!(local["repoRoot"], "/destination");
    assert_eq!(local["mcpEnabled"], false);
    assert_eq!(local["ccEnabled"], false);
    assert_eq!(local["ccAgentToken"], "");
    assert_ne!(local["mcpAuthToken"], "source-mcp");
    assert!(!restore.credentials.contains_key("cc-telemetry-queue-key"));
    assert!(!restore.credentials.contains_key("boundary_app_token"));
}

#[test]
fn backup_unselected_sections_and_credentials_stay_untouched() {
    let archive = fixture();
    let options = RestoreOptions {
        shared: false,
        servers: false,
        local: false,
        secrets: false,
        ..RestoreOptions::default()
    };
    assert!(compile_restore(&archive, &fixture(), &options).is_err());
    let options = RestoreOptions {
        shared: false,
        servers: false,
        local: false,
        ..RestoreOptions::default()
    };
    let restore = compile_restore(&archive, &fixture(), &options).unwrap();
    assert!(restore.files.is_empty());
    assert_eq!(restore.credentials["openvpn_password"], None);
}

#[test]
fn backup_export_distinguishes_missing_from_keyring_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = FakeSecrets::default();
    let archive = snapshot(dir.path(), &mut store).unwrap();
    assert_eq!(archive.credentials["openvpn_password"], None);
    store.fail_read = true;
    assert!(snapshot(dir.path(), &mut store).is_err());
}

#[derive(Default)]
struct FakeSecrets {
    values: BTreeMap<String, String>,
    fail_read: bool,
    fail_write_once: bool,
    fail_writes: bool,
}
impl SecretStore for FakeSecrets {
    fn read(&mut self, account: &str) -> Result<Option<String>, String> {
        if self.fail_read {
            return Err("locked".into());
        }
        Ok(self.values.get(account).cloned())
    }
    fn write(&mut self, account: &str, value: Option<&str>) -> Result<(), String> {
        if (self.fail_write_once || self.fail_writes) && account != transaction::JOURNAL_ACCOUNT {
            self.fail_write_once = false;
            return Err("injected write failure".into());
        }
        match value {
            Some(value) => {
                self.values.insert(account.into(), value.into());
            }
            None => {
                self.values.remove(account);
            }
        }
        Ok(())
    }
}

#[test]
fn backup_transaction_rolls_back_files_and_secrets_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let old = b"old exact bytes";
    std::fs::write(dir.path().join("config.json"), old).unwrap();
    let mut store = FakeSecrets::default();
    store
        .values
        .insert("openvpn_password".into(), "old-secret".into());
    store.fail_write_once = true;
    let changes = Changes {
        files: BTreeMap::from([("config.json".into(), Some(json!({"new":true})))]),
        credentials: BTreeMap::from([("openvpn_password".into(), Some("new-secret".into()))]),
    };
    assert!(transaction::commit(dir.path(), &changes, &mut store).is_err());
    assert_eq!(std::fs::read(dir.path().join("config.json")).unwrap(), old);
    assert_eq!(store.values["openvpn_password"], "old-secret");
    assert!(!dir.path().join(transaction::JOURNAL_FILE).exists());
}

#[test]
fn backup_crash_recovery_requires_durable_key_and_restores_exact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = FakeSecrets::default();
    let changes = Changes {
        files: BTreeMap::from([("config.json".into(), Some(json!({"new":true})))]),
        credentials: BTreeMap::from([("openvpn_password".into(), Some("new-secret".into()))]),
    };
    std::fs::write(dir.path().join("config.json"), b"original").unwrap();
    transaction::prepare(dir.path(), &changes, &mut store).unwrap();
    std::fs::write(dir.path().join("config.json"), b"interrupted").unwrap();
    store.fail_read = true;
    assert!(transaction::recover(dir.path(), &mut store).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("config.json")).unwrap(),
        b"interrupted"
    );
    store.fail_read = false;
    transaction::recover(dir.path(), &mut store).unwrap();
    transaction::recover(dir.path(), &mut store).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("config.json")).unwrap(),
        b"original"
    );
}

#[test]
fn backup_atomic_write_preserves_existing_file_and_rejects_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("backup.scpbak");
    io::atomic_write(&path, b"original", false).unwrap();
    assert!(io::atomic_write(&path, b"replacement", false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(io::atomic_write(&link, b"replacement", true).is_err());
    }
}

#[test]
fn backup_maintenance_gate_is_nested_but_refuses_other_threads() {
    let _serial = GATE_TEST_LOCK.lock().unwrap();
    let _first = gate::maintain().unwrap();
    let _second = gate::enter().unwrap();
    assert!(std::thread::spawn(|| gate::enter().is_err())
        .join()
        .unwrap());
}

#[test]
fn backup_secret_only_restore_requires_matching_server_ids() {
    let mut source = fixture();
    source.files.insert("servers.json".into(), Some(json!({"servers":[{"id":"new","name":"s","host":"h","connectionMethod":"ssh","sshPort":22,"sshUser":"u"}]})));
    source
        .credentials
        .insert("server_ssh_password:new".into(), Some("secret".into()));
    let options = RestoreOptions {
        servers: false,
        ..RestoreOptions::default()
    };
    assert!(compile_restore(&source, &fixture(), &options).is_err());
}

#[test]
fn backup_server_replacement_deletes_removed_credentials_and_strips_legacy_secrets() {
    let mut destination = fixture();
    destination.files.insert("servers.json".into(), Some(json!({"servers":[{"id":"old","name":"s","host":"h","connectionMethod":"ssh","sshPort":22,"sshUser":"u"}]})));
    destination
        .credentials
        .insert("server_ssh_password:old".into(), Some("old-secret".into()));
    let mut source = fixture();
    source
        .files
        .get_mut("config.json")
        .unwrap()
        .as_mut()
        .unwrap()["openvpn"]["password"] = json!("legacy-secret");
    let changes = compile_restore(&source, &destination, &RestoreOptions::default()).unwrap();
    assert_eq!(changes.credentials["server_ssh_password:old"], None);
    assert!(changes.files["config.json"].as_ref().unwrap()["openvpn"]
        .get("password")
        .is_none());
}

#[test]
fn backup_gate_blocks_existing_credential_read_and_write_before_os_access() {
    let _serial = GATE_TEST_LOCK.lock().unwrap();
    let _guard = gate::maintain().unwrap();
    std::thread::spawn(|| {
        assert!(crate::services::keyring_get_optional("openvpn_password").is_err());
        assert!(
            crate::services::keyring_set_or_delete("openvpn_password", "must-not-write").is_err()
        );
    })
    .join()
    .unwrap();
}

#[test]
fn backup_preview_redacts_values_and_detects_stale_destination() {
    let mut source = fixture();
    source
        .credentials
        .insert("openvpn_password".into(), Some("never-expose-this".into()));
    let preview = desktop::describe(&source, &fixture(), &RestoreOptions::default()).unwrap();
    let rendered = serde_json::to_string(&preview).unwrap();
    assert!(!rendered.contains("never-expose-this"));
    assert!(!rendered.contains("source-token"));
    assert!(!rendered.contains("source-mcp"));
    let before = desktop::fingerprint(&fixture()).unwrap();
    let mut changed = fixture();
    changed
        .credentials
        .insert("openvpn_password".into(), Some("changed".into()));
    assert_ne!(before, desktop::fingerprint(&changed).unwrap());
}

#[test]
fn backup_external_inventory_records_ovpn_keys_and_ssh_identity_without_bundling() {
    let dir = tempfile::tempdir().unwrap();
    let ovpn = dir.path().join("client.ovpn");
    std::fs::write(
        &ovpn,
        "ca \"client ca.pem\"\nkey client.key\n<key>\nsecret-inline-key\n</key>\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("sync-local.json"),
        serde_json::to_vec(&json!({"openvpnOvpnPath":ovpn})).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.path().join("config.json"), serde_json::to_vec(&json!({"app":{"defaultTerminalCols":120,"defaultTerminalRows":36},"boundary":{"connectSshOptions":"-i '/foreign/identity key'"}})).unwrap()).unwrap();
    let archive = snapshot(dir.path(), &mut FakeSecrets::default()).unwrap();
    assert!(archive
        .external_files
        .iter()
        .any(|file| file.path.ends_with("client ca.pem")));
    assert!(archive
        .external_files
        .iter()
        .any(|file| file.path.ends_with("client.key")));
    assert!(archive
        .external_files
        .iter()
        .any(|file| file.path == "/foreign/identity key"));
    assert!(!serde_json::to_string(&archive)
        .unwrap()
        .contains("secret-inline-key"));
}

#[test]
fn backup_remaps_paths_explicitly_without_activating_copied_identity() {
    let mut options = RestoreOptions::default();
    options
        .path_overrides
        .insert("openvpnOvpnPath".into(), "/remapped/client.ovpn".into());
    options.recover_identity = true;
    let changes = compile_restore(&fixture(), &fixture(), &options).unwrap();
    let local = changes.files["sync-local.json"].as_ref().unwrap();
    assert_eq!(local["openvpnOvpnPath"], "/remapped/client.ovpn");
    assert_eq!(local["ccAgentToken"], "source-token");
    assert_eq!(local["mcpEnabled"], false);
    assert_eq!(local["ccEnabled"], false);
    options
        .path_overrides
        .insert("arbitrarySetting".into(), "bad".into());
    assert!(compile_restore(&fixture(), &fixture(), &options).is_err());
}

#[test]
fn backup_successful_transaction_commits_exact_credential_deletions() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = FakeSecrets::default();
    store
        .values
        .insert("openvpn_password".into(), "remove".into());
    let changes = Changes {
        files: BTreeMap::from([("servers.json".into(), Some(json!({"servers":[]})))]),
        credentials: BTreeMap::from([("openvpn_password".into(), None)]),
    };
    transaction::commit(dir.path(), &changes, &mut store).unwrap();
    assert!(!store.values.contains_key("openvpn_password"));
    assert!(!store.values.contains_key(transaction::JOURNAL_ACCOUNT));
    assert!(!dir.path().join(transaction::JOURNAL_FILE).exists());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(dir.path().join("servers.json")).unwrap()
        )
        .unwrap(),
        json!({"servers":[]})
    );
}

#[test]
fn backup_work_cancellation_stops_before_mutation() {
    let work = desktop::BackupWork::begin("cancel-test".into()).unwrap();
    desktop::cancel_settings_backup_work("cancel-test".into()).unwrap();
    assert!(work.check().is_err());
}

#[test]
fn backup_failed_rollback_retains_journal_until_keyring_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = FakeSecrets::default();
    store.fail_writes = true;
    let changes = Changes {
        files: BTreeMap::new(),
        credentials: BTreeMap::from([("openvpn_password".into(), Some("new".into()))]),
    };
    let error = transaction::commit(dir.path(), &changes, &mut store).unwrap_err();
    assert!(error.contains("rollback failed"));
    assert!(dir.path().join(transaction::JOURNAL_FILE).exists());
    store.fail_writes = false;
    transaction::recover(dir.path(), &mut store).unwrap();
    assert!(!dir.path().join(transaction::JOURNAL_FILE).exists());
    assert!(!store.values.contains_key("openvpn_password"));
}

#[test]
fn backup_required_rollback_read_failure_blocks_all_mutations() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = FakeSecrets {
        fail_read: true,
        ..FakeSecrets::default()
    };
    std::fs::write(dir.path().join("config.json"), b"original").unwrap();
    let changes = Changes {
        files: BTreeMap::from([("config.json".into(), Some(json!({"changed":true})))]),
        credentials: BTreeMap::from([("openvpn_password".into(), None)]),
    };
    assert!(transaction::commit(dir.path(), &changes, &mut store).is_err());
    assert!(store.values.is_empty());
    assert_eq!(
        std::fs::read(dir.path().join("config.json")).unwrap(),
        b"original"
    );
    assert!(!dir.path().join(transaction::JOURNAL_FILE).exists());
}

#[test]
fn backup_recovery_keeps_a_durably_committed_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = FakeSecrets::default();
    let changes = Changes {
        files: BTreeMap::from([("config.json".into(), Some(json!({"new":true})))]),
        credentials: BTreeMap::new(),
    };
    std::fs::write(dir.path().join("config.json"), b"old").unwrap();
    transaction::prepare(dir.path(), &changes, &mut store).unwrap();
    std::fs::write(dir.path().join("config.json"), b"committed").unwrap();
    transaction::mark_committed(dir.path(), &mut store).unwrap();
    transaction::recover(dir.path(), &mut store).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("config.json")).unwrap(),
        b"committed"
    );
}

#[test]
fn backup_completed_journal_tombstone_needs_no_recovery_key() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(transaction::JOURNAL_FILE),
        transaction::DONE,
    )
    .unwrap();
    let mut store = FakeSecrets {
        fail_read: true,
        ..FakeSecrets::default()
    };
    transaction::recover(dir.path(), &mut store).unwrap();
}

#[test]
fn backup_async_settings_changes_are_visible_until_the_request_finishes() {
    let _serial = GATE_TEST_LOCK.lock().unwrap();
    assert!(!gate::has_async_changes());
    let operation = gate::begin_async_change().unwrap();
    assert!(gate::has_async_changes());
    drop(operation);
    assert!(!gate::has_async_changes());
}

#[test]
fn backup_retry_waits_for_maintenance_before_rechecking_its_generation() {
    let _serial = GATE_TEST_LOCK.lock().unwrap();
    let guard = gate::maintain().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _guard = gate::wait_for_maintenance().unwrap();
        send.send(()).unwrap();
    });
    assert!(receive
        .recv_timeout(std::time::Duration::from_millis(20))
        .is_err());
    drop(guard);
    receive
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn backup_boundary_controller_change_requires_fresh_login_and_previews_token_deletion() {
    let mut destination = fixture();
    destination
        .files
        .get_mut("config.json")
        .unwrap()
        .as_mut()
        .unwrap()["boundary"] = json!({"addr":"https://old.example"});
    destination.credentials.insert(
        "boundary_app_token".into(),
        Some("old-controller-token".into()),
    );
    let mut source = fixture();
    source
        .files
        .get_mut("config.json")
        .unwrap()
        .as_mut()
        .unwrap()["boundary"] = json!({"addr":"https://new.example"});
    let changes = compile_restore(&source, &destination, &RestoreOptions::default()).unwrap();
    assert_eq!(changes.credentials["boundary_app_token"], None);
    let preview = desktop::describe(&source, &destination, &RestoreOptions::default()).unwrap();
    let text = serde_json::to_string(&preview).unwrap();
    assert!(text.contains("boundary_app_token"));
    assert!(!text.contains("old-controller-token"));
}

#[test]
fn backup_preview_rejects_expired_replaced_and_changed_destinations() {
    let _serial = GATE_TEST_LOCK.lock().unwrap();
    let _guard = gate::enter().unwrap();
    let mut pending =
        desktop::Pending::new(fixture(), RestoreOptions::default(), &fixture()).unwrap();
    pending.verify_identity(&pending.id).unwrap();
    pending.verify_destination(&fixture()).unwrap();
    assert!(pending.verify_identity("different-preview").is_err());
    let mut changed = fixture();
    changed
        .credentials
        .insert("openvpn_password".into(), Some("changed".into()));
    assert!(pending.verify_destination(&changed).is_err());
    gate::changed();
    assert!(pending.verify_destination(&fixture()).is_err());
    pending.expires = std::time::Instant::now() - std::time::Duration::from_secs(1);
    assert!(pending.verify_identity(&pending.id).is_err());
}

#[test]
fn backup_manifest_metadata_and_reference_sizes_are_bounded() {
    let mut archive = fixture();
    archive.platform = "x".repeat(128);
    assert!(archive.validate().is_err());
    archive.platform = "linux".into();
    archive.external_files.push(ExternalFile {
        kind: "key".into(),
        path: "x".repeat(5000),
    });
    assert!(archive.validate().is_err());
}

#[test]
fn backup_ordinary_settings_reads_remain_concurrent_outside_maintenance() {
    let _serial = GATE_TEST_LOCK.lock().unwrap();
    let _ordinary = gate::enter().unwrap();
    assert!(std::thread::spawn(
        || crate::services::keyring_get_optional("openvpn_password").is_ok()
    )
    .join()
    .unwrap());
}

#[test]
fn backup_snapshot_waits_until_ordinary_operations_release_their_leases() {
    let _serial = GATE_TEST_LOCK.lock().unwrap();
    let guard = gate::enter().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _maintenance = gate::maintain().unwrap();
        send.send(()).unwrap();
    });
    assert!(receive
        .recv_timeout(std::time::Duration::from_millis(20))
        .is_err());
    drop(guard);
    receive
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn backup_rejects_server_ids_that_could_escape_ui_attributes() {
    let mut source = fixture();
    let id = "srv-\" onclick=\"alert(1)";
    source.files.insert("servers.json".into(), Some(json!({"servers":[{"id":id,"name":"s","host":"h","connectionMethod":"ssh","sshPort":22,"sshUser":"u"}]})));
    source.credentials.insert(keyring_server_ssh_password_account(id), None);
    assert!(source.validate().is_err());
}

#[test]
fn backup_manifest_precedes_sections_and_credential_checks() {
    let mut archive = fixture();
    archive.files.clear();
    archive.credentials.clear();
    archive.schema = 2;
    archive.platform = "x".repeat(65);
    assert_eq!(
        archive.validate(),
        Err("Backup manifest metadata exceeds size limits".into())
    );
    archive.platform.clear();
    assert_eq!(
        archive.validate(),
        Err("Unsupported settings schema; update the application".into())
    );
    archive.schema = 1;
    assert_eq!(
        archive.validate(),
        Err("Backup must declare all settings sections".into())
    );
}

#[test]
fn backup_external_reference_serialization_remains_nested_and_strict() {
    let mut archive = fixture();
    archive.external_files.push(ExternalFile {
        kind: "foreign".into(),
        path: "../key".into(),
    });
    let mut wire = serde_json::to_value(&archive).unwrap();
    assert_eq!(
        wire["externalFiles"],
        json!([{"kind":"foreign","path":"../key"}])
    );
    assert_eq!(wire.as_object().unwrap().len(), 7);
    let decoded: Archive = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded.validate(), Ok(()));
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    wire["externalFiles"][0]["contents"] = json!("must-reject");
    assert!(serde_json::from_value::<Archive>(wire).is_err());
}

mod backup_server_id_contracts {
    fn server_ids(value: Option<&serde_json::Value>) -> Result<Vec<String>, String> {
        let mut archive = super::fixture();
        archive.files.remove("servers.json");
        if let Some(value) = value {
            archive
                .files
                .insert("servers.json".into(), Some(value.clone()));
        }
        archive.server_ids()
    }
    mod contracts {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/scp-core/tests/contracts/backup_server_ids.rs"
        ));
    }
}

#[test]
fn backup_server_id_errors_follow_sections_and_precede_credentials() {
    let mut archive = fixture();
    archive.files.insert("servers.json".into(), None);
    assert_eq!(archive.server_ids(), Ok(vec![]));
    archive
        .files
        .insert("servers.json".into(), Some(json!({"servers":[{"id":""}]})));
    archive.credentials.clear();
    assert_eq!(
        archive.validate(),
        Err("Invalid backup server record".into())
    );
    archive
        .files
        .insert("sync-local.json".into(), Some(json!({"mcpPort":"invalid"})));
    assert_eq!(archive.validate(), Err("Invalid local settings".into()));
    archive
        .files
        .insert("servers.json".into(), Some(serde_json::Value::Null));
    assert_eq!(archive.validate(), Err("Invalid settings section".into()));
    archive.files.remove("config.json");
    assert_eq!(
        archive.validate(),
        Err("Backup must declare all settings sections".into())
    );
}

mod backup_section_contracts {
    fn validate_sections(files: &BTreeMap<String, Option<Value>>) -> Result<(), String> {
        let mut archive = super::fixture();
        archive.files = files.clone();
        archive.validate()
    }
    use super::*;
    mod contracts {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/scp-core/tests/contracts/backup_sections.rs"
        ));
    }
}

#[test]
fn backup_section_errors_preserve_outer_validation_precedence() {
    let mut archive = fixture();
    archive.schema = 2;
    archive.platform = "x".repeat(65);
    archive.files.remove("config.json");
    archive
        .files
        .insert("servers.json".into(), Some(json!({"servers":[{}]})));
    archive
        .files
        .insert("sync-local.json".into(), Some(json!({"mcpPort":null})));
    archive.credentials.clear();
    assert_eq!(
        archive.validate(),
        Err("Backup manifest metadata exceeds size limits".into())
    );
    archive.platform.clear();
    assert_eq!(
        archive.validate(),
        Err("Unsupported settings schema; update the application".into())
    );
    archive.schema = 1;
    assert_eq!(
        archive.validate(),
        Err("Backup must declare all settings sections".into())
    );
    archive.files.insert("config.json".into(), Some(json!({})));
    assert_eq!(archive.validate(), Err("Invalid app settings".into()));
    archive.files.insert("config.json".into(), None);
    assert_eq!(archive.validate(), Err("Invalid local settings".into()));
    archive.files.insert("sync-local.json".into(), None);
    assert_eq!(
        archive.validate(),
        Err("Invalid backup server record".into())
    );
    archive.files.insert("servers.json".into(), None);
    assert_eq!(
        archive.validate(),
        Err("Backup credential inventory does not match known app accounts and server IDs".into())
    );
    archive.credentials = known_accounts(&[]).into_iter().map(|a| (a, None)).collect();
    archive
        .credentials
        .insert("openvpn_password".into(), Some("x".repeat(65537)));
    assert_eq!(
        archive.validate(),
        Err("Backup credential is too large".into())
    );
}

#[test]
fn backup_wire_null_is_an_absent_file_not_a_present_null_value() {
    let mut wire = serde_json::to_value(fixture()).unwrap();
    wire["files"]["config.json"] = Value::Null;
    let mut archive: Archive = serde_json::from_value(wire).unwrap();
    assert_eq!(archive.files["config.json"], None);
    assert_eq!(archive.validate(), Ok(()));
    archive
        .files
        .insert("config.json".into(), Some(Value::Null));
    assert_eq!(archive.validate(), Err("Invalid settings section".into()));
}

mod backup_credential_contracts {
    use super::*;

    fn validate_credentials(
        credentials: &BTreeMap<String, Option<String>>,
        expected: &BTreeSet<String>,
    ) -> Result<(), String> {
        let mut archive = super::fixture();
        archive
            .files
            .get_mut("servers.json")
            .unwrap()
            .as_mut()
            .unwrap()["servers"] = json!([{
            "id": "node:one", "name": "server", "host": "host",
            "connectionMethod": "ssh", "sshPort": 22, "sshUser": "user"
        }]);
        assert_eq!(
            *expected,
            known_accounts(&archive.server_ids().unwrap())
                .into_iter()
                .collect()
        );
        archive.credentials = credentials.clone();
        archive.validate()
    }

    mod contracts {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/scp-core/tests/contracts/backup_credentials.rs"
        ));
    }
}
