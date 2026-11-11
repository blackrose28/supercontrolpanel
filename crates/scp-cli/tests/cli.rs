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

#[test]
fn profile_init_creates_only_local_identity_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path().join("new-profile");
    let args = [
        "--config-dir",
        profile.to_str().unwrap(),
        "profile",
        "init",
        "--json",
    ];
    let output = cli(&args);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value["credentialNamespace"]
        .as_str()
        .unwrap()
        .starts_with("supercontrolpanel-cli-"));
    let again = cli(&args);
    assert!(again.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&again.stdout).unwrap(),
        value
    );
    assert_eq!(fs::read_dir(&profile).unwrap().count(), 2);
    fs::write(profile.join("profile.json"), "invalid-private-metadata").unwrap();
    let invalid = cli(&args);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("invalid-private-metadata"));
}

#[test]
fn profile_init_reports_owner_conflict_but_inventory_stays_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let owner = scp_core::profile::ProfileOwner::acquire(temp.path()).unwrap();
    let before = fs::read(temp.path().join("profile.json")).unwrap();
    let output = cli(&[
        "--config-dir",
        temp.path().to_str().unwrap(),
        "profile",
        "init",
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already owned"));
    let inventory = cli(&[
        "--config-dir",
        temp.path().to_str().unwrap(),
        "server",
        "list",
        "--json",
    ]);
    assert!(inventory.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&inventory.stdout).unwrap(),
        json!([])
    );
    assert_eq!(fs::read(temp.path().join("profile.json")).unwrap(), before);
    drop(owner);
}

#[test]
fn caddy_requires_explicit_foreground_folder_and_log() {
    let help = cli(&["caddy", "run", "--help"]);
    assert!(help.status.success(), "{help:?}");
    for flag in ["--foreground", "--folder", "--output"] {
        assert!(String::from_utf8_lossy(&help.stdout).contains(flag));
    }
    for args in [
        vec!["caddy", "run", "--folder", ".", "--output", "log"],
        vec!["caddy", "run", "--foreground", "--output", "log"],
        vec!["caddy", "run", "--foreground", "--folder", "."],
    ] {
        assert_eq!(cli(&args).status.code(), Some(2));
    }
}

#[test]
#[cfg(target_os = "linux")]
fn caddy_validation_and_log_conflicts_do_not_launch_or_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("Caddy folder");
    let profile = temp.path().join("profile");
    let log = temp.path().join("log");
    fs::create_dir(&folder).unwrap();
    let args = [
        "--config-dir",
        profile.to_str().unwrap(),
        "--json",
        "caddy",
        "run",
        "--foreground",
        "--folder",
        folder.to_str().unwrap(),
        "--output",
        log.to_str().unwrap(),
    ];
    let missing = cli(&args);
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("Caddyfile not found"));
    assert!(!profile.exists());
    assert!(!log.exists());
    fs::write(folder.join("Caddyfile"), "never execute user configuration").unwrap();
    fs::write(&log, "keep existing log").unwrap();
    let conflict = cli(&args);
    assert_eq!(conflict.status.code(), Some(1));
    assert!(conflict.stdout.is_empty());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("create Caddy output file"));
    assert_eq!(fs::read_to_string(&log).unwrap(), "keep existing log");
    assert!(scp_core::profile::ProfileOwner::acquire(&profile).is_ok());
}

