use scp_core::backup::{
    compile_restore, crypto, describe, fingerprint, io as backup_io, known_accounts, snapshot,
    snapshot_with_version, transaction, validate_export_target, Archive, Changes, ExternalFile,
    PendingRestore, RestoreOptions, CONFIGURED, FILES, LOCAL_PATHS,
};
use scp_core::config_ops::{
    resolve_config_secret_account, validate_rdp_target, validate_server_id,
};
use scp_core::error::CoreError;
use scp_core::models::{
    KEYRING_BOUNDARY_APP_TOKEN, KEYRING_BOUNDARY_PASSWORD, KEYRING_OPENVPN_PASSWORD,
    KEYRING_SERVICE,
};
use scp_core::profile::ProfileLocationInput;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::{KeyringHealth, MemoryKeyringBackend};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use tempfile::TempDir;

#[derive(Default)]
struct MapSecrets(BTreeMap<String, String>);

impl scp_core::backup::SecretStore for MapSecrets {
    fn read(&mut self, account: &str) -> Result<Option<String>, String> {
        Ok(self.0.get(account).cloned())
    }

    fn write(&mut self, account: &str, value: Option<&str>) -> Result<(), String> {
        if let Some(secret) = value {
            self.0.insert(account.to_owned(), secret.to_owned());
        } else {
            self.0.remove(account);
        }
        Ok(())
    }
}

fn fixture_archive() -> Archive {
    let mut files = BTreeMap::new();
    files.insert(
        "config.json".to_string(),
        Some(json!({
            "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 36},
            "openvpn": {"extraArgs": "--verb 3", "password": "legacy-should-strip"},
            "boundary": {"addr": "https://source.boundary.internal"}
        })),
    );
    let server = json!({
        "id": "srv-1",
        "name": "App 1",
        "host": "10.0.0.10",
        "os": "linux",
        "connectionMethod": "ssh",
        "sshPort": 22,
        "sshUser": "ops"
    });
    files.insert(
        "servers.json".to_string(),
        Some(json!({ "servers": [server] })),
    );
    files.insert(
        "sync-local.json".to_string(),
        Some(json!({
            "mcpEnabled": true,
            "ccEnabled": true,
            "mcpAuthToken": "source-mcp-token",
            "ccAgentToken": "source-cc-token",
            "ccDeviceId": "source-device-id",
            "ccDeviceName": "source-device",
            "repoRoot": "/source/repo"
        })),
    );
    let mut credentials = BTreeMap::new();
    for account in known_accounts(&["srv-1".to_string()]) {
        credentials.insert(account, None);
    }
    Archive {
        schema: 1,
        app_version: String::from("1.3.3"),
        platform: String::from("foreign"),
        created_at_ms: 1,
        files,
        credentials,
        external_files: Vec::new(),
    }
}

fn open_isolated_runtime(
    root: &TempDir,
    profile: &str,
) -> (ProfileRuntime<MemoryKeyringBackend>, MemoryKeyringBackend) {
    let backend = MemoryKeyringBackend::new(KeyringHealth::Ready);
    let desktop_dir = root.path().join("desktop-ignored");
    let loc = ProfileLocationInput {
        config_dir: Some(root.path()),
        env_config_dir: None,
        xdg_config_home: None,
        home_dir: None,
        desktop_config_dir: Some(&desktop_dir),
        profile: Some(profile),
    };
    let rt =
        ProfileRuntime::open_with_backend(&loc, "test", backend.clone()).expect("open runtime");
    (rt, backend)
}

