#[path = "support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use scp_core::models::{StdioPolicy, KEYRING_OPENVPN_PASSWORD, KEYRING_SERVICE};
use scp_core::openvpn_ops::OpenVpnRuntimeOverrides;
use scp_core::profile::ProfileLocationInput;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::{KeyringHealth, MemoryKeyringBackend};
use scp_core::supervisor::CancelToken;
use scp_mcp::contracts::{
    authorize_standalone, is_standalone_stdio_available, operation, standalone_unavailable_reason,
    Capability, ErrorCode, OperationError, OPERATIONS,
};
use scp_mcp::standalone::{
    capabilities_report, execute_standalone, execute_standalone_cancellable,
    permitted_standalone_operations, standalone_schema_catalog,
};
use scp_mcp::validation::{conforms, output_schema};

fn setup_profile(
    stdio_policy: Option<Value>,
) -> (
    TempDir,
    ProfileRuntime<MemoryKeyringBackend>,
    MemoryKeyringBackend,
) {
    support::open_seeded_stdio_runtime("127.0.0.1", KeyringHealth::Ready, None, stdio_policy)
}

fn permissive_policy() -> Value {
    json!({
        "sensitiveActionPolicy": "allow-local-policy",
        "allowedToolCategories": [
            "read-only",
            "runtime-control",
            "config-mutation",
            "file-transfer"
        ]
    })
}

#[test]
fn slice4_45_operation_authorization_matrix_and_live_policy_reload() {
    let default_policy = StdioPolicy::default();
    let default_ops = permitted_standalone_operations(&default_policy);
    assert_eq!(default_ops.len(), 11);
    for expected in [
        "app_get_overview",
        "function_status_list",
        "function_status_get",
        "openvpn_get_status",
        "boundary_get_status",
        "servers_list",
        "server_get",
        "config_get_paths",
        "sync_settings_get",
        "boundary_search_targets",
        "detect_openvpn_executable",
    ] {
        assert!(
            authorize_standalone(expected, &default_policy).is_ok(),
            "missing default read-only op {expected}"
        );
    }

    let read_plus_transfer = StdioPolicy {
        sensitive_action_policy: "read-only".into(),
        allowed_tool_categories: vec!["read-only".into(), "file-transfer".into()],
    };
    assert_eq!(
        permitted_standalone_operations(&read_plus_transfer).len(),
        15
    );
    for op_name in [
        "scp_transfer_list",
        "scp_transfer_get",
        "remote_browser_list_children",
        "remote_browser_stat_path",
    ] {
        assert!(
            authorize_standalone(op_name, &read_plus_transfer).is_ok(),
            "expected file-transfer query {op_name} to be permitted"
        );
    }

    let full_policy: StdioPolicy = serde_json::from_value(permissive_policy()).unwrap();
    assert_eq!(permitted_standalone_operations(&full_policy).len(), 41);

    let mut available_count = 0usize;
    let mut unavailable_count = 0usize;
    for op in OPERATIONS {
        let expect_available = matches!(op.capability, Capability::Runtime);
        assert_eq!(
            is_standalone_stdio_available(op),
            expect_available,
            "{}",
            op.name
        );
        if expect_available {
            available_count += 1;
            assert_eq!(standalone_unavailable_reason(op), None, "{}", op.name);
            assert!(
                authorize_standalone(op.name, &full_policy).is_ok(),
                "{}",
                op.name
            );
        } else {
            unavailable_count += 1;
            let err = authorize_standalone(op.name, &full_policy).unwrap_err();
            assert_eq!(err.code, ErrorCode::CapabilityUnavailable, "{}", op.name);
            let reason = standalone_unavailable_reason(op).expect("unavailable reason");
            match op.name {
                "server_rdp_start" => assert_eq!(reason, "interactive_host_required"),
                "config_sync_pull" => assert_eq!(reason, "protected_import"),
                "config_git_pull" | "config_git_push" => assert_eq!(reason, "reviewed_git"),
                other => panic!("unexpected unavailable op {other}"),
            }
        }
    }
    assert_eq!(available_count, 41);
    assert_eq!(unavailable_count, 4);

    let catalog = standalone_schema_catalog(None).unwrap();
    let ops = catalog["mcpOperations"].as_array().unwrap();
    assert_eq!(ops.iter().filter(|o| o["available"] == true).count(), 41);
    assert_eq!(ops.iter().filter(|o| o["available"] == false).count(), 4);

    // Live reload without reopening runtime + fail closed on corruption.
    let (_tmp, rt, _backend) = setup_profile(None);
    assert_eq!(
        execute_standalone(
            &rt,
            "1.3.3",
            "server_upsert",
            &json!({"server": {"id":"srv-2","name":"beta","host":"10.0.0.2","connectionMethod":"ssh","sshPort":22,"sshUser":"ops"}}),
        )
        .unwrap_err()
        .code,
        ErrorCode::PolicyDenied
    );
    fs::write(
        &rt.profile_paths().stdio_policy_path,
        permissive_policy().to_string(),
    )
    .unwrap();
    let updated = execute_standalone(
        &rt,
        "1.3.3",
        "server_upsert",
        &json!({"server": {"id":"srv-2","name":"beta","host":"10.0.0.2","connectionMethod":"ssh","sshPort":22,"sshUser":"ops"}}),
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("server_upsert").unwrap()),
        &updated
    ));

    fs::write(&rt.profile_paths().stdio_policy_path, b"{corrupt").unwrap();
    let fail_closed = execute_standalone(&rt, "1.3.3", "servers_list", &json!({})).unwrap_err();
    assert_eq!(fail_closed.code, ErrorCode::PolicyDenied);

    let caps = capabilities_report(Some(&rt)).unwrap();
    assert!(caps["unavailableFamilies"].get("sftp").is_none());
}

