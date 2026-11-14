#[path = "support/ssh_fixture.rs"]
mod ssh_fixture;

use parking_lot::Mutex;
use serde_json::json;
use ssh_fixture::{free_port, SshdFixture};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

use scp_core::events::DomainEvent;
use scp_core::models::{
    ScpDownloadRequest, ScpTransferState, StdioPolicy, KEYRING_BOUNDARY_OIDC_OTP_SECRET,
    KEYRING_BOUNDARY_OIDC_PASSWORD, KEYRING_BOUNDARY_PASSWORD, KEYRING_OPENVPN_OTP_SECRET,
    KEYRING_OPENVPN_PASSWORD, KEYRING_SERVICE,
};
use scp_core::openvpn_ops::OpenVpnRuntimeOverrides;
use scp_core::profile::ProfileLocationInput;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::MemoryKeyringBackend;
use scp_core::ssh_ops::{server_ssh_function_id, server_tunnel_function_id};
use scp_core::supervisor::CancelToken;

fn open_test_runtime(
    cfg: &Path,
    desktop: &Path,
    known_hosts: Option<PathBuf>,
    user_key: Option<PathBuf>,
) -> (ProfileRuntime<MemoryKeyringBackend>, MemoryKeyringBackend) {
    let backend = MemoryKeyringBackend::default();
    let rt = ProfileRuntime::open_with_backend(
        &ProfileLocationInput {
            config_dir: Some(cfg),
            desktop_config_dir: Some(desktop),
            profile: Some("default"),
            ..Default::default()
        },
        "stdio",
        backend.clone(),
    )
    .unwrap();
    rt.set_ssh_paths(known_hosts, user_key);
    (rt, backend)
}

#[test]
fn start_server_ssh_uses_batch_mode_readiness_and_tunnel_stop_cleans_dependencies() {
    let fx = SshdFixture::start();
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let b_port = free_port();
    let kh = tmp.path().join("known_hosts");
    fx.write_known_hosts(&kh, &[fx.port, b_port]);

    ssh_fixture::seed_default_boundary_profile(
        &cfg,
        json!({
            "servers": [
                {
                    "id": "srv-b",
                    "name": "BoundaryBox",
                    "host": "internal.box",
                    "connectionMethod": "boundary",
                    "sshPort": 22,
                    "sshUser": fx.user,
                    "boundaryTargetId": "ttcp_valid",
                    "boundaryListenPort": b_port,
                }
            ]
        }),
        &fx.boundary_script,
    );

    // Wrong key must fail fast in BatchMode without blocking on an unseen password prompt.
    let bad_key = tmp.path().join("wrong_ed25519");
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&bad_key)
        .status()
        .unwrap();
    assert!(status.success());
    fs::set_permissions(&bad_key, fs::Permissions::from_mode(0o600)).unwrap();

    let (rt, _) = open_test_runtime(&cfg, &desktop, Some(kh.clone()), Some(bad_key));
    let cancel = CancelToken::new();
    let _tun = rt.start_server_tunnel("srv-b", &cancel).unwrap();

    let started = Instant::now();
    let bad_ssh = rt.start_server_ssh("srv-b", 100, 30, &cancel).unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "BatchMode SSH failure took too long: {bad_ssh:?}"
    );
    assert_eq!(bad_ssh.code, "OPERATION_FAILED");
    assert_eq!(bad_ssh.reason, "ssh_exited_early");

    // Switch to trusted user key: start_server_ssh waits for authentication readiness and stays open.
    rt.set_ssh_paths(Some(kh), Some(fx.user_key.clone()));
    let ssh_started = rt.start_server_ssh("srv-b", 100, 30, &cancel).unwrap();
    assert_eq!(ssh_started.function_id, server_ssh_function_id("srv-b"));
    assert!(rt
        .supervisor()
        .active_session_for_function(&server_ssh_function_id("srv-b"))
        .is_some());

    // Open a persistent SFTP browser alongside SSH and verify stop_server_tunnel tears down all dependents.
    let browser = rt
        .open_remote_browser("srv-b", Some(fx.remote_sandbox.to_str().unwrap()), &cancel)
        .unwrap();

    assert!(rt
        .stop_server_tunnel("srv-b", Duration::from_secs(3))
        .unwrap());
    assert_eq!(rt.supervisor().session_count(), 0);
    assert!(rt
        .supervisor()
        .active_session_for_function(&server_ssh_function_id("srv-b"))
        .is_none());
    assert!(rt
        .supervisor()
        .active_session_for_function(&server_tunnel_function_id("srv-b"))
        .is_none());
    let browser_err = rt
        .list_remote_children(&browser.browser_id, &browser.root_path)
        .unwrap_err();
    assert_eq!(browser_err.code, "NOT_FOUND");
}