#[test]
fn shared_backup_engine_validates_describes_and_protects_export_targets() {
    let mut source = fixture_archive();
    source
        .credentials
        .insert(KEYRING_OPENVPN_PASSWORD.into(), Some("vpn-secret".into()));
    source.credentials.insert(
        "server_ssh_password:srv-1".into(),
        Some("ssh-secret".into()),
    );
    source.external_files.push(ExternalFile {
        kind: "SSH identity".into(),
        path: "/tmp/nonexistent-id-ed25519".into(),
    });
    source.validate().expect("fixture should validate");

    let mut bad = fixture_archive();
    bad.schema = 99;
    assert!(bad.validate().is_err());

    let mut destination = fixture_archive();
    destination
        .files
        .get_mut("config.json")
        .unwrap()
        .as_mut()
        .unwrap()["boundary"] = json!({"addr": "https://old.boundary.internal"});
    destination.credentials.insert(
        KEYRING_BOUNDARY_APP_TOKEN.into(),
        Some("old-app-token".into()),
    );
    destination
        .files
        .get_mut("sync-local.json")
        .unwrap()
        .as_mut()
        .unwrap()["repoRoot"] = json!("/dest/repo");

    let mut options = RestoreOptions::default();
    options
        .path_overrides
        .insert("openvpnOvpnPath".into(), "/custom/client.ovpn".into());
    options.recover_identity = false;

    let preview = describe(&source, &destination, &options).expect("describe preview");
    let rendered = serde_json::to_string(&preview).unwrap();
    assert!(!rendered.contains("vpn-secret"));
    assert!(!rendered.contains("ssh-secret"));
    assert!(!rendered.contains("old-app-token"));
    assert!(!rendered.contains("source-cc-token"));
    assert!(!rendered.contains("source-mcp-token"));

    let changes = compile_restore(&source, &destination, &options).expect("compile restore");
    let cfg = changes.files["config.json"].as_ref().unwrap();
    assert!(cfg["openvpn"].get("password").is_none());
    let local = changes.files["sync-local.json"].as_ref().unwrap();
    assert_eq!(local["mcpEnabled"], false);
    assert_eq!(local["ccEnabled"], false);
    assert_eq!(local["ccAgentToken"], "");
    assert_eq!(local["ccDeviceId"], "");
    assert_eq!(local["ccDeviceName"], "");
    assert_ne!(local["mcpAuthToken"], "source-mcp-token");
    assert_eq!(local["openvpnOvpnPath"], "/custom/client.ovpn");
    assert_eq!(local["repoRoot"], "/dest/repo");
    assert_eq!(changes.credentials[KEYRING_BOUNDARY_APP_TOKEN], None);

    let mut pending = PendingRestore::new(
        source.clone(),
        options,
        &destination,
        1,
        Duration::from_secs(60),
    )
    .expect("pending restore");
    pending.verify_identity(&pending.id).unwrap();
    pending.verify_destination(&destination, 1).unwrap();
    assert!(pending.verify_destination(&destination, 2).is_err());
    pending.expires = std::time::Instant::now() - Duration::from_secs(1);
    assert!(pending.verify_identity(&pending.id).is_err());

    let tmp = TempDir::new().unwrap();
    let profile_dir = tmp.path().join("prof");
    std::fs::create_dir_all(&profile_dir).unwrap();
    for protected in [
        "config.json",
        "servers.json",
        "sync-local.json",
        transaction::JOURNAL_FILE,
        "owner.lock",
        "profile-meta.json",
        "stdio-policy.json",
    ] {
        assert!(validate_export_target(&profile_dir.join(protected), &profile_dir, false).is_err());
    }
    let valid = validate_export_target(&tmp.path().join("backup.scpbak"), &profile_dir, false)
        .expect("safe export target");
    assert!(valid.is_absolute());
}

