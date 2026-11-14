use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

use scp_core::backup::{transaction, Changes};
use scp_core::error::CoreError;
use scp_core::gate::MaintenanceGate;
use scp_core::models::{
    default_app_config, ConnectionMethod, Server, ServerOs, StdioPolicy, KEYRING_OPENVPN_PASSWORD,
};
use scp_core::profile::{acquire_owner_lease, resolve_profile_paths, ProfileLocationInput};
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::{
    KeyringBackend, KeyringHealth, MemoryKeyringBackend, NamespacedSecretStore,
};

struct CountingLockedBackend {
    reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    writes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl KeyringBackend for CountingLockedBackend {
    fn preflight(&self) -> KeyringHealth {
        KeyringHealth::Locked
    }

    fn read_raw(&mut self, _service: &str, _account: &str) -> Result<Option<String>, CoreError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(CoreError::capability_unavailable(
            "keyring_locked",
            "OS keyring is locked",
            "unlock_keyring_and_retry",
        ))
    }

    fn write_raw(
        &mut self,
        _service: &str,
        _account: &str,
        _value: Option<&str>,
    ) -> Result<(), CoreError> {
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(CoreError::capability_unavailable(
            "keyring_locked",
            "OS keyring is locked",
            "unlock_keyring_and_retry",
        ))
    }
}

fn loc<'a>(
    config_dir: Option<&'a std::path::Path>,
    env_dir: Option<&'a std::path::Path>,
    xdg_dir: Option<&'a std::path::Path>,
    home_dir: Option<&'a std::path::Path>,
    desktop_dir: Option<&'a std::path::Path>,
    profile: Option<&'a str>,
) -> ProfileLocationInput<'a> {
    ProfileLocationInput {
        config_dir,
        env_config_dir: env_dir,
        xdg_config_home: xdg_dir,
        home_dir,
        desktop_config_dir: desktop_dir,
        profile,
    }
}

#[test]
fn profile_resolution_precedence_and_name_validation() {
    let tmp = TempDir::new().unwrap();
    let explicit = tmp.path().join("explicit");
    let env_dir = tmp.path().join("env");
    let xdg_dir = tmp.path().join("xdg");
    let home_dir = tmp.path().join("home");
    let desktop_dir = tmp.path().join("desktop");

    let p1 = resolve_profile_paths(&loc(
        Some(&explicit),
        Some(&env_dir),
        Some(&xdg_dir),
        Some(&home_dir),
        Some(&desktop_dir),
        Some("agent_1"),
    ))
    .unwrap();
    assert_eq!(p1.root, fs::canonicalize(&explicit).unwrap());
    assert_eq!(p1.profile_name, "agent_1");

    let p2 = resolve_profile_paths(&loc(
        None,
        Some(&env_dir),
        Some(&xdg_dir),
        Some(&home_dir),
        Some(&desktop_dir),
        None,
    ))
    .unwrap();
    assert_eq!(p2.root, fs::canonicalize(&env_dir).unwrap());
    assert_eq!(p2.profile_name, "default");

    let p3 = resolve_profile_paths(&loc(
        None,
        None,
        Some(&xdg_dir),
        Some(&home_dir),
        Some(&desktop_dir),
        Some("alpha"),
    ))
    .unwrap();
    assert_eq!(p3.root, fs::canonicalize(xdg_dir.join("scpctl")).unwrap());

    for bad in ["", "../escape", "BadCaps", "-leading", "space name"] {
        let err = resolve_profile_paths(&loc(
            Some(&explicit),
            None,
            None,
            None,
            Some(&desktop_dir),
            Some(bad),
        ))
        .unwrap_err();
        assert_eq!(err.code, "INVALID_ARGUMENTS");
        assert_eq!(err.reason, "invalid_profile_name");
        assert_eq!(err.exit_code(), 2);
    }
}