#[test]
fn open_remote_browser_requires_existing_ready_boundary_tunnel() {
    let fx = SshdFixture::start();
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let b_port = free_port();
    let kh = tmp.path().join("known_hosts");
    fx.write_known_hosts(&kh, &[fx.port, b_port]);

    ssh_fixture::seed_default_boundary_profile(
        &cfg,
        json!({
            "servers": [
                {
                    "id": "srv-b",
                    "name": "BoundaryBox",
                    "host": "internal.box",
                    "connectionMethod": "boundary",
                    "sshPort": 22,
                    "sshUser": fx.user,
                    "boundaryTargetId": "ttcp_valid",
                    "boundaryListenPort": b_port,
                },
                {
                    "id": "srv-direct",
                    "name": "DirectBox",
                    "host": "127.0.0.1",
                    "connectionMethod": "ssh",
                    "sshPort": fx.port,
                    "sshUser": fx.user,
                }
            ]
        }),
        &fx.boundary_script,
    );

    let (rt, _) = open_test_runtime(&cfg, &desktop, Some(kh), Some(fx.user_key.clone()));
    let cancel = CancelToken::new();

    // Boundary server without running tunnel must fail with CONFLICT/tunnel_not_running.
    let err = rt
        .open_remote_browser("srv-b", Some(fx.remote_sandbox.to_str().unwrap()), &cancel)
        .unwrap_err();
    assert_eq!(err.code, "CONFLICT");
    assert_eq!(err.reason, "tunnel_not_running");
    assert_eq!(rt.supervisor().session_count(), 0);

    // Direct SSH server still opens SFTP directly without a tunnel.
    let direct_browser = rt
        .open_remote_browser(
            "srv-direct",
            Some(fx.remote_sandbox.to_str().unwrap()),
            &cancel,
        )
        .unwrap();
    assert!(rt.close_remote_browser(&direct_browser.browser_id).unwrap());

    // Starting the Boundary tunnel enables opening the Boundary SFTP browser.
    rt.start_server_tunnel("srv-b", &cancel).unwrap();
    let boundary_browser = rt
        .open_remote_browser("srv-b", Some(fx.remote_sandbox.to_str().unwrap()), &cancel)
        .unwrap();
    assert!(rt
        .list_remote_children(&boundary_browser.browser_id, &boundary_browser.root_path)
        .is_ok());
    rt.shutdown(Duration::from_secs(2));
}