#[test]
fn desktop_to_cli_and_cli_to_desktop_scpbak_round_trip_with_namespace_isolation() {
    let root = TempDir::new().unwrap();
    let (rt, backend) = open_isolated_runtime(&root, "backup-rt");
    // Seed an un-namespaced desktop credential that must stay untouched during CLI restore/export.
    backend.seed_entry(
        KEYRING_SERVICE,
        KEYRING_OPENVPN_PASSWORD,
        "desktop-vpn-pass",
    );

    std::fs::write(
        rt.profile_paths().stdio_policy_path.clone(),
        br#"{"sensitiveActionPolicy":"read-only","allowedToolCategories":["read-only"]}"#,
    )
    .unwrap();
    let original_ns = rt.credential_namespace().to_string();

    let mut source = fixture_archive();
    source
        .credentials
        .insert(KEYRING_OPENVPN_PASSWORD.into(), Some("cli-vpn-pass".into()));
    source.credentials.insert(
        "server_ssh_password:srv-1".into(),
        Some("cli-ssh-pass".into()),
    );
    let scpbak = root.path().join("desktop-export.scpbak");
    let encrypted = crypto::encrypt(&source, "correct horse battery staple").unwrap();
    backup_io::atomic_write(&scpbak, &encrypted, true).unwrap();

    // Recover identity flag MUST never preserve source tokens in standalone CLI prepare.
    let mut opts = RestoreOptions {
        recover_identity: true,
        ..RestoreOptions::default()
    };
    opts.recover_identity = true;
    let (preview, pending) = rt
        .prepare_settings_import(
            &scpbak,
            "correct horse battery staple",
            opts,
            Duration::from_secs(60),
        )
        .expect("prepare import");
    assert_eq!(preview.servers, 1);

    let pid = preview.id.clone();
    let res = rt
        .commit_pending_import(pending, &pid)
        .expect("commit import");
    assert_eq!(res["applied"], true);

    assert_eq!(
        backend
            .snapshot_entry(KEYRING_SERVICE, KEYRING_OPENVPN_PASSWORD)
            .as_deref(),
        Some("desktop-vpn-pass"),
        "desktop keyring entry must remain untouched"
    );
    let scoped_key = format!("profile:{original_ns}:{KEYRING_OPENVPN_PASSWORD}");
    assert_eq!(
        backend
            .snapshot_entry(KEYRING_SERVICE, &scoped_key)
            .as_deref(),
        Some("cli-vpn-pass")
    );
    let local: Value =
        serde_json::from_slice(&std::fs::read(&rt.profile_paths().sync_local_path).unwrap())
            .unwrap();
    assert_eq!(local["mcpEnabled"], false);
    assert_eq!(local["ccEnabled"], false);
    assert_eq!(local["ccAgentToken"], "");
    assert_ne!(local["mcpAuthToken"], "source-mcp-token");
    assert_eq!(
        std::fs::read_to_string(&rt.profile_paths().stdio_policy_path).unwrap(),
        r#"{"sensitiveActionPolicy":"read-only","allowedToolCategories":["read-only"]}"#
    );

    let export_path = root.path().join("cli-export.scpbak");
    let export_summary = rt
        .export_settings_backup(&export_path, "another long passphrase!", false)
        .expect("cli export");
    assert_eq!(export_summary["complete"], true);

    let bytes = backup_io::read_bounded(&export_path).unwrap().unwrap();
    let decoded: Archive = crypto::decrypt(&bytes, "another long passphrase!").unwrap();
    assert_eq!(
        decoded.credentials[KEYRING_OPENVPN_PASSWORD].as_deref(),
        Some("cli-vpn-pass")
    );
    assert_eq!(
        decoded.credentials["server_ssh_password:srv-1"].as_deref(),
        Some("cli-ssh-pass")
    );
    let desktop_preview =
        compile_restore(&decoded, &fixture_archive(), &RestoreOptions::default()).unwrap();
    assert_eq!(
        desktop_preview.credentials["server_ssh_password:srv-1"].as_deref(),
        Some("cli-ssh-pass")
    );
}