#[test]
#[cfg(target_os = "linux")]
fn caddy_real_cli_keeps_completed_raw_bytes_out_of_json_and_returns_failure() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    let folder = root.path().join("Caddy folder λ ; literal");
    let log = root.path().join("raw-output");
    let profile = root.path().join("profile");
    fs::create_dir(&bin).unwrap();
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("Caddyfile"), "not executed by tests").unwrap();
    // The shim immediately execs this test executable: the PTY's direct child
    // becomes the Rust fixture, with no descendant keeping the PTY alive.
    let shim = bin.join("caddy");
    fs::write(&shim, "#!/bin/sh\nexec \"$SCP_CADDY_FIXTURE_EXE\" --ignored --exact caddy_cli_child_fixture --nocapture\n").unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o700)).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_scpctl"))
        .env("PATH", &bin)
        .env("SCP_CADDY_FIXTURE_EXE", std::env::current_exe().unwrap())
        .env("SCP_CADDY_FIXTURE_ROOT", root.path())
        .args([
            "--config-dir",
            profile.to_str().unwrap(),
            "--json",
            "caddy",
            "run",
            "--foreground",
            "--folder",
            folder.to_str().unwrap(),
            "--output",
            log.to_str().unwrap(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = child;
    let start = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > std::time::Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("CLI fixture timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"outcome":"completed", "exitCode":7})
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Caddy exited"));
    let raw = fs::read(log).unwrap();
    assert!(raw.len() > 128 * 1024);
    assert!(String::from_utf8_lossy(&raw).contains("RAW-FINAL-MARKER"));
    assert_eq!(
        fs::read_to_string(root.path().join("cwd")).unwrap(),
        folder.to_str().unwrap()
    );
    assert!(scp_core::profile::ProfileOwner::acquire(&profile).is_ok());
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "direct child fixture exercised by Caddy CLI subprocess test"]
fn caddy_cli_child_fixture() {
    use std::io::Write;
    let root = std::path::PathBuf::from(std::env::var_os("SCP_CADDY_FIXTURE_ROOT").unwrap());
    fs::write(
        root.join("cwd"),
        std::env::current_dir().unwrap().to_str().unwrap(),
    )
    .unwrap();
    for _ in 0..2048 {
        println!("{}", "x".repeat(64));
    }
    println!("RAW-FINAL-MARKER");
    std::io::stdout().flush().unwrap();
    std::process::exit(7);
}

#[test]
#[cfg(not(target_os = "linux"))]
fn unqualified_caddy_platform_fails_before_creating_profile_or_log() {
    let root = tempfile::tempdir().unwrap();
    let output = cli(&[
        "--config-dir",
        root.path().join("profile").to_str().unwrap(),
        "caddy",
        "run",
        "--foreground",
        "--folder",
        "missing",
        "--output",
        root.path().join("log").to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Linux preview"));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn config_show_projects_legacy_fields_without_touching_other_settings() {
    let temp = tempfile::tempdir().unwrap();
    let raw = json!({
        "app": {"defaultTerminalCols": 91, "defaultTerminalRows": 27},
        "openvpn": {
            "password": "vpn-secret", "otpSecret": "vpn-otp", "command": "legacy-command",
            "executablePath": "local-vpn", "ovpnPath": "local-ovpn", "username": "alice",
            "extraArgs": "sensitive-free-text\n\u{1b}[31m"
        },
        "boundary": {
            "cliPath": "local-boundary", "password": "boundary-secret",
            "oidcPassword": "oidc-secret", "oidcOtpSecret": "oidc-otp",
            "addr": "https://controller.test", "authKind": "password"
        }
    })
    .to_string();
    let path = temp.path().join("config.json");
    fs::write(&path, &raw).unwrap();
    fs::write(
        temp.path().join("sync-local.json"),
        "invalid private local settings",
    )
    .unwrap();
    fs::write(temp.path().join("profile.json"), "invalid identity").unwrap();
    let output = cli(&[
        "config",
        "show",
        "--json",
        "--config-dir",
        temp.path().to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let projected: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(projected["app"]["defaultTerminalCols"], 91);
    assert_eq!(
        projected["openvpn"],
        json!({
            "command": "", "extraArgs": "sensitive-free-text\n\u{1b}[31m", "username": "alice",
            "terminateOpenVpnWhenHelperExits": false, "autoReconnectEnabled": false,
            "autoReconnectBaseSeconds": 5, "autoReconnectMaxSeconds": 3600
        })
    );
    assert_eq!(
        projected["boundary"],
        json!({
            "addr": "https://controller.test", "authMethodId": "", "loginName": "",
            "authKind": "password", "oidcLoginMode": "browser", "oidcUsername": "",
            "connectSshOptions": ""
        })
    );
    let human = cli(&[
        "--config-dir",
        temp.path().to_str().unwrap(),
        "config",
        "show",
    ]);
    assert!(human.status.success(), "{human:?}");
    assert!(human.stderr.is_empty());
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text
        .starts_with("Shared configuration (config.json; free-text values may be sensitive):\n"));
    assert!(!text.contains('\u{1b}'));
    let (_, body) = text.split_once('\n').unwrap();
    assert_eq!(serde_json::from_str::<Value>(body).unwrap(), projected);
    for excluded in [
        "vpn-secret",
        "vpn-otp",
        "legacy-command",
        "local-vpn",
        "local-ovpn",
        "local-boundary",
        "boundary-secret",
        "oidc-secret",
        "oidc-otp",
        "invalid private",
        "invalid identity",
    ] {
        assert!(!String::from_utf8_lossy(&output.stdout).contains(excluded));
        assert!(!text.contains(excluded));
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), raw);
    assert_eq!(
        fs::read_to_string(temp.path().join("sync-local.json")).unwrap(),
        "invalid private local settings"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("profile.json")).unwrap(),
        "invalid identity"
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 3);
}

#[test]
fn config_show_missing_file_returns_null_without_initializing_or_synthesizing_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    for root in [&missing, temp.path()] {
        let output = cli(&[
            "--json",
            "--config-dir",
            root.to_str().unwrap(),
            "config",
            "show",
        ]);
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            Value::Null
        );
        let human = cli(&["config", "show", "--config-dir", root.to_str().unwrap()]);
        assert!(human.status.success(), "{human:?}");
        assert_eq!(
            human.stdout,
            b"No shared configuration (config.json) in this CLI profile.\n"
        );
        assert!(human.stderr.is_empty());
    }
    assert!(!missing.exists());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn config_show_errors_are_sanitized_and_usage_is_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    for raw in [
        r#"{"app":"private-config-value"}"#,
        r#"{"app":{"defaultTerminalCols":80,"defaultTerminalRows":24},"boundary":{"authKind":"private-config-value"}}"#,
        r#"{"private-config-value":"#,
    ] {
        fs::write(&path, raw).unwrap();
        for flags in [vec!["--json"], vec![]] {
            let mut args = vec![
                "config",
                "show",
                "--config-dir",
                temp.path().to_str().unwrap(),
            ];
            args.extend(flags);
            let output = cli(&args);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(output.stdout.is_empty());
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains("parse config"), "{error}");
            assert!(!error.contains("private-config-value"), "{error}");
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), raw);
    }
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let output = cli(&[
        "config",
        "show",
        "--json",
        "--config-dir",
        temp.path().to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("read config"));
    for args in [
        vec!["config"],
        vec!["config", "export"],
        vec!["config", "show", "--unknown"],
    ] {
        let output = cli(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    let help = cli(&["config", "show", "--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("sensitive"));
}

#[test]
fn config_show_reads_while_another_process_owns_profile() {
    let temp = tempfile::tempdir().unwrap();
    let owner = scp_core::profile::ProfileOwner::acquire(temp.path()).unwrap();
    let before = fs::read(temp.path().join("profile.json")).unwrap();
    fs::write(
        temp.path().join("config.json"),
        r#"{"app":{"defaultTerminalCols":100,"defaultTerminalRows":40}}"#,
    )
    .unwrap();
    let output = cli(&[
        "config",
        "show",
        "--json",
        "--config-dir",
        temp.path().to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["app"]["defaultTerminalCols"],
        100
    );
    assert_eq!(fs::read(temp.path().join("profile.json")).unwrap(), before);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 3);
    assert!(scp_core::profile::ProfileOwner::acquire(temp.path()).is_err());
    drop(owner);
}

#[test]
#[cfg(target_os = "linux")]
fn config_show_default_profile_uses_isolated_cli_directory() {
    let temp = tempfile::tempdir().unwrap();
    let config_root = temp.path().join("config");
    for (directory, cols) in [("supercontrolpanel-cli", 123), ("supercontrolpanel", 99)] {
        let profile = config_root.join(directory);
        fs::create_dir_all(&profile).unwrap();
        fs::write(
            profile.join("config.json"),
            json!({"app":{"defaultTerminalCols":cols,"defaultTerminalRows":24}}).to_string(),
        )
        .unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_scpctl"))
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", &config_root)
        .args(["config", "show", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["app"]["defaultTerminalCols"],
        123
    );
    assert_eq!(
        fs::read_dir(config_root.join("supercontrolpanel-cli"))
            .unwrap()
            .count(),
        1
    );
}