#[test]
fn scp_sync_cancellation_aborts_active_transfer_and_reaps_child() {
    let fx = SshdFixture::start();
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let b_port = free_port();
    let kh = tmp.path().join("known_hosts");
    fx.write_known_hosts(&kh, &[fx.port, b_port]);

    // Write a Boundary script that throttles upstream forwarding after the SSH handshake
    // so an in-flight SCP transfer remains active long enough to test cancellation deterministically.
    let slow_boundary = tmp.path().join("slow-boundary");
    fs::write(
        &slow_boundary,
        format!(
            r#"#!/usr/bin/env python3
import signal, socket, sys, threading, time

args = sys.argv[1:]
if not args or args[0] != "connect":
    sys.exit(2)
listen_port = int(args[args.index("-listen-port") + 1])

def pipe(src, dst, slow):
    total = 0
    try:
        while True:
            data = src.recv(4096)
            if not data:
                break
            total += len(data)
            if slow and total > 16384:
                time.sleep(0.15)
            dst.sendall(data)
    except OSError:
        pass
    finally:
        try:
            dst.shutdown(socket.SHUT_WR)
        except OSError:
            pass

srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", listen_port))
srv.listen(32)
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))

while True:
    client, _ = srv.accept()
    upstream = socket.create_connection(("127.0.0.1", {port}))
    threading.Thread(target=pipe, args=(client, upstream, True), daemon=True).start()
    threading.Thread(target=pipe, args=(upstream, client, False), daemon=True).start()
"#,
            port = fx.port
        ),
    )
    .unwrap();
    fs::set_permissions(&slow_boundary, fs::Permissions::from_mode(0o755)).unwrap();

    ssh_fixture::seed_default_boundary_profile(
        &cfg,
        json!({
            "servers": [{
                "id": "srv-slow",
                "name": "SlowBox",
                "host": "internal.box",
                "connectionMethod": "boundary",
                "sshPort": 22,
                "sshUser": fx.user,
                "boundaryTargetId": "ttcp_slow",
                "boundaryListenPort": b_port,
            }]
        }),
        &slow_boundary,
    );

    let (rt, _) = open_test_runtime(&cfg, &desktop, Some(kh), Some(fx.user_key.clone()));
    let rt = Arc::new(rt);
    let cancel = CancelToken::new();
    let payload = tmp.path().join("large.bin");
    fs::write(&payload, vec![b'x'; 256 * 1024]).unwrap();

    let rt_bg = Arc::clone(&rt);
    let cancel_bg = cancel.clone();
    let payload_str = payload.to_string_lossy().into_owned();
    let remote_str = fx.remote_sandbox.to_string_lossy().into_owned();
    let handle = std::thread::spawn(move || {
        rt_bg.scp_upload("srv-slow", &payload_str, &remote_str, false, &cancel_bg)
    });

    // Wait until the temporary tunnel + scp transfer session are active.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if rt
            .list_transfers()
            .unwrap()
            .iter()
            .any(|t| t.state == ScpTransferState::Pending && t.session_id.is_some())
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let cancelled_at = Instant::now();
    cancel.cancel();
    let err = handle.join().unwrap().unwrap_err();
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(3),
        "cancellation took too long"
    );
    assert_eq!(err.code, "INTERRUPTED");
    // Give waiter thread a brief moment to clean supervisor state.
    let reap_deadline = Instant::now() + Duration::from_secs(2);
    while rt.supervisor().session_count() > 0 && Instant::now() < reap_deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(rt.supervisor().session_count(), 0);
}