#[test]
fn profile_resolution_rejects_desktop_and_aliases_without_mutating_desktop() {
    let tmp = TempDir::new().unwrap();
    let desktop_dir = tmp.path().join("desktop_app");
    fs::create_dir_all(&desktop_dir).unwrap();

    // Direct desktop directory
    let err = resolve_profile_paths(&loc(
        Some(&desktop_dir),
        None,
        None,
        None,
        Some(&desktop_dir),
        Some("default"),
    ))
    .unwrap_err();
    assert_eq!(err.code, "POLICY_DENIED");
    assert_eq!(err.reason, "desktop_profile_rejected");
    assert_eq!(err.exit_code(), 3);
    assert!(!desktop_dir.join("profiles").exists());

    // Relative `..` alias into desktop dir
    let other_sibling = tmp.path().join("sibling");
    fs::create_dir_all(&other_sibling).unwrap();
    let dotdot_alias = other_sibling.join("..").join("desktop_app");
    let err = resolve_profile_paths(&loc(
        Some(&dotdot_alias),
        None,
        None,
        None,
        Some(&desktop_dir),
        Some("default"),
    ))
    .unwrap_err();
    assert_eq!(err.reason, "desktop_profile_rejected");
    assert!(!desktop_dir.join("profiles").exists());

    // Symlink pointing to desktop dir
    #[cfg(unix)]
    {
        let symlink_path = tmp.path().join("desktop_symlink");
        std::os::unix::fs::symlink(&desktop_dir, &symlink_path).unwrap();
        let err = resolve_profile_paths(&loc(
            Some(&symlink_path),
            None,
            None,
            None,
            Some(&desktop_dir),
            Some("default"),
        ))
        .unwrap_err();
        assert_eq!(err.reason, "desktop_profile_rejected");
        assert!(!desktop_dir.join("profiles").exists());

        // Symlink parent where profile leaf lives inside desktop dir
        let cli_root = tmp.path().join("cli_root");
        fs::create_dir_all(cli_root.join("profiles")).unwrap();
        std::os::unix::fs::symlink(&desktop_dir, cli_root.join("profiles").join("linked")).unwrap();
        let err = resolve_profile_paths(&loc(
            Some(&cli_root),
            None,
            None,
            None,
            Some(&desktop_dir),
            Some("linked"),
        ))
        .unwrap_err();
        assert_eq!(err.reason, "desktop_profile_rejected");
    }
}

#[test]
fn owner_lease_enforces_exclusion_across_canonical_aliases() {
    let tmp = TempDir::new().unwrap();
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");
    let paths1 = resolve_profile_paths(&loc(
        Some(&cli_root),
        None,
        None,
        None,
        Some(&desktop),
        Some("work"),
    ))
    .unwrap();
    let lease = acquire_owner_lease(&paths1, "rt-1", "query").unwrap();

    #[cfg(unix)]
    {
        let alias_root = tmp.path().join("alias_root");
        std::os::unix::fs::symlink(&cli_root, &alias_root).unwrap();
        let paths2 = resolve_profile_paths(&loc(
            Some(&alias_root),
            None,
            None,
            None,
            Some(&desktop),
            Some("work"),
        ))
        .unwrap();
        let err = acquire_owner_lease(&paths2, "rt-2", "query").unwrap_err();
        assert_eq!(err.code, "CONFLICT");
        assert_eq!(err.reason, "profile_owner_conflict");
        assert_eq!(err.exit_code(), 5);
    }

    drop(lease);
    let _lease2 = acquire_owner_lease(&paths1, "rt-3", "query").unwrap();
}

#[test]
fn maintenance_gate_blocks_concurrent_queries_when_maintaining() {
    let gate = MaintenanceGate::new();
    let maint = gate.maintain().unwrap();
    let err = std::thread::scope(|s| s.spawn(|| gate.enter().unwrap_err()).join().unwrap());
    assert_eq!(err.code, "CONFLICT");
    assert_eq!(err.reason, "maintenance_busy");

    // Re-entrant enter while same thread owns maintain succeeds
    let nested = gate.enter().unwrap();
    drop(nested);
    drop(maint);

    let shared = gate.enter().unwrap();
    drop(shared);
}

#[test]
fn missing_profile_meta_with_existing_recovery_journal_blocks_startup() {
    let tmp = TempDir::new().unwrap();
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");
    let paths = resolve_profile_paths(&loc(
        Some(&cli_root),
        None,
        None,
        None,
        Some(&desktop),
        Some("broken"),
    ))
    .unwrap();

    // Simulate interrupted restore where journal exists but profile-meta.json is missing
    fs::write(&paths.journal_path, b"not-done-encrypted-journal-bytes").unwrap();
    assert!(!paths.meta_path.exists());

    let backend = MemoryKeyringBackend::default();
    let err = ProfileRuntime::open_with_backend(
        &loc(
            Some(&cli_root),
            None,
            None,
            None,
            Some(&desktop),
            Some("broken"),
        ),
        "query",
        backend,
    )
    .err()
    .unwrap();
    assert_eq!(err.code, "CONFLICT");
    assert_eq!(err.reason, "missing_credential_namespace");
    assert!(!paths.meta_path.exists());
}