#[test]
fn host_neutral_errors_never_reference_desktop_in_standalone_mode() {
    for code in [
        ErrorCode::PolicyDenied,
        ErrorCode::InvalidArguments,
        ErrorCode::CapabilityUnavailable,
        ErrorCode::NotFound,
        ErrorCode::Conflict,
        ErrorCode::Cancelled,
        ErrorCode::Timeout,
        ErrorCode::OperationFailed,
    ] {
        let standalone = OperationError::for_host(code, true);
        assert!(
            !standalone.message.to_lowercase().contains("desktop"),
            "standalone error for {code:?} leaked desktop guidance: {}",
            standalone.message
        );
    }
    let desktop_fail = OperationError::for_host(ErrorCode::OperationFailed, false);
    assert!(desktop_fail.message.contains("desktop"));
}

#[cfg(unix)]
#[test]
fn standalone_config_keyring_boundary_caddy_vpn_and_sync_push_preserve_contracts() {
    use std::os::unix::fs::PermissionsExt;

    let (tmp, rt, backend) = setup_profile(Some(permissive_policy()));
    let fake_boundary = tmp.path().join("fake-boundary");
    let fake_caddy = tmp.path().join("fake-caddy");
    let fake_vpn = tmp.path().join("fake-openvpn");
    let fake_sudo = tmp.path().join("fake-sudo");
    let ovpn_file = tmp.path().join("client.ovpn");
    let caddy_dir = tmp.path().join("caddy-site");
    let repo_dir = tmp.path().join("git-repo");
    fs::create_dir_all(&caddy_dir).unwrap();
    fs::create_dir_all(&repo_dir).unwrap();
    fs::write(caddy_dir.join("Caddyfile"), ":0 {\n}\n").unwrap();
    fs::write(&ovpn_file, "remote 127.0.0.1 1194\n").unwrap();

    fs::write(
        &fake_boundary,
        r#"#!/usr/bin/env python3
import json, signal, sys, time
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
args = sys.argv[1:]
if args[:2] == ["targets", "list"]:
    print(json.dumps({"items": [{"id": "ttcp_1", "name": "Prod-Box", "type": "tcp"}]}))
    sys.exit(0)
if args[:2] == ["authenticate", "password"]:
    _pw = sys.stdin.read()
    print(json.dumps({"item": {"attributes": {"token": "tbat_standalone_tok"}}}))
    sys.exit(0)
time.sleep(30)
"#,
    )
    .unwrap();
    fs::write(
        &fake_caddy,
        r#"#!/usr/bin/env python3
import signal, socket, sys, time
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
s = socket.socket()
s.bind(("127.0.0.1", 0))
s.listen(1)
print("serving initial configuration", flush=True)
time.sleep(30)
"#,
    )
    .unwrap();
    fs::write(
        &fake_vpn,
        "#!/bin/sh\necho 'Enter Auth Username:'\nread u\necho 'Enter Auth Password:'\nread p\necho 'Initialization Sequence Completed'\nsleep 30\n",
    )
    .unwrap();
    fs::write(
        &fake_sudo,
        "#!/bin/sh\nif [ \"$1\" = \"-n\" ] && [ \"$2\" = \"true\" ]; then exit 0; fi\nif [ \"$1\" = \"-n\" ]; then shift; fi\nexec \"$@\"\n",
    )
    .unwrap();
    for p in [&fake_boundary, &fake_caddy, &fake_vpn, &fake_sudo] {
        fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fs::write(
        &rt.profile_paths().sync_local_path,
        json!({
            "boundaryCliPath": fake_boundary,
            "openvpnExecutablePath": fake_vpn,
            "openvpnOvpnPath": ovpn_file,
            "caddyFolder": caddy_dir,
        })
        .to_string(),
    )
    .unwrap();
    drop(rt);
    let p_input = ProfileLocationInput {
        config_dir: Some(&tmp.path().join("cli_root")),
        desktop_config_dir: Some(&tmp.path().join("desktop")),
        profile: Some("agent"),
        ..Default::default()
    };
    let rt = ProfileRuntime::open_with_backend(&p_input, "stdio", backend.clone()).unwrap();
    rt.set_caddy_executable(Some(fake_caddy.to_string_lossy().into_owned()));
    rt.set_openvpn_overrides(OpenVpnRuntimeOverrides {
        sudo_program: Some(fake_sudo.to_string_lossy().into_owned()),
        vpn_lock_path: Some(tmp.path().join("vpn.lock")),
        locator_program: Some("false".into()),
        candidate_paths: Some(vec![fake_vpn.to_string_lossy().into_owned()]),
        ready_timeout: Some(std::time::Duration::from_secs(4)),
    });

    // Launch path protection on config_save
    let tamper_err = execute_standalone(
        &rt,
        "1.3.3",
        "config_save",
        &json!({
            "config": {
                "app": {"defaultTerminalCols": 100, "defaultTerminalRows": 30},
                "openvpn": {"executablePath": "/evil/openvpn"},
                "boundary": {"cliPath": fake_boundary}
            }
        }),
    )
    .unwrap_err();
    assert_eq!(tamper_err.code, ErrorCode::PolicyDenied);
    assert!(!tamper_err.message.to_lowercase().contains("desktop"));

    // Valid config_save retaining local launch paths + saving secrets into keyring
    let saved_cfg = execute_standalone(
        &rt,
        "1.3.3",
        "config_save",
        &json!({
            "config": {
                "app": {"defaultTerminalCols": 100, "defaultTerminalRows": 30},
                "openvpn": {
                    "command": "",
                    "executablePath": fake_vpn,
                    "ovpnPath": ovpn_file,
                    "extraArgs": "",
                    "username": "alice",
                    "password": "vpn-mcp-secret",
                },
                "boundary": {
                    "addr": "https://boundary.example",
                    "authMethodId": "ampw_1",
                    "loginName": "alice",
                    "authKind": "password",
                    "password": "b-pw-secret",
                    "cliPath": fake_boundary,
                    "connectSshOptions": ""
                }
            }
        }),
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("config_save").unwrap()),
        &saved_cfg
    ));
    let ns = rt.credential_namespace();
    assert_eq!(
        backend
            .snapshot_entry(
                KEYRING_SERVICE,
                &format!("profile:{ns}:{KEYRING_OPENVPN_PASSWORD}")
            )
            .as_deref(),
        Some("vpn-mcp-secret")
    );
    let disk_cfg = fs::read_to_string(&rt.profile_paths().config_path).unwrap();
    assert!(!disk_cfg.contains("vpn-mcp-secret"));
    assert!(!disk_cfg.contains("b-pw-secret"));

    let patched_cfg = execute_standalone(
        &rt,
        "1.3.3",
        "config_save_partial",
        &json!({"patch": {"app": {"defaultTerminalCols": 120}}}),
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("config_save_partial").unwrap()),
        &patched_cfg
    ));

    let patched_sync = execute_standalone(
        &rt,
        "1.3.3",
        "sync_settings_save_partial",
        &json!({"patch": {"repoRoot": repo_dir}}),
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("sync_settings_save_partial").unwrap()),
        &patched_sync
    ));
    assert_eq!(
        patched_sync["repoRoot"],
        repo_dir.to_string_lossy().as_ref()
    );

    // Push config sync never copies stdio-policy.json or sync-local.json
    let pushed = execute_standalone(
        &rt,
        "1.3.3",
        "config_sync_push",
        &json!({"repoRoot": repo_dir}),
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("config_sync_push").unwrap()),
        &pushed
    ));
    let app_cfg_repo = repo_dir.join("app-config");
    assert!(app_cfg_repo.join("config.json").is_file());
    assert!(app_cfg_repo.join("servers.json").is_file());
    assert!(!app_cfg_repo.join("stdio-policy.json").exists());
    assert!(!app_cfg_repo.join("sync-local.json").exists());

    // Boundary search targets + password auth + OpenVPN + Caddy + server credentials + delete
    let targets = execute_standalone(&rt, "1.3.3", "boundary_search_targets", &json!({})).unwrap();
    assert!(conforms(
        &output_schema(operation("boundary_search_targets").unwrap()),
        &targets
    ));

    let detected_vpn =
        execute_standalone(&rt, "1.3.3", "detect_openvpn_executable", &json!({})).unwrap();
    assert!(conforms(
        &output_schema(operation("detect_openvpn_executable").unwrap()),
        &detected_vpn
    ));

    let cancel = CancelToken::new();
    for (name, args) in [
        ("boundary_auth_start", json!({})),
        ("caddy_start", json!({"folder": caddy_dir})),
        ("openvpn_start", json!({})),
        (
            "server_ssh_password_set",
            json!({"serverId": "srv-1", "password": "ssh-pw-secret"}),
        ),
        ("server_ssh_password_clear", json!({"serverId": "srv-1"})),
        ("openvpn_stop", json!({})),
        ("caddy_stop", json!({})),
        ("boundary_auth_stop", json!({})),
        ("server_delete", json!({"id": "srv-1"})),
    ] {
        let out = execute_standalone_cancellable(&rt, "1.3.3", name, &args, &cancel).unwrap();
        assert!(
            conforms(&output_schema(operation(name).unwrap()), &out),
            "schema mismatch for {name}: {out}"
        );
    }
    rt.shutdown(std::time::Duration::from_secs(2));
}

