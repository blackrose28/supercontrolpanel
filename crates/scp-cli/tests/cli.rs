use serde_json::{json, Value};
use std::{
    fs,
    process::{Command, Output},
};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scpctl"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn lists_servers_directly_without_desktop_or_http_and_keeps_files_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let raw = json!({"servers":[{
        "id":"srv-offline", "name":"Offline inventory", "host":"example.test",
        "connectionMethod":"ssh", "sshPort":22, "sshUser":"user"
    }]});
    let path = temp.path().join("servers.json");
    fs::write(&path, raw.to_string()).unwrap();
    fs::write(
        temp.path().join("sync-local.json"),
        r#"{"mcpEnabled":false,"mcpAuthToken":"do-not-print"}"#,
    )
    .unwrap();
    let output = cli(&[
        "server",
        "list",
        "--config-dir",
        temp.path().to_str().unwrap(),
        "--json",
    ]);
    assert!(output.status.success(), "{:?}", output);
    let servers: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(servers[0]["id"], "srv-offline");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("do-not-print"));
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read_to_string(path).unwrap(), raw.to_string());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    let human = cli(&[
        "--config-dir",
        temp.path().to_str().unwrap(),
        "server",
        "list",
    ]);
    assert!(human.status.success());
    assert!(String::from_utf8_lossy(&human.stdout).contains("Offline inventory"));
}

#[test]
fn missing_inventory_returns_empty_json_without_creating_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("missing");
    let output = cli(&[
        "--config-dir",
        root.to_str().unwrap(),
        "--json",
        "server",
        "list",
    ]);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!([])
    );
    assert!(!root.exists());
}

#[test]
#[cfg(target_os = "linux")]
fn default_profile_is_separate_from_desktop_and_follows_platform_config_root() {
    let temp = tempfile::tempdir().unwrap();
    let config_root = temp.path().join("config");
    let profile = config_root.join("supercontrolpanel-cli");
    fs::create_dir_all(&profile).unwrap();
    fs::write(profile.join("servers.json"), r#"{"servers":[{"id":"cli-profile","name":"CLI","host":"host","connectionMethod":"ssh","sshPort":22,"sshUser":"user"}]}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scpctl"))
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", &config_root)
        .env("APPDATA", &config_root)
        .args(["server", "list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let servers: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(servers[0]["id"], "cli-profile");
}

#[test]
fn errors_use_stderr_and_distinct_usage_and_operation_exit_codes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("servers.json"), "invalid-private-contents").unwrap();
    let output = cli(&[
        "--config-dir",
        temp.path().to_str().unwrap(),
        "server",
        "list",
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("parse servers"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("invalid-private-contents"));
    for args in [
        &["server", "start"][..],
        &["--config-dir"][..],
        &["mcp", "serve", "--stdio"][..],
    ] {
        let output = cli(args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn help_and_version_are_available_without_a_profile() {
    let help = cli(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--config-dir"));
    let version = cli(&["--version"]);
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("scpctl "));
}