#[test]
fn startup_runs_recovery_before_step4_config_or_server_parsing() {
    let tmp = TempDir::new().unwrap();
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");
    let input = loc(
        Some(&cli_root),
        None,
        None,
        None,
        Some(&desktop),
        Some("recover_ok"),
    );
    let shared_keyring = MemoryKeyringBackend::default();

    // 1. Initialize profile cleanly so it gets its stable namespace
    let ns = {
        let rt = ProfileRuntime::open_with_backend(&input, "init", shared_keyring.clone()).unwrap();
        let ns = rt.credential_namespace().to_string();
        let original_servers = json!({
            "servers": [{
                "id": "srv-1",
                "name": "bastion",
                "host": "10.0.0.10",
                "os": "linux",
                "connectionMethod": "ssh",
                "sshPort": 22,
                "sshUser": "ops",
                "boundaryTargetId": "",
                "boundaryListenPort": 0,
                "services": []
            }]
        });
        fs::write(
            &rt.profile_paths().servers_path,
            serde_json::to_string_pretty(&original_servers).unwrap(),
        )
        .unwrap();

        // Prepare a pending restore journal that snapshots srv-1, then corrupt servers.json
        let mut store = NamespacedSecretStore::new(
            scp_core::models::KEYRING_SERVICE,
            &ns,
            shared_keyring.clone(),
        );
        let changes = Changes {
            files: BTreeMap::from([("servers.json".into(), Some(json!({"servers":[]})))]),
            credentials: BTreeMap::new(),
        };
        transaction::prepare(&rt.profile_paths().profile_dir, &changes, &mut store).unwrap();
        fs::write(&rt.profile_paths().servers_path, b"{CORRUPTED_JSON").unwrap();
        ns
    };

    // 2. Reopen: Step 3 MUST recover original servers.json BEFORE Step 4 loads it
    let rt = ProfileRuntime::open_with_backend(&input, "query", shared_keyring.clone()).unwrap();
    assert_eq!(rt.credential_namespace(), ns);
    let page = rt.list_servers(None, None).unwrap();
    assert_eq!(page.servers.len(), 1);
    assert_eq!(page.servers[0].id, "srv-1");
    assert!(!rt.profile_paths().journal_path.exists());

    // 3. Now simulate corrupted journal / failing keyring while servers.json is also broken:
    // Step 3 MUST fail with recovery_blocked BEFORE Step 4 reports servers_corrupted.
    let failing_backend = CountingLockedBackend {
        reads: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        writes: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    fs::write(&rt.profile_paths().journal_path, b"corrupted-journal-data").unwrap();
    fs::write(&rt.profile_paths().servers_path, b"{CORRUPTED_AGAIN").unwrap();
    drop(rt);

    let err = ProfileRuntime::open_with_backend(&input, "query", failing_backend)
        .err()
        .unwrap();
    assert_eq!(err.code, "CONFLICT");
    assert_eq!(err.reason, "recovery_blocked");
    assert_eq!(err.exit_code(), 5);
}

#[test]
fn query_path_skips_keyring_and_migrates_only_when_plaintext_secrets_exist() {
    let tmp = TempDir::new().unwrap();
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");
    let input = loc(
        Some(&cli_root),
        None,
        None,
        None,
        Some(&desktop),
        Some("no_keyring"),
    );

    let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counting = CountingLockedBackend {
        reads: reads.clone(),
        writes: writes.clone(),
    };

    // Empty / normal profile with locked keyring MUST succeed with 0 keyring reads!
    let rt = ProfileRuntime::open_with_backend(&input, "query", counting).unwrap();
    let _ = rt.list_servers(None, None).unwrap();
    let _ = rt.list_function_statuses().unwrap();
    assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 0);
    drop(rt);

    // If config.json contains legacy plaintext password, migration uses the namespaced keyring
    let mut cfg = default_app_config();
    cfg.openvpn.password = "legacy-secret-123".into();
    let cfg_path = resolve_profile_paths(&input).unwrap().config_path;
    fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();

    let memory = MemoryKeyringBackend::default();
    let rt2 = ProfileRuntime::open_with_backend(&input, "query", memory.clone()).unwrap();
    let ns = rt2.credential_namespace().to_string();
    let disk_raw = fs::read_to_string(&rt2.profile_paths().config_path).unwrap();
    assert!(!disk_raw.contains("legacy-secret-123"));

    // Verify stored under namespaced key, NOT bare desktop key
    assert_eq!(
        memory.snapshot_entry(
            scp_core::models::KEYRING_SERVICE,
            &format!("profile:{ns}:{KEYRING_OPENVPN_PASSWORD}")
        ),
        Some("legacy-secret-123".into())
    );
    assert_eq!(
        memory.snapshot_entry(scp_core::models::KEYRING_SERVICE, KEYRING_OPENVPN_PASSWORD),
        None
    );
}