#[cfg(unix)]
fn find_sshd_fixture_example() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors() {
            let Some(name) = ancestor.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if matches!(name, "debug" | "release") {
                let candidate = ancestor.join("examples").join("sshd_fixture_daemon");
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug/examples/sshd_fixture_daemon")
}

#[cfg(unix)]
#[test]
fn standalone_ssh_scp_sftp_tools_conform_to_schemas_and_preserve_safety() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let example = find_sshd_fixture_example();
    assert!(
        example.is_file(),
        "build sshd_fixture_daemon before running this test: {example:?}"
    );
    let mut daemon = Command::new(&example)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout_line = String::new();
    BufReader::new(daemon.stdout.take().unwrap())
        .read_line(&mut stdout_line)
        .unwrap();
    let fx: Value = serde_json::from_str(&stdout_line).unwrap();

    let (tmp, rt, _backend) = setup_profile(Some(permissive_policy()));
    rt.set_ssh_paths(
        Some(PathBuf::from(fx["knownHosts"].as_str().unwrap())),
        Some(PathBuf::from(fx["userKey"].as_str().unwrap())),
    );
    execute_standalone(
        &rt,
        "1.3.3",
        "server_upsert",
        &json!({
            "server": {
                "id": "srv-fx",
                "name": "FixtureSSH",
                "host": "127.0.0.1",
                "os": "linux",
                "connectionMethod": "ssh",
                "sshPort": fx["port"],
                "sshUser": fx["user"]
            }
        }),
    )
    .unwrap();

    let cancel = CancelToken::new();
    let ssh_start = execute_standalone_cancellable(
        &rt,
        "1.3.3",
        "server_ssh_start",
        &json!({"serverId": "srv-fx", "cols": 100, "rows": 30}),
        &cancel,
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("server_ssh_start").unwrap()),
        &ssh_start
    ));

    let browser = execute_standalone_cancellable(
        &rt,
        "1.3.3",
        "remote_browser_open",
        &json!({"serverId": "srv-fx"}),
        &cancel,
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("remote_browser_open").unwrap()),
        &browser
    ));
    let bid = browser["browserId"].as_str().unwrap();
    let sandbox_remote = fx["remoteSandbox"].as_str().unwrap();
    let new_dir = format!("{sandbox_remote}/s4-dir");

    for (name, args) in [
        (
            "remote_browser_create_directory",
            json!({"browserId": bid, "path": new_dir}),
        ),
        (
            "remote_browser_list_children",
            json!({"browserId": bid, "path": sandbox_remote}),
        ),
        (
            "remote_browser_stat_path",
            json!({"browserId": bid, "path": new_dir}),
        ),
        (
            "remote_browser_delete_path",
            json!({"browserId": bid, "path": new_dir, "recursive": true}),
        ),
        ("remote_browser_close", json!({"browserId": bid})),
    ] {
        let out = execute_standalone_cancellable(&rt, "1.3.3", name, &args, &cancel).unwrap();
        assert!(
            conforms(&output_schema(operation(name).unwrap()), &out),
            "schema mismatch {name}: {out}"
        );
    }

    let up_file = tmp.path().join("payload.txt");
    fs::write(&up_file, b"slice4-scp-payload").unwrap();
    let uploaded = execute_standalone_cancellable(
        &rt,
        "1.3.3",
        "scp_upload",
        &json!({"serverId": "srv-fx", "localPath": up_file, "remotePath": sandbox_remote}),
        &cancel,
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("scp_upload").unwrap()),
        &uploaded
    ));
    assert!(!uploaded["result"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("desktop"));

    let dl_file = tmp.path().join("downloaded.txt");
    let downloaded = execute_standalone_cancellable(
        &rt,
        "1.3.3",
        "scp_download",
        &json!({
            "serverId": "srv-fx",
            "remotePath": format!("{sandbox_remote}/payload.txt"),
            "localPath": dl_file
        }),
        &cancel,
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("scp_download").unwrap()),
        &downloaded
    ));
    assert_eq!(fs::read(&dl_file).unwrap(), b"slice4-scp-payload");

    let async_up = execute_standalone_cancellable(
        &rt,
        "1.3.3",
        "scp_start_upload",
        &json!({"serverId": "srv-fx", "localPath": up_file, "remotePath": sandbox_remote}),
        &cancel,
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("scp_start_upload").unwrap()),
        &async_up
    ));

    let stop_ssh = execute_standalone_cancellable(
        &rt,
        "1.3.3",
        "server_ssh_stop",
        &json!({"serverId": "srv-fx"}),
        &cancel,
    )
    .unwrap();
    assert!(conforms(
        &output_schema(operation("server_ssh_stop").unwrap()),
        &stop_ssh
    ));

    rt.shutdown(std::time::Duration::from_secs(2));
    drop(daemon.stdin.take());
    let _ = daemon.wait();
}