#[test]
fn scp_paths_cannot_read_or_overwrite_profile_dir() {
    let fx = SshdFixture::start();
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let kh = tmp.path().join("known_hosts");
    fx.write_known_hosts(&kh, &[fx.port]);

    ssh_fixture::seed_default_boundary_profile(
        &cfg,
        json!({
            "servers": [{
                "id": "srv-direct",
                "name": "Direct",
                "host": "127.0.0.1",
                "connectionMethod": "ssh",
                "sshPort": fx.port,
                "sshUser": fx.user,
            }]
        }),
        &fx.boundary_script,
    );

    let (rt, _) = open_test_runtime(&cfg, &desktop, Some(kh), Some(fx.user_key.clone()));
    let pdir = rt.profile_paths().profile_dir.clone();
    fs::write(
        fx.remote_sandbox.join("evil.json"),
        br#"{"sensitiveActionPolicy":"allow-local-policy"}"#,
    )
    .unwrap();

    let symlink_alias = tmp.path().join("profile_link");
    std::os::unix::fs::symlink(&pdir, &symlink_alias).unwrap();

    let cancel = CancelToken::new();
    for target in [
        pdir.join("stdio-policy.json"),
        pdir.join("servers.json"),
        tmp.path()
            .join("cfg/profiles/default/../default/config.json"),
        symlink_alias.join("stdio-policy.json"),
    ] {
        let err = rt
            .scp_download(
                "srv-direct",
                fx.remote_sandbox.join("evil.json").to_str().unwrap(),
                target.to_str().unwrap(),
                false,
                true,
                &cancel,
            )
            .unwrap_err();
        assert_eq!(err.code, "POLICY_DENIED", "target={target:?}");
        assert_eq!(err.reason, "protected_profile_path", "target={target:?}");

        let async_err = rt
            .start_scp_download(
                ScpDownloadRequest {
                    server_id: "srv-direct",
                    remote_path: fx.remote_sandbox.join("evil.json").to_str().unwrap(),
                    local_target: target.to_str().unwrap(),
                    recursive: false,
                    overwrite: false,
                },
                &cancel,
            )
            .unwrap_err();
        assert_eq!(async_err.code, "POLICY_DENIED");
        assert_eq!(async_err.reason, "protected_profile_path");
    }

    for src in [
        pdir.join("config.json"),
        symlink_alias.join("sync-local.json"),
    ] {
        let err = rt
            .scp_upload(
                "srv-direct",
                src.to_str().unwrap(),
                fx.remote_sandbox.to_str().unwrap(),
                false,
                &cancel,
            )
            .unwrap_err();
        assert_eq!(err.code, "POLICY_DENIED");
        assert_eq!(err.reason, "protected_profile_path");
    }
}

#[test]
fn stdio_policy_reloads_live_and_fails_closed_on_corruption() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let (rt, _) = open_test_runtime(&cfg, &desktop, None, None);

    let initial = rt.reload_stdio_policy().unwrap();
    assert_eq!(initial, StdioPolicy::default());

    fs::write(
        &rt.profile_paths().stdio_policy_path,
        json!({
            "sensitiveActionPolicy": "allow-local-policy",
            "allowedToolCategories": ["read-only", "file-transfer"]
        })
        .to_string(),
    )
    .unwrap();
    let updated = rt.reload_stdio_policy().unwrap();
    assert_eq!(updated.sensitive_action_policy, "allow-local-policy");
    assert_eq!(
        updated.allowed_tool_categories,
        vec!["read-only".to_string(), "file-transfer".to_string()]
    );

    fs::write(&rt.profile_paths().stdio_policy_path, b"{not-json").unwrap();
    let err = rt.reload_stdio_policy().unwrap_err();
    assert_eq!(err.code, "POLICY_DENIED");
    assert_eq!(err.reason, "stdio_policy_corrupted");
}