#[test]
fn namespaced_secret_store_isolates_cache_and_storage_by_namespace() {
    let backend = MemoryKeyringBackend::default();
    let mut store_a =
        NamespacedSecretStore::new(scp_core::models::KEYRING_SERVICE, "ns_a", backend.clone());
    let mut store_b =
        NamespacedSecretStore::new(scp_core::models::KEYRING_SERVICE, "ns_b", backend.clone());

    store_a.set(KEYRING_OPENVPN_PASSWORD, "secret-a").unwrap();
    store_b.set(KEYRING_OPENVPN_PASSWORD, "secret-b").unwrap();

    assert_eq!(
        store_a.cached_get(KEYRING_OPENVPN_PASSWORD),
        Some("secret-a")
    );
    assert_eq!(
        store_b.cached_get(KEYRING_OPENVPN_PASSWORD),
        Some("secret-b")
    );
    assert_eq!(
        store_a.get(KEYRING_OPENVPN_PASSWORD).unwrap().as_deref(),
        Some("secret-a")
    );
    assert_eq!(
        store_b.get(KEYRING_OPENVPN_PASSWORD).unwrap().as_deref(),
        Some("secret-b")
    );

    store_a.delete(KEYRING_OPENVPN_PASSWORD).unwrap();
    assert_eq!(store_a.cached_get(KEYRING_OPENVPN_PASSWORD), None);
    assert_eq!(
        store_b.get(KEYRING_OPENVPN_PASSWORD).unwrap().as_deref(),
        Some("secret-b")
    );
}

#[test]
fn bounded_server_pagination_exact_lookup_and_isolated_stdio_policy() {
    let tmp = TempDir::new().unwrap();
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");
    let input = loc(
        Some(&cli_root),
        None,
        None,
        None,
        Some(&desktop),
        Some("servers"),
    );
    let paths = resolve_profile_paths(&input).unwrap();

    let servers: Vec<Server> = ["srv-c", "srv-a", "srv-b"]
        .into_iter()
        .map(|id| Server {
            id: id.into(),
            name: format!("Name {id}"),
            host: "10.0.0.1".into(),
            os: ServerOs::Linux,
            connection_method: ConnectionMethod::Ssh,
            ssh_port: 22,
            ssh_user: "root".into(),
            boundary_target_id: String::new(),
            boundary_listen_port: 0,
            services: vec![],
        })
        .collect();
    fs::write(
        &paths.servers_path,
        serde_json::to_string_pretty(&json!({"servers": servers})).unwrap(),
    )
    .unwrap();

    // sync-local.json attempts to allow everything -> MUST NOT escalate StdioPolicy!
    fs::write(
        &paths.sync_local_path,
        serde_json::to_string_pretty(&json!({
            "mcpSensitiveActionPolicy": "allow-all",
            "mcpAllowedToolCategories": ["read-only", "runtime-control", "sensitive"]
        }))
        .unwrap(),
    )
    .unwrap();

    let rt = ProfileRuntime::open_with_backend(&input, "stdio", MemoryKeyringBackend::default())
        .unwrap();

    // Default stdio policy ignores sync-local.json
    assert_eq!(rt.get_stdio_policy().unwrap(), &StdioPolicy::default());

    // Sorted order: srv-a, srv-b, srv-c
    let p1 = rt.list_servers(Some(2), None).unwrap();
    assert_eq!(
        p1.servers.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["srv-a", "srv-b"]
    );
    assert!(p1.truncated);
    assert_eq!(p1.next_after.as_deref(), Some("srv-b"));
    assert_eq!(p1.total, 3);

    let p2 = rt.list_servers(Some(2), p1.next_after.as_deref()).unwrap();
    assert_eq!(
        p2.servers.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["srv-c"]
    );
    assert!(!p2.truncated);
    assert_eq!(p2.next_after, None);

    let err = rt.list_servers(Some(0), None).unwrap_err();
    assert_eq!(err.code, "INVALID_ARGUMENTS");

    let err = rt.list_servers(Some(10), Some("srv-missing")).unwrap_err();
    assert_eq!(err.code, "INVALID_ARGUMENTS");

    assert_eq!(rt.get_server("srv-b").unwrap().name, "Name srv-b");
    let err = rt.get_server("srv-none").unwrap_err();
    assert_eq!(err.code, "NOT_FOUND");
    assert_eq!(err.exit_code(), 6);
}