#[test]
fn crash_recovery_restores_interrupted_journal_on_profile_open() {
    let root = TempDir::new().unwrap();
    let desktop_dir = root.path().join("desktop-ignored");
    let loc = ProfileLocationInput {
        config_dir: Some(root.path()),
        env_config_dir: None,
        xdg_config_home: None,
        home_dir: None,
        desktop_config_dir: Some(&desktop_dir),
        profile: Some("recover"),
    };
    let backend = MemoryKeyringBackend::new(KeyringHealth::Ready);
    let rt = ProfileRuntime::open_with_backend(&loc, "test", backend.clone()).unwrap();
    let profile_dir = rt.profile_paths().profile_dir.clone();
    let ns = rt.credential_namespace().to_string();
    drop(rt);

    std::fs::write(profile_dir.join("config.json"), b"before-crash").unwrap();
    let mut store =
        scp_core::secrets::NamespacedSecretStore::new(KEYRING_SERVICE, &ns, backend.clone());
    let changes = Changes {
        files: BTreeMap::from([("config.json".into(), Some(json!({"after": true})))]),
        credentials: BTreeMap::from([(KEYRING_OPENVPN_PASSWORD.into(), Some("new".into()))]),
    };
    transaction::prepare(&profile_dir, &changes, &mut store).unwrap();
    std::fs::write(profile_dir.join("config.json"), b"partial-corrupt").unwrap();

    // Locked keyring blocks startup cleanly without overwriting files.
    let locked_backend = MemoryKeyringBackend::new(KeyringHealth::Locked);
    assert!(ProfileRuntime::open_with_backend(&loc, "test", locked_backend).is_err());
    assert_eq!(
        std::fs::read(profile_dir.join("config.json")).unwrap(),
        b"partial-corrupt"
    );

    // Valid keyring recovers exact pre-transaction bytes (non-JSON -> overwritten cleanly).
    std::fs::write(
        profile_dir.join("config.json"),
        b"{\"app\":{\"defaultTerminalCols\":80,\"defaultTerminalRows\":24}}",
    )
    .unwrap();
    let mut store2 =
        scp_core::secrets::NamespacedSecretStore::new(KEYRING_SERVICE, &ns, backend.clone());
    transaction::recover(&profile_dir, &mut store2).unwrap();
    assert_eq!(
        std::fs::read(profile_dir.join("config.json")).unwrap(),
        b"before-crash"
    );
    assert!(!profile_dir.join(transaction::JOURNAL_FILE).exists());
}