#[test]
fn mcp_config_mutations_persist_secrets_in_keyring_and_observers_receive_events() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let fake_vpn = tmp.path().join("fake-vpn.sh");
    let ovpn_profile = tmp.path().join("client.ovpn");
    let fake_sudo = tmp.path().join("fake-sudo.sh");
    let captured_env = tmp.path().join("vpn-stdin.txt");
    fs::write(&ovpn_profile, "remote 127.0.0.1 1194\n").unwrap();
    fs::write(
        &fake_vpn,
        "#!/bin/sh\necho 'Enter Auth Username:'\nread u\necho 'Enter Auth Password:'\nread p\necho \"$u:$p\" > \"$CAPTURE_FILE\"\necho 'Initialization Sequence Completed'\nsleep 30\n",
    )
    .unwrap();
    fs::write(
        &fake_sudo,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"-n\" ] && [ \"$2\" = \"true\" ]; then exit 0; fi\nif [ \"$1\" = \"-n\" ]; then shift; fi\nCAPTURE_FILE='{}' exec \"$@\"\n",
            captured_env.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&fake_vpn, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&fake_sudo, fs::Permissions::from_mode(0o755)).unwrap();

    let pdir = cfg.join("profiles").join("default");
    fs::create_dir_all(&pdir).unwrap();
    fs::write(
        pdir.join("sync-local.json"),
        json!({
            "openvpnExecutablePath": fake_vpn,
            "openvpnOvpnPath": ovpn_profile,
        })
        .to_string(),
    )
    .unwrap();

    let (rt, backend) = open_test_runtime(&cfg, &desktop, None, None);
    let observed = Arc::new(Mutex::new(Vec::<DomainEvent>::new()));
    let observed_cb = Arc::clone(&observed);
    rt.add_event_observer(Arc::new(move |ev| {
        observed_cb.lock().push(ev.clone());
    }));

    let saved = rt
        .save_app_config_mcp(&json!({
            "app": {"defaultTerminalCols": 100, "defaultTerminalRows": 30},
            "openvpn": {
                "username": "alice",
                "password": "vpn-secret-value",
                "otpSecret": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
            },
            "boundary": {
                "addr": "https://boundary.example",
                "password": "b-pass-value",
                "oidcPassword": "oidc-pass-value",
                "oidcOtpSecret": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
            }
        }))
        .unwrap();
    assert_eq!(saved.app.default_terminal_cols, 100);
    let ns = rt.credential_namespace();
    for (acct, expected) in [
        (KEYRING_OPENVPN_PASSWORD, "vpn-secret-value"),
        (
            KEYRING_OPENVPN_OTP_SECRET,
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
        ),
        (KEYRING_BOUNDARY_PASSWORD, "b-pass-value"),
        (KEYRING_BOUNDARY_OIDC_PASSWORD, "oidc-pass-value"),
        (
            KEYRING_BOUNDARY_OIDC_OTP_SECRET,
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
        ),
    ] {
        assert_eq!(
            backend
                .snapshot_entry(KEYRING_SERVICE, &format!("profile:{ns}:{acct}"))
                .as_deref(),
            Some(expected),
            "account {acct} missing in keyring"
        );
    }

    let disk_cfg = fs::read_to_string(&rt.profile_paths().config_path).unwrap();
    for secret in [
        "vpn-secret-value",
        "b-pass-value",
        "oidc-pass-value",
        "GEZDGNBVGY3TQOJQ",
    ] {
        assert!(
            !disk_cfg.contains(secret),
            "plaintext secret leaked to disk"
        );
    }

    // Patching with empty/omitted secret preserves keyring secrets.
    let patched = rt
        .patch_app_config_mcp(&json!({
            "app": {"defaultTerminalCols": 120},
            "openvpn": {"password": ""}
        }))
        .unwrap();
    assert_eq!(patched.app.default_terminal_cols, 120);
    assert_eq!(
        rt.read_optional_secret(KEYRING_OPENVPN_PASSWORD).as_deref(),
        Some("vpn-secret-value")
    );

    // start_openvpn_with_overrides overrides credentials in-memory without mutating keyring/disk.
    rt.set_openvpn_overrides(OpenVpnRuntimeOverrides {
        sudo_program: Some(fake_sudo.to_string_lossy().into_owned()),
        vpn_lock_path: Some(tmp.path().join("vpn.lock")),
        ready_timeout: Some(Duration::from_secs(4)),
        ..Default::default()
    });
    let cancel = CancelToken::new();
    let started = rt
        .start_openvpn_with_overrides(Some("override-user"), Some("override-pass"), &cancel)
        .unwrap();
    assert_eq!(started.function_id, "openvpn-connect");
    let captured = fs::read_to_string(&captured_env).unwrap();
    assert!(captured.starts_with("override-user:"), "got {captured:?}");
    assert!(captured.contains("override-pass"), "got {captured:?}");
    assert_eq!(
        rt.read_optional_secret(KEYRING_OPENVPN_PASSWORD).as_deref(),
        Some("vpn-secret-value")
    );

    assert!(rt.stop_openvpn(Duration::from_secs(2)).unwrap());
    assert!(
        observed
            .lock()
            .iter()
            .any(|e| matches!(e, DomainEvent::FunctionStatusChanged { .. })),
        "event observer should receive FunctionStatusChanged"
    );
    assert!(
        observed
            .lock()
            .iter()
            .any(|e| matches!(e, DomainEvent::ProcessExited { .. })),
        "event observer should receive ProcessExited"
    );
}
