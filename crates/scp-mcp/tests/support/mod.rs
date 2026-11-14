use serde_json::{json, Value};
use std::fs;
use tempfile::TempDir;

use scp_core::models::{ConnectionMethod, Server, ServerOs};
use scp_core::profile::{resolve_profile_paths, ProfileLocationInput};
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::{KeyringHealth, MemoryKeyringBackend};

#[allow(dead_code)]
pub fn open_seeded_stdio_runtime(
    host: &str,
    health: KeyringHealth,
    sync_local: Option<Value>,
    stdio_policy: Option<Value>,
) -> (
    TempDir,
    ProfileRuntime<MemoryKeyringBackend>,
    MemoryKeyringBackend,
) {
    let tmp = TempDir::new().unwrap();
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");
    let input = ProfileLocationInput {
        config_dir: Some(&cli_root),
        desktop_config_dir: Some(&desktop),
        profile: Some("agent"),
        ..Default::default()
    };
    let paths = resolve_profile_paths(&input).unwrap();
    fs::create_dir_all(&paths.profile_dir).unwrap();
    let server = Server {
        id: "srv-1".into(),
        name: "app-box".into(),
        host: host.into(),
        os: ServerOs::Linux,
        connection_method: ConnectionMethod::Ssh,
        ssh_port: 22,
        ssh_user: "deploy".into(),
        boundary_target_id: String::new(),
        boundary_listen_port: 0,
        services: vec![],
    };
    fs::write(
        &paths.servers_path,
        serde_json::to_string_pretty(&json!({ "servers": [server] })).unwrap(),
    )
    .unwrap();
    if let Some(sync) = sync_local {
        fs::write(
            &paths.sync_local_path,
            serde_json::to_string_pretty(&sync).unwrap(),
        )
        .unwrap();
    }
    if let Some(policy) = stdio_policy {
        fs::write(&paths.stdio_policy_path, policy.to_string()).unwrap();
    }
    let backend = MemoryKeyringBackend::new(health);
    let rt = ProfileRuntime::open_with_backend(&input, "stdio", backend.clone()).unwrap();
    (tmp, rt, backend)
}