#[test]
fn server_config_settings_credentials_sync_push_and_rdp_operations_enforce_contracts() {
    let root = TempDir::new().unwrap();
    let (rt, backend) = open_isolated_runtime(&root, "mutations");
    let ns = rt.credential_namespace().to_string();

    assert!(validate_server_id("").is_err());
    assert!(validate_server_id("srv bad").is_err());
    validate_server_id("srv-win_1.prod:01").unwrap();
    assert_eq!(
        resolve_config_secret_account("openvpn-password").unwrap(),
        KEYRING_OPENVPN_PASSWORD
    );
    assert!(resolve_config_secret_account("unknown-key").is_err());
    assert!(validate_rdp_target("linux", false, true, "win.internal").is_err());
    assert!(validate_rdp_target("linux", true, false, "win.internal").is_err());
    assert!(validate_rdp_target("linux", true, true, "bad host?cmd=1").is_err());
    validate_rdp_target("linux", true, true, "win.internal").unwrap();

    let srv = json!({
        "id": "srv-1",
        "name": "$(id); --evil",
        "host": "10.0.0.5",
        "os": "windows",
        "connectionMethod": "ssh",
        "sshPort": 22,
        "sshUser": "admin;rm"
    });
    let prev = rt.upsert_server(&srv, true, None).expect("preview upsert");
    assert_eq!(prev["preview"], true);
    assert_eq!(prev["action"], "create");
    assert!(!rt.profile_paths().servers_path.exists());
    let rev0 = prev["revision"].as_str().unwrap().to_string();

    let committed = rt
        .upsert_server(&srv, false, Some(&rev0))
        .expect("commit upsert");
    assert_eq!(committed["preview"], false);
    assert_eq!(rt.get_server("srv-1").unwrap().name, "$(id); --evil");

    let stale = rt.upsert_server(&srv, false, Some(&rev0)).unwrap_err();
    assert_eq!(stale.code, "CONFLICT");
    assert_eq!(stale.reason, "revision_mismatch");

    rt.set_server_ssh_password("srv-1", "ssh-pass-1").unwrap();
    assert_eq!(
        backend
            .snapshot_entry(
                KEYRING_SERVICE,
                &format!("profile:{ns}:server_ssh_password:srv-1")
            )
            .as_deref(),
        Some("ssh-pass-1")
    );
    assert!(backend
        .snapshot_entry(KEYRING_SERVICE, "server_ssh_password:srv-1")
        .is_none());

    rt.set_config_secret("openvpn-password", "vpn-pass-1")
        .unwrap();
    assert_eq!(
        backend
            .snapshot_entry(
                KEYRING_SERVICE,
                &format!("profile:{ns}:{KEYRING_OPENVPN_PASSWORD}")
            )
            .as_deref(),
        Some("vpn-pass-1")
    );

    let bad_cfg = json!({
        "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 36},
        "openvpn": {"password": "inline-secret-forbidden"}
    });
    let inline_err = rt.save_app_config(&bad_cfg, false, None).unwrap_err();
    assert_eq!(inline_err.code, "INVALID_ARGUMENTS");
    assert_eq!(inline_err.reason, "inline_secrets_forbidden");

    let cfg_prev = rt
        .patch_app_config(
            &json!({"app": {"defaultTerminalCols": 132}, "boundary": {"addr": "https://b.internal"}}),
            true,
            None,
        )
        .expect("preview config patch");
    let cfg_rev = cfg_prev["revision"].as_str().unwrap().to_string();
    rt.patch_app_config(
        &json!({"app": {"defaultTerminalCols": 132}, "boundary": {"addr": "https://b.internal"}}),
        false,
        Some(&cfg_rev),
    )
    .expect("commit config patch");
    assert_eq!(rt.get_app_config().unwrap().app.default_terminal_cols, 132);
    assert!(rt
        .patch_app_config(
            &json!({"app": {"defaultTerminalCols": 80}}),
            false,
            Some(&cfg_rev)
        )
        .is_err());

    let esc_err = rt
        .patch_sync_settings(&json!({"mcpEnabled": true}), false, None)
        .unwrap_err();
    assert_eq!(esc_err.code, "INVALID_ARGUMENTS");
    assert_eq!(esc_err.reason, "desktop_service_activation_forbidden");

    let repo = root.path().join("sync-repo");
    std::fs::create_dir_all(&repo).unwrap();
    rt.patch_sync_settings(
        &json!({"repoRoot": repo.to_string_lossy(), "caddyFolder": "/tmp/caddy"}),
        false,
        None,
    )
    .expect("patch sync settings");
    assert_eq!(rt.get_sync_settings().unwrap().caddy_folder, "/tmp/caddy");

    let push_prev = rt.push_config_sync(None, true, None).expect("preview push");
    let push_rev = push_prev["revision"].as_str().unwrap().to_string();
    assert!(!repo.join("app-config").join("config.json").exists());
    rt.push_config_sync(None, false, Some(&push_rev))
        .expect("commit push");
    let pushed_cfg = std::fs::read_to_string(repo.join("app-config").join("config.json")).unwrap();
    assert!(!pushed_cfg.contains("vpn-pass-1"));
    assert!(repo.join("app-config").join("servers.json").exists());
    assert!(!repo.join("app-config").join("sync-local.json").exists());
    assert!(!repo.join("app-config").join("stdio-policy.json").exists());
    assert!(!repo.join("app-config").join("profile-meta.json").exists());

    let del_prev = rt.delete_server("srv-1", true, None).unwrap();
    let del_rev = del_prev["revision"].as_str().unwrap().to_string();
    rt.delete_server("srv-1", false, Some(&del_rev)).unwrap();
    assert!(backend
        .snapshot_entry(
            KEYRING_SERVICE,
            &format!("profile:{ns}:server_ssh_password:srv-1")
        )
        .is_none());
}

