use scp_core::servers::{read_servers, ConnectionMethod, Server, ServerOs};
use serde_json::json;
use std::fs;

#[test]
fn desktop_server_shape_and_legacy_defaults_are_preserved() {
    let input = json!({
        "id": "srv-test", "name": "Production", "host": "example.test",
        "connectionMethod": "ssh", "sshPort": 2222, "sshUser": "alice"
    });
    let server: Server = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(server.os, ServerOs::Linux);
    assert_eq!(server.connection_method, ConnectionMethod::Ssh);
    assert_eq!(
        serde_json::to_value(&server).unwrap(),
        json!({
            "os": "linux", "boundaryTargetId": "", "boundaryListenPort": 0,
            "services": [],
            "id": "srv-test", "name": "Production", "host": "example.test",
            "connectionMethod": "ssh", "sshPort": 2222, "sshUser": "alice"
        })
    );
    let boundary = json!({
        "os": "windows", "connectionMethod": "boundary", "boundaryTargetId": "ttcp_test",
        "boundaryListenPort": 2223, "services": [{"name": "web", "port": 443}],
        "id": "srv-test", "name": "Production", "host": "example.test",
        "sshPort": 22, "sshUser": "alice"
    });
    let server: Server = serde_json::from_value(boundary.clone()).unwrap();
    assert_eq!(server.connection_method, ConnectionMethod::Boundary);
    assert_eq!(serde_json::to_value(server).unwrap(), boundary);
}

#[test]
fn missing_inventory_is_empty_and_does_not_create_a_profile() {
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path().join("absent");
    assert!(read_servers(&profile).unwrap().is_empty());
    assert!(!profile.exists());
}

#[test]
fn inventory_read_is_read_only_and_ignores_unrelated_local_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let raw = r#"{"servers":[{"id":"srv-1","name":"Test","host":"host","connectionMethod":"ssh","sshPort":22,"sshUser":"user"}]}"#;
    fs::write(temp.path().join("servers.json"), raw).unwrap();
    fs::write(
        temp.path().join("sync-local.json"),
        "invalid secret settings",
    )
    .unwrap();
    assert_eq!(read_servers(temp.path()).unwrap()[0].id, "srv-1");
    assert_eq!(
        fs::read_to_string(temp.path().join("servers.json")).unwrap(),
        raw
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}

#[test]
fn invalid_inventory_and_io_errors_are_explicit_without_echoing_contents() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("servers.json");
    fs::write(&path, r#"{"servers":["private-value"]}"#).unwrap();
    let error = read_servers(temp.path()).unwrap_err();
    assert!(error.contains("parse servers"), "{error}");
    assert!(!error.contains("private-value"), "{error}");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(read_servers(temp.path())
        .unwrap_err()
        .contains("read servers"));
}
