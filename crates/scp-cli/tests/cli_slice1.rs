use scp_core::profile::{resolve_profile_paths, ProfileLocationInput};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn scpctl_bin() -> &'static str {
    env!("CARGO_BIN_EXE_scpctl")
}

fn run_with_env(args: &[&str], desktop_dir: &Path, extra_env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(scpctl_bin());
    cmd.args(args)
        .env("SCPCTL_DESKTOP_CONFIG_DIR", desktop_dir)
        .stdin(Stdio::null());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn seed_profile(
    cli_root: &Path,
    desktop_dir: &Path,
    profile: &str,
) -> scp_core::profile::ProfilePaths {
    let paths = resolve_profile_paths(&ProfileLocationInput {
        config_dir: Some(cli_root),
        desktop_config_dir: Some(desktop_dir),
        profile: Some(profile),
        ..Default::default()
    })
    .unwrap();
    fs::write(
        &paths.servers_path,
        serde_json::to_string_pretty(&json!({
            "servers": [
                {"id": "srv-a", "name": "alpha", "host": "10.0.0.1", "os": "linux", "connectionMethod": "ssh", "sshPort": 22, "sshUser": "ops"},
                {"id": "srv-b", "name": "bravo", "host": "10.0.0.2", "os": "linux", "connectionMethod": "ssh", "sshPort": 22, "sshUser": "ops"},
                {"id": "srv-c", "name": "charlie", "host": "10.0.0.3", "os": "windows", "connectionMethod": "boundary", "sshPort": 22, "sshUser": "admin", "boundaryTargetId": "ttcp_1", "boundaryListenPort": 2222}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        &paths.sync_local_path,
        serde_json::to_string_pretty(&json!({
            "mcpSensitiveActionPolicy": "allow-with-token",
            "mcpAllowedToolCategories": ["read-only", "runtime-control", "config-mutation", "file-transfer"],
            "mcpAuthToken": "secret-mcp-tok-xyz",
            "ccAgentToken": "secret-cc-tok-xyz"
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        &paths.config_path,
        serde_json::to_string_pretty(&json!({
            "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 36},
            "openvpn": {"username": "vpn-user"},
            "boundary": {"addr": "https://boundary.example.com", "loginName": "b-user"}
        }))
        .unwrap(),
    )
    .unwrap();
    paths
}

fn parse_single_json(output: &Output) -> Value {
    assert_eq!(
        output.stdout.iter().filter(|b| **b == b'\n').count(),
        1,
        "expected single-line JSON, got stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn schema_command_filter_and_unknown_command_rejection() {
    let tmp = TempDir::new().unwrap();
    let desktop = tmp.path().join("desktop");
    let ok = run_with_env(
        &["schema", "--command", "server.list", "--json"],
        &desktop,
        &[],
    );
    assert!(ok.status.success());
    let body = parse_single_json(&ok);
    let cmds = body["data"]["commands"].as_array().unwrap();
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0]["command"], "server.list");

    let bad = run_with_env(
        &["schema", "--command", "unknown.cmd", "--json"],
        &desktop,
        &[],
    );
    assert_eq!(bad.status.code(), Some(2));
    assert_eq!(
        parse_single_json(&bad)["error"]["code"],
        "INVALID_ARGUMENTS"
    );

    let missing_val = run_with_env(&["schema", "--command", "--json"], &desktop, &[]);
    assert_eq!(missing_val.status.code(), Some(2));
}

#[test]
fn conflicting_flags_and_mcp_stdout_silence() {
    let tmp = TempDir::new().unwrap();
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");
    let root_str = cli_root.to_str().unwrap();

    for args in [
        vec![
            "--config-dir",
            root_str,
            "status",
            "--interactive",
            "--json",
        ],
        vec![
            "--config-dir",
            root_str,
            "capabilities",
            "--no-profile",
            "--interactive",
            "--non-interactive",
            "--json",
        ],
        vec![
            "--config-dir",
            root_str,
            "status",
            "--interactive",
            "--non-interactive",
            "--json",
        ],
        vec![
            "--config-dir",
            root_str,
            "mcp",
            "serve",
            "--stdio",
            "--json",
        ],
        vec!["--config-dir", root_str, "mcp", "serve", "--json"],
    ] {
        let out = run_with_env(&args, &desktop, &[]);
        assert_eq!(out.status.code(), Some(2), "args={args:?}");
        assert!(
            !cli_root.exists(),
            "invalid args must not create config dir"
        );
        if args.contains(&"mcp") {
            assert!(out.stdout.is_empty(), "mcp errors must keep stdout empty");
            assert!(!out.stderr.is_empty());
        } else {
            let body = parse_single_json(&out);
            assert_eq!(body["error"]["code"], "INVALID_ARGUMENTS");
            assert_eq!(body["error"]["outcome"], "not_started");
        }
    }
}

#[test]
fn capabilities_without_and_with_profile() {
    let tmp = TempDir::new().unwrap();
    let nonexistent = tmp.path().join("must_not_create");
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");

    let no_prof = run_with_env(
        &["capabilities", "--no-profile", "--json"],
        &desktop,
        &[("SCPCTL_CONFIG_DIR", nonexistent.to_str().unwrap())],
    );
    assert!(no_prof.status.success());
    assert!(!nonexistent.exists());
    let body = parse_single_json(&no_prof);
    assert!(body["runtimeId"].is_null());
    assert_eq!(body["data"]["profileOpen"], false);
    assert_eq!(
        body["data"]["stdioPolicy"]["sensitiveActionPolicy"],
        "read-only"
    );
    assert_eq!(
        body["data"]["availableOperations"]
            .as_array()
            .unwrap()
            .len(),
        9
    );

    let with_prof = run_with_env(
        &[
            "--config-dir",
            cli_root.to_str().unwrap(),
            "--profile",
            "agent",
            "capabilities",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert!(with_prof.status.success());
    let body = parse_single_json(&with_prof);
    assert!(body["runtimeId"].as_str().unwrap().starts_with("rt-"));
    assert_eq!(body["data"]["profileOpen"], true);
    assert!(!body["data"]["profile"]["credentialNamespace"]
        .as_str()
        .unwrap()
        .is_empty());
}

#[test]
fn seeded_profile_queries_and_redaction() {
    let tmp = TempDir::new().unwrap();
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");
    let paths = seed_profile(&cli_root, &desktop, "agent");
    let root = cli_root.to_str().unwrap();

    let st_all = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "agent",
            "status",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert!(st_all.status.success());
    let st_body = parse_single_json(&st_all);
    assert!(st_body["runtimeId"].as_str().unwrap().starts_with("rt-"));
    assert_eq!(st_body["data"]["statuses"].as_array().unwrap().len(), 3);

    let st_one = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "agent",
            "status",
            "--function-id",
            "openvpn-connect",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert!(st_one.status.success());
    assert_eq!(
        parse_single_json(&st_one)["data"]["functionId"],
        "openvpn-connect"
    );

    let st_missing = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "agent",
            "status",
            "--function-id",
            "missing",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert_eq!(st_missing.status.code(), Some(6));
    assert_eq!(parse_single_json(&st_missing)["error"]["code"], "NOT_FOUND");

    let page1 = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "agent",
            "server",
            "list",
            "--limit",
            "2",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert!(page1.status.success());
    let p1 = parse_single_json(&page1);
    assert_eq!(p1["data"]["servers"].as_array().unwrap().len(), 2);
    assert_eq!(p1["data"]["truncated"], true);
    assert_eq!(p1["data"]["nextAfter"], "srv-b");
    assert_eq!(p1["data"]["total"], 3);

    let page2 = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "agent",
            "server",
            "list",
            "--limit",
            "2",
            "--after",
            "srv-b",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert!(page2.status.success());
    let p2 = parse_single_json(&page2);
    assert_eq!(p2["data"]["servers"][0]["id"], "srv-c");
    assert_eq!(p2["data"]["truncated"], false);

    for bad_args in [
        vec![
            "--config-dir",
            root,
            "--profile",
            "agent",
            "server",
            "list",
            "--limit",
            "0",
            "--json",
        ],
        vec![
            "--config-dir",
            root,
            "--profile",
            "agent",
            "server",
            "list",
            "--limit",
            "501",
            "--json",
        ],
        vec![
            "--config-dir",
            root,
            "--profile",
            "agent",
            "server",
            "get",
            "--json",
        ],
    ] {
        assert_eq!(
            run_with_env(&bad_args, &desktop, &[]).status.code(),
            Some(2)
        );
    }

    let got = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "agent",
            "server",
            "get",
            "--server-id",
            "srv-b",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert!(got.status.success());
    assert_eq!(parse_single_json(&got)["data"]["name"], "bravo");

    let missing_srv = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "agent",
            "server",
            "get",
            "--server-id",
            "nope",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert_eq!(missing_srv.status.code(), Some(6));

    let cfg_paths = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "agent",
            "config",
            "paths",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert!(cfg_paths.status.success());
    assert_eq!(
        parse_single_json(&cfg_paths)["data"]["appConfigPath"],
        paths.config_path.to_str().unwrap()
    );

    let settings = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "agent",
            "settings",
            "show",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert!(settings.status.success());
    let text = String::from_utf8_lossy(&settings.stdout);
    assert!(!text.contains("secret-mcp-tok-xyz"));
    assert!(!text.contains("secret-cc-tok-xyz"));
    let sbody = parse_single_json(&settings);
    assert_eq!(
        sbody["data"]["appConfig"]["openvpn"]["username"],
        "vpn-user"
    );
    assert_eq!(
        sbody["data"]["stdioPolicy"]["sensitiveActionPolicy"],
        "read-only"
    );
}

#[test]
fn desktop_alias_rejection_covers_direct_and_traversal_and_stdio() {
    let tmp = TempDir::new().unwrap();
    let desktop = tmp.path().join("desktop");
    fs::create_dir_all(&desktop).unwrap();
    let helper_dir = tmp.path().join("elsewhere");
    fs::create_dir_all(&helper_dir).unwrap();
    let traversal = helper_dir.join("..").join("desktop");

    for dir in [desktop.as_path(), traversal.as_path()] {
        let out = run_with_env(
            &["--config-dir", dir.to_str().unwrap(), "status", "--json"],
            &desktop,
            &[],
        );
        assert_eq!(out.status.code(), Some(3));
        let body = parse_single_json(&out);
        assert_eq!(body["error"]["code"], "POLICY_DENIED");
        assert_eq!(body["error"]["reason"], "desktop_profile_rejected");

        let stdio = run_with_env(
            &[
                "--config-dir",
                dir.to_str().unwrap(),
                "mcp",
                "serve",
                "--stdio",
            ],
            &desktop,
            &[],
        );
        assert_eq!(stdio.status.code(), Some(3));
        assert!(stdio.stdout.is_empty());
        assert!(!stdio.stderr.is_empty());
    }
    assert_eq!(fs::read_dir(&desktop).unwrap().count(), 0);
}

#[test]
fn concurrent_owner_lease_blocks_second_cli_until_stdio_exits_on_eof() {
    let tmp = TempDir::new().unwrap();
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");
    let root = cli_root.to_str().unwrap();

    let mut child: Child = Command::new(scpctl_bin())
        .args([
            "--config-dir",
            root,
            "--profile",
            "locktest",
            "mcp",
            "serve",
            "--stdio",
        ])
        .env("SCPCTL_DESKTOP_CONFIG_DIR", &desktop)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    writeln!(
        stdin,
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"lock-test\",\"version\":\"1\"}}}}}}"
    )
    .unwrap();
    stdin.flush().unwrap();
    let mut init_line = String::new();
    reader.read_line(&mut init_line).unwrap();
    let init_resp: Value = serde_json::from_str(&init_line).unwrap();
    assert_eq!(init_resp["id"], 1);

    let blocked_cli = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "locktest",
            "status",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert_eq!(blocked_cli.status.code(), Some(5));
    assert_eq!(
        parse_single_json(&blocked_cli)["error"]["reason"],
        "profile_owner_conflict"
    );

    let blocked_stdio = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "locktest",
            "mcp",
            "serve",
            "--stdio",
        ],
        &desktop,
        &[],
    );
    assert_eq!(blocked_stdio.status.code(), Some(5));
    assert!(blocked_stdio.stdout.is_empty());
    assert!(!blocked_stdio.stderr.is_empty());

    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(4);
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        assert!(
            Instant::now() < deadline,
            "stdio child did not exit after stdin EOF"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(status.code(), Some(0));

    let after_exit = run_with_env(
        &[
            "--config-dir",
            root,
            "--profile",
            "locktest",
            "status",
            "--json",
        ],
        &desktop,
        &[],
    );
    assert!(after_exit.status.success());
}

#[cfg(unix)]
#[test]
fn pty_attached_noninteractive_commands_never_prompt_or_hang() {
    use std::os::unix::io::FromRawFd;

    let tmp = TempDir::new().unwrap();
    let cli_root = tmp.path().join("cli_root");
    let desktop = tmp.path().join("desktop");
    seed_profile(&cli_root, &desktop, "agent");

    let mut master_fd = -1;
    let mut slave_fd = -1;
    let rc = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty failed");
    let mut master = unsafe { std::fs::File::from_raw_fd(master_fd) };
    let slave = unsafe { std::fs::File::from_raw_fd(slave_fd) };

    let mut child = Command::new(scpctl_bin())
        .args([
            "--config-dir",
            cli_root.to_str().unwrap(),
            "--profile",
            "agent",
            "status",
            "--json",
        ])
        .env("SCPCTL_DESKTOP_CONFIG_DIR", &desktop)
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(4);
    let st = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        assert!(
            Instant::now() < deadline,
            "PTY-attached noninteractive call hung"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(st.success());

    let mut buf = [0u8; 8192];
    let mut captured = Vec::new();
    loop {
        match master.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => captured.extend_from_slice(&buf[..n]),
            Err(err) if err.raw_os_error() == Some(libc::EIO) => break,
            Err(err) => panic!("pty read error: {err}"),
        }
    }
    let trimmed = String::from_utf8_lossy(&captured).trim().to_string();
    let body: Value = serde_json::from_str(&trimmed).unwrap();
    assert_eq!(body["ok"], true);
}

#[cfg(target_os = "linux")]
#[test]
fn isolated_dbus_keyring_preflight_never_hangs() {
    let tmp = TempDir::new().unwrap();
    let desktop = tmp.path().join("desktop");

    let no_bus = run_with_env(
        &["capabilities", "--no-profile", "--json"],
        &desktop,
        &[(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/scp-dbus-test",
        )],
    );
    assert!(no_bus.status.success());
    let no_bus_json = parse_single_json(&no_bus);
    assert_eq!(no_bus_json["data"]["keyring"]["state"], "unavailable");

    let has_tool = |name: &str| {
        std::env::var_os("PATH")
            .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
    };
    if has_tool("dbus-run-session") && has_tool("gnome-keyring-daemon") {
        let xdg_runtime = tmp.path().join("xdg-runtime");
        fs::create_dir_all(&xdg_runtime).unwrap();
        let mut child = Command::new("dbus-run-session")
            .args([
                "--",
                "sh",
                "-c",
                "gnome-keyring-daemon --daemonize --components=secrets >/dev/null 2>&1 || true; exec \"$1\" capabilities --no-profile --json",
                "_",
                scpctl_bin(),
            ])
            .env("SCPCTL_DESKTOP_CONFIG_DIR", &desktop)
            .env("HOME", tmp.path())
            .env("XDG_RUNTIME_DIR", &xdg_runtime)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "isolated dbus/keyring preflight hung"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        let body = parse_single_json(&out);
        let state = body["data"]["keyring"]["state"].as_str().unwrap();
        assert!(matches!(state, "ready" | "locked" | "unavailable"));
    }
}

#[test]
fn scp_cli_dependency_graph_excludes_gui_and_http_crates() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let out = Command::new("cargo")
        .args(["tree", "--locked", "-p", "scp-cli", "-e", "normal,build"])
        .current_dir(workspace_root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let forbidden = [
        "tauri",
        "tauri-build",
        "tauri-runtime",
        "wry",
        "tao",
        "webkit2gtk",
        "gtk",
        "axum",
        "hyper",
    ];
    let tree = String::from_utf8_lossy(&out.stdout);
    for line in tree.lines() {
        for crate_name in line.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_') {
            assert!(
                !forbidden.contains(&crate_name),
                "forbidden GUI/HTTP crate `{crate_name}` in scp-cli dependency tree:\n{line}"
            );
        }
    }
}