#[cfg(all(feature = "stdio", unix))]
#[test]
fn stdio_host_invalidates_subscriptions_on_events_and_shuts_down_runtime_on_eof() {
    use scp_mcp::stdio::serve_stdio_io;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (tmp, rt, _backend) = setup_profile(Some(permissive_policy()));
    let caddy_dir = tmp.path().join("caddy-site");
    let fake_caddy = tmp.path().join("fake-caddy-stdio");
    fs::create_dir_all(&caddy_dir).unwrap();
    fs::write(caddy_dir.join("Caddyfile"), ":0 {\n}\n").unwrap();
    fs::write(
        &fake_caddy,
        "#!/usr/bin/env python3\nimport signal, sys, time\nsignal.signal(signal.SIGTERM, lambda *_: sys.exit(0))\nprint('serving initial configuration', flush=True)\ntime.sleep(30)\n",
    )
    .unwrap();
    fs::set_permissions(&fake_caddy, fs::Permissions::from_mode(0o755)).unwrap();
    rt.set_caddy_executable(Some(fake_caddy.to_string_lossy().into_owned()));

    let rt = Arc::new(rt);
    let rt_check = Arc::clone(&rt);
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    tokio_rt.block_on(async move {
        let (client_in, server_in) = tokio::io::duplex(16384);
        let (server_out, client_out) = tokio::io::duplex(16384);
        let server_task = tokio::spawn(async move {
            serve_stdio_io(rt, "1.3.3", server_in, server_out).await
        });

        let mut writer = client_in;
        let mut reader = BufReader::new(client_out).lines();

        writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "initialize",
                        "params": {
                            "protocolVersion": "2025-11-25",
                            "capabilities": {},
                            "clientInfo": {"name": "s4-rust", "version": "1"}
                        }
                    })
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let _ = reader.next_line().await.unwrap().unwrap();
        writer
            .write_all(
                format!(
                    "{}\n{}\n",
                    json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                    json!({"jsonrpc": "2.0", "id": 2, "method": "resources/subscribe", "params": {"uri": "scp://status/functions"}})
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let mut got_sub_ack = false;
        let mut got_initial_update = false;
        while !(got_sub_ack && got_initial_update) {
            let msg: Value =
                serde_json::from_str(&reader.next_line().await.unwrap().unwrap()).unwrap();
            if msg["id"] == 2 {
                got_sub_ack = true;
            } else if msg["method"] == "notifications/resources/updated" {
                got_initial_update = true;
            }
        }

        // Trigger FunctionStatusChanged by starting caddy without stopping it before EOF.
        writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "tools/call",
                        "params": {"name": "caddy_start", "arguments": {"folder": caddy_dir}}
                    })
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let mut saw_call_ok = false;
        let mut saw_invalidation = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
        while !(saw_call_ok && saw_invalidation) {
            let line = tokio::time::timeout_at(deadline, reader.next_line())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let msg: Value = serde_json::from_str(&line).unwrap();
            if msg["id"] == 3 {
                assert_eq!(msg["result"]["isError"], false);
                saw_call_ok = true;
            } else if msg["method"] == "notifications/resources/updated"
                && msg["params"]["uri"] == "scp://status/functions"
            {
                saw_invalidation = true;
            }
        }
        assert_eq!(rt_check.supervisor().session_count(), 1);

        // Dropping client_in triggers EOF; serve_stdio_io must shut down runtime and reap Caddy.
        drop(writer);
        let code = server_task.await.unwrap();
        assert_eq!(code, 0);
        assert_eq!(rt_check.supervisor().session_count(), 0);
    });
}