#[test]
fn reference_inventory_and_snapshot_work_without_tauri() {
    let dir = TempDir::new().unwrap();
    let ovpn = dir.path().join("client.ovpn");
    std::fs::write(&ovpn, "ca \"ca cert.pem\"\n<key>\nINLINE-SECRET\n</key>\n").unwrap();
    std::fs::write(
        dir.path().join("sync-local.json"),
        serde_json::to_vec(&json!({"openvpnOvpnPath": ovpn})).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("config.json"),
        serde_json::to_vec(
            &json!({"app": {"defaultTerminalCols": 120, "defaultTerminalRows": 36}}),
        )
        .unwrap(),
    )
    .unwrap();

    let mut store = MapSecrets::default();
    let archive = snapshot_with_version(dir.path(), "0.1.0", &mut store).unwrap();
    assert_eq!(archive.app_version, "0.1.0");
    assert!(archive
        .external_files
        .iter()
        .any(|f| f.path.ends_with("ca cert.pem")));
    let encoded = serde_json::to_string(&archive).unwrap();
    assert!(!encoded.contains("INLINE-SECRET"));

    let fp1 = fingerprint(&archive).unwrap();
    let mut mod_arch = fixture_archive();
    mod_arch.created_at_ms = 99999;
    let fp2 = fingerprint(&fixture_archive()).unwrap();
    assert_eq!(fingerprint(&mod_arch).unwrap(), fp2);
    assert_ne!(fp1, fp2);
    let _ = snapshot(dir.path(), &mut store).unwrap();
    let _ = FILES;
    let _ = CONFIGURED;
    let _ = LOCAL_PATHS;
    let _: Option<CoreError> = None;
}

#[test]
fn clear_secrets_and_refresh_pending_import_preview_work() {
    let root = TempDir::new().unwrap();
    let (rt, backend) = open_isolated_runtime(&root, "refresh");
    let ns = rt.credential_namespace().to_string();

    let srv = json!({
        "id": "srv-2",
        "name": "Linux Box",
        "host": "10.0.0.6",
        "os": "linux",
        "connectionMethod": "ssh",
        "sshPort": 22,
        "sshUser": "ops"
    });
    rt.upsert_server(&srv, false, None).unwrap();
    rt.set_server_ssh_password("srv-2", "temp-pass").unwrap();
    assert_eq!(
        backend
            .snapshot_entry(
                KEYRING_SERVICE,
                &format!("profile:{ns}:server_ssh_password:srv-2")
            )
            .as_deref(),
        Some("temp-pass")
    );
    rt.clear_server_ssh_password("srv-2").unwrap();
    assert!(backend
        .snapshot_entry(
            KEYRING_SERVICE,
            &format!("profile:{ns}:server_ssh_password:srv-2")
        )
        .is_none());

    rt.set_config_secret("boundary-password", "tok-123")
        .unwrap();
    assert_eq!(
        backend
            .snapshot_entry(
                KEYRING_SERVICE,
                &format!("profile:{ns}:{KEYRING_BOUNDARY_PASSWORD}")
            )
            .as_deref(),
        Some("tok-123")
    );
    rt.clear_config_secret("boundary-password").unwrap();
    assert!(backend
        .snapshot_entry(
            KEYRING_SERVICE,
            &format!("profile:{ns}:{KEYRING_BOUNDARY_PASSWORD}")
        )
        .is_none());

    let source = fixture_archive();
    let scpbak = root.path().join("refresh.scpbak");
    let encrypted = crypto::encrypt(&source, "correct horse battery staple").unwrap();
    backup_io::atomic_write(&scpbak, &encrypted, true).unwrap();

    let (first_preview, mut pending) = rt
        .prepare_settings_import(
            &scpbak,
            "correct horse battery staple",
            RestoreOptions::default(),
            Duration::from_secs(60),
        )
        .unwrap();

    let mut narrower = RestoreOptions {
        servers: false,
        ..Default::default()
    };
    let invalid = rt
        .refresh_pending_import_preview(&mut pending, narrower.clone(), Duration::from_secs(60))
        .unwrap_err();
    assert_eq!(invalid.code, "INVALID_ARGUMENTS");

    narrower.secrets = false;
    let refreshed = rt
        .refresh_pending_import_preview(&mut pending, narrower, Duration::from_secs(60))
        .unwrap();
    assert_ne!(refreshed.id, first_preview.id);
    assert!(pending.verify_identity(&first_preview.id).is_err());
    pending.verify_identity(&refreshed.id).unwrap();

    let res = rt.commit_pending_import(pending, &refreshed.id).unwrap();
    assert_eq!(res["applied"], true);
}

#[cfg(unix)]
#[test]
fn launch_server_rdp_enforces_gui_and_uses_safe_launcher() {
    use std::os::unix::fs::PermissionsExt;
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap();

    let root = TempDir::new().unwrap();
    let (rt, _) = open_isolated_runtime(&root, "rdp");
    let win_srv = json!({
        "id": "win-1",
        "name": "Win Box",
        "host": "win.internal",
        "os": "windows",
        "connectionMethod": "ssh",
        "sshPort": 22,
        "sshUser": "admin"
    });
    let linux_srv = json!({
        "id": "lin-1",
        "name": "Linux Box",
        "host": "lin.internal",
        "os": "linux",
        "connectionMethod": "ssh",
        "sshPort": 22,
        "sshUser": "admin"
    });
    rt.upsert_server(&win_srv, false, None).unwrap();
    rt.upsert_server(&linux_srv, false, None).unwrap();

    let prev_display = std::env::var_os("DISPLAY");
    let prev_wayland = std::env::var_os("WAYLAND_DISPLAY");
    let prev_launcher = std::env::var_os("SCPCTL_RDP_LAUNCHER");

    std::env::remove_var("DISPLAY");
    std::env::remove_var("WAYLAND_DISPLAY");
    let headless_err = rt.launch_server_rdp("win-1", true).unwrap_err();
    assert_eq!(headless_err.code, "CAPABILITY_UNAVAILABLE");

    let capture = root.path().join("rdp-args.txt");
    let launcher = root.path().join("fake-rdp.sh");
    std::fs::write(
        &launcher,
        format!(
            "#!/usr/bin/env sh\nprintf '%s' \"$1\" > \"{}\"\n",
            capture.display()
        ),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&launcher).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&launcher, perms).unwrap();

    std::env::set_var("DISPLAY", ":99");
    std::env::set_var("SCPCTL_RDP_LAUNCHER", &launcher);

    let linux_err = rt.launch_server_rdp("lin-1", true).unwrap_err();
    assert_eq!(linux_err.code, "CAPABILITY_UNAVAILABLE");
    let non_interactive_err = rt.launch_server_rdp("win-1", false).unwrap_err();
    assert_eq!(non_interactive_err.code, "CAPABILITY_UNAVAILABLE");

    let ok = rt.launch_server_rdp("win-1", true).unwrap();
    assert_eq!(ok["launched"], true);
    assert_eq!(ok["host"], "win.internal");
    assert_eq!(
        std::fs::read_to_string(&capture).unwrap(),
        "rdp://win.internal"
    );

    match prev_display {
        Some(v) => std::env::set_var("DISPLAY", v),
        None => std::env::remove_var("DISPLAY"),
    }
    match prev_wayland {
        Some(v) => std::env::set_var("WAYLAND_DISPLAY", v),
        None => std::env::remove_var("WAYLAND_DISPLAY"),
    }
    match prev_launcher {
        Some(v) => std::env::set_var("SCPCTL_RDP_LAUNCHER", v),
        None => std::env::remove_var("SCPCTL_RDP_LAUNCHER"),
    }
}
