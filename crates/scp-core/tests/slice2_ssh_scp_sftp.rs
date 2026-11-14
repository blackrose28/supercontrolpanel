#[path = "support/ssh_fixture.rs"]
mod ssh_fixture;

use ssh_fixture::{free_port, SshdFixture};
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tempfile::TempDir;

use scp_core::events::{BufferedEventSink, DomainEvent};
use scp_core::models::{RemotePathKind, ScpTransferState};
use scp_core::profile::ProfileLocationInput;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::MemoryKeyringBackend;
use scp_core::ssh_ops::server_tunnel_function_id;
use scp_core::supervisor::CancelToken;

#[test]
fn untrusted_and_mismatched_host_keys_rejected_strictly_in_scp_and_sftp() {
    let fx = SshdFixture::start();
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");

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

    let rt = ProfileRuntime::open_with_backend(
        &ProfileLocationInput {
            config_dir: Some(&cfg),
            desktop_config_dir: Some(&desktop),
            profile: Some("default"),
            ..Default::default()
        },
        "test",
        MemoryKeyringBackend::default(),
    )
    .unwrap();

    // Missing known_hosts -> host_key_untrusted in both SCP and SFTP.
    let missing_kh = tmp.path().join("missing_known_hosts");
    rt.set_ssh_paths(Some(missing_kh), Some(fx.user_key.clone()));

    let local_file = tmp.path().join("sample.txt");
    fs::write(&local_file, b"payload").unwrap();
    let cancel = CancelToken::new();

    let err_scp = rt
        .scp_upload(
            "srv-direct",
            local_file.to_str().unwrap(),
            fx.remote_sandbox.to_str().unwrap(),
            false,
            &cancel,
        )
        .unwrap_err();
    assert_eq!(err_scp.code, "CAPABILITY_UNAVAILABLE");
    assert_eq!(err_scp.reason, "host_key_untrusted");
    assert_eq!(err_scp.next_action, "verify_host_key");

    let err_sftp = rt
        .open_remote_browser(
            "srv-direct",
            Some(fx.remote_sandbox.to_str().unwrap()),
            &cancel,
        )
        .unwrap_err();
    assert_eq!(err_sftp.code, "CAPABILITY_UNAVAILABLE");
    assert_eq!(err_sftp.reason, "host_key_untrusted");
    assert_eq!(err_sftp.next_action, "verify_host_key");

    // Mismatched host key in known_hosts -> also rejected.
    let bad_kh = tmp.path().join("bad_known_hosts");
    fs::write(
        &bad_kh,
        format!("[127.0.0.1]:{} ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIInvalidKeyForTestingOnly000000000000000\n", fx.port),
    )
    .unwrap();
    rt.set_ssh_paths(Some(bad_kh), Some(fx.user_key.clone()));

    let err_mismatch = rt
        .open_remote_browser(
            "srv-direct",
            Some(fx.remote_sandbox.to_str().unwrap()),
            &cancel,
        )
        .unwrap_err();
    assert_eq!(err_mismatch.code, "CAPABILITY_UNAVAILABLE");
    assert_eq!(err_mismatch.reason, "host_key_untrusted");
}

#[test]
fn direct_ssh_scp_upload_download_recursive_and_overwrite_policy() {
    let fx = SshdFixture::start();
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");

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

    let rt = ProfileRuntime::open_with_backend(
        &ProfileLocationInput {
            config_dir: Some(&cfg),
            desktop_config_dir: Some(&desktop),
            profile: Some("default"),
            ..Default::default()
        },
        "test",
        MemoryKeyringBackend::default(),
    )
    .unwrap();
    rt.set_ssh_paths(Some(fx.known_hosts.clone()), Some(fx.user_key.clone()));

    let cancel = CancelToken::new();
    let local_file = tmp.path().join("upload.bin");
    fs::write(&local_file, b"direct-scp-content-123").unwrap();

    let up_entry = rt
        .scp_upload(
            "srv-direct",
            local_file.to_str().unwrap(),
            fx.remote_sandbox.to_str().unwrap(),
            false,
            &cancel,
        )
        .unwrap();
    assert_eq!(up_entry.state, ScpTransferState::Success);
    assert_eq!(
        fs::read(fx.remote_sandbox.join("upload.bin")).unwrap(),
        b"direct-scp-content-123"
    );

    // Recursive directory upload
    let local_dir = tmp.path().join("bundle");
    fs::create_dir_all(local_dir.join("nested")).unwrap();
    fs::write(local_dir.join("nested/item.txt"), b"nested-data").unwrap();
    let up_dir = rt
        .scp_upload(
            "srv-direct",
            local_dir.to_str().unwrap(),
            fx.remote_sandbox.to_str().unwrap(),
            true,
            &cancel,
        )
        .unwrap();
    assert_eq!(up_dir.state, ScpTransferState::Success);
    assert_eq!(
        fs::read(fx.remote_sandbox.join("bundle/nested/item.txt")).unwrap(),
        b"nested-data"
    );

    // Download to fresh path
    let dl_file = tmp.path().join("downloaded.bin");
    let dl_entry = rt
        .scp_download(
            "srv-direct",
            fx.remote_sandbox.join("upload.bin").to_str().unwrap(),
            dl_file.to_str().unwrap(),
            false,
            false,
            &cancel,
        )
        .unwrap();
    assert_eq!(dl_entry.state, ScpTransferState::Success);
    assert_eq!(fs::read(&dl_file).unwrap(), b"direct-scp-content-123");

    // Existing destination without overwrite must fail with CONFLICT
    let conflict = rt
        .scp_download(
            "srv-direct",
            fx.remote_sandbox.join("upload.bin").to_str().unwrap(),
            dl_file.to_str().unwrap(),
            false,
            false,
            &cancel,
        )
        .unwrap_err();
    assert_eq!(conflict.code, "CONFLICT");
    assert_eq!(conflict.reason, "destination_exists");

    // Overwrite: true succeeds
    fs::write(fx.remote_sandbox.join("upload.bin"), b"updated").unwrap();
    rt.scp_download(
        "srv-direct",
        fx.remote_sandbox.join("upload.bin").to_str().unwrap(),
        dl_file.to_str().unwrap(),
        false,
        true,
        &cancel,
    )
    .unwrap();
    assert_eq!(fs::read(&dl_file).unwrap(), b"updated");
    assert_eq!(rt.list_transfers().unwrap().len(), 4);
}

#[test]
fn persistent_sftp_browser_crud_weird_names_and_tunnel_stop_shutdown_cleanup() {
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
            "servers": [{
                "id": "srv-b",
                "name": "BoundaryBox",
                "host": "internal.box",
                "connectionMethod": "boundary",
                "sshPort": 22,
                "sshUser": fx.user,
                "boundaryTargetId": "ttcp_valid",
                "boundaryListenPort": b_port,
            }]
        }),
        &fx.boundary_script,
    );

    let sink = Arc::new(BufferedEventSink::new());
    let rt = ProfileRuntime::open_with_backend_and_sink(
        &ProfileLocationInput {
            config_dir: Some(&cfg),
            desktop_config_dir: Some(&desktop),
            profile: Some("default"),
            ..Default::default()
        },
        "test",
        MemoryKeyringBackend::default(),
        sink.clone(),
    )
    .unwrap();
    rt.set_ssh_paths(Some(kh), Some(fx.user_key.clone()));

    // Weird filenames in remote directory remain inert data.
    fs::write(fx.remote_sandbox.join("hello world.txt"), b"a").unwrap();
    fs::write(fx.remote_sandbox.join("quote's \"f\" $(id).txt"), b"b").unwrap();

    let cancel = CancelToken::new();
    rt.start_server_tunnel("srv-b", &cancel).unwrap();

    let session = rt
        .open_remote_browser("srv-b", Some(fx.remote_sandbox.to_str().unwrap()), &cancel)
        .unwrap();
    let entries = rt
        .list_remote_children(&session.browser_id, &session.root_path)
        .unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"hello world.txt"));
    assert!(names.contains(&"quote's \"f\" $(id).txt"));

    let new_dir = fx.remote_sandbox.join("sub_sftp");
    let created = rt
        .create_remote_directory(&session.browser_id, new_dir.to_str().unwrap())
        .unwrap();
    assert_eq!(created.kind, RemotePathKind::Directory);
    assert!(created.exists);

    fs::write(new_dir.join("child.txt"), b"nested").unwrap();
    let stat_file = rt
        .stat_remote_path(
            &session.browser_id,
            new_dir.join("child.txt").to_str().unwrap(),
        )
        .unwrap();
    assert_eq!(stat_file.kind, RemotePathKind::File);
    assert!(stat_file.exists);

    rt.delete_remote_path(&session.browser_id, new_dir.to_str().unwrap(), true)
        .unwrap();
    assert!(!new_dir.exists());

    let missing_stat = rt
        .stat_remote_path(&session.browser_id, new_dir.to_str().unwrap())
        .unwrap();
    assert_eq!(missing_stat.kind, RemotePathKind::Missing);
    assert!(!missing_stat.exists);

    // Stopping server tunnel must auto-close the open remote browser session.
    assert!(rt
        .stop_server_tunnel("srv-b", Duration::from_secs(2))
        .unwrap());
    let closed_err = rt
        .list_remote_children(&session.browser_id, &session.root_path)
        .unwrap_err();
    assert_eq!(closed_err.code, "NOT_FOUND");
    assert!(sink.snapshot().iter().any(|e| matches!(
        e,
        DomainEvent::RemoteBrowserClosed { session_id, .. } if session_id == &session.browser_id
    )));

    // Open again and verify ProfileRuntime::shutdown closes browser + tunnel cleanly.
    rt.start_server_tunnel("srv-b", &cancel).unwrap();
    let s2 = rt
        .open_remote_browser("srv-b", Some(fx.remote_sandbox.to_str().unwrap()), &cancel)
        .unwrap();
    rt.shutdown(Duration::from_secs(2));
    assert_eq!(rt.supervisor().session_count(), 0);
    assert_eq!(
        rt.list_remote_children(&s2.browser_id, &s2.root_path)
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
}

#[test]
fn boundary_tunnel_failures_one_shot_cleanup_and_caller_owned_reuse() {
    let fx = SshdFixture::start();
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let p_good = free_port();
    let p_early = free_port();
    let p_hang = free_port();
    let kh = tmp.path().join("known_hosts");
    fx.write_known_hosts(&kh, &[fx.port, p_good]);

    ssh_fixture::seed_default_boundary_profile(
        &cfg,
        json!({
            "servers": [
                {
                    "id": "srv-good",
                    "name": "Good",
                    "host": "internal.box",
                    "connectionMethod": "boundary",
                    "sshPort": 22,
                    "sshUser": fx.user,
                    "boundaryTargetId": "ttcp_valid",
                    "boundaryListenPort": p_good,
                },
                {
                    "id": "srv-early",
                    "name": "EarlyExit",
                    "host": "internal.box",
                    "connectionMethod": "boundary",
                    "sshPort": 22,
                    "sshUser": fx.user,
                    "boundaryTargetId": "early-exit",
                    "boundaryListenPort": p_early,
                },
                {
                    "id": "srv-hang",
                    "name": "Hang",
                    "host": "internal.box",
                    "connectionMethod": "boundary",
                    "sshPort": 22,
                    "sshUser": fx.user,
                    "boundaryTargetId": "hang-no-listen",
                    "boundaryListenPort": p_hang,
                },
                {
                    "id": "srv-dup-port",
                    "name": "DupPort",
                    "host": "internal.box",
                    "connectionMethod": "boundary",
                    "sshPort": 22,
                    "sshUser": fx.user,
                    "boundaryTargetId": "ttcp_valid",
                    "boundaryListenPort": p_good,
                }
            ]
        }),
        &fx.boundary_script,
    );

    let rt = ProfileRuntime::open_with_backend(
        &ProfileLocationInput {
            config_dir: Some(&cfg),
            desktop_config_dir: Some(&desktop),
            profile: Some("default"),
            ..Default::default()
        },
        "test",
        MemoryKeyringBackend::default(),
    )
    .unwrap();
    rt.set_ssh_paths(Some(kh), Some(fx.user_key.clone()));
    rt.set_tunnel_ready_timeout(Some(Duration::from_millis(350)));
    let cancel = CancelToken::new();

    // Early exit fails promptly
    let early_err = rt.start_server_tunnel("srv-early", &cancel).unwrap_err();
    assert_eq!(early_err.code, "OPERATION_FAILED");
    assert_eq!(early_err.reason, "tunnel_exited_early");

    // Hang timeout cleans up child and frees port
    let hang_err = rt.start_server_tunnel("srv-hang", &cancel).unwrap_err();
    assert_eq!(hang_err.code, "TIMEOUT");
    assert_eq!(hang_err.reason, "tunnel_ready_timeout");
    assert_eq!(rt.supervisor().session_count(), 0);

    // One-shot SCP upload without running tunnel -> opens temporary tunnel and tears it down
    let local_file = tmp.path().join("oneshot.txt");
    fs::write(&local_file, b"one-shot-payload").unwrap();
    let res = rt
        .scp_upload(
            "srv-good",
            local_file.to_str().unwrap(),
            fx.remote_sandbox.to_str().unwrap(),
            false,
            &cancel,
        )
        .unwrap();
    assert_eq!(res.state, ScpTransferState::Success);
    assert_eq!(
        fs::read(fx.remote_sandbox.join("oneshot.txt")).unwrap(),
        b"one-shot-payload"
    );
    assert!(rt
        .supervisor()
        .active_session_for_function(&server_tunnel_function_id("srv-good"))
        .is_none());

    // One-shot failure still cleans up temporary tunnel
    let dl_missing = rt.scp_download(
        "srv-good",
        fx.remote_sandbox.join("nonexistent.txt").to_str().unwrap(),
        tmp.path().join("out.txt").to_str().unwrap(),
        false,
        true,
        &cancel,
    );
    assert!(dl_missing.is_err());
    assert!(rt
        .supervisor()
        .active_session_for_function(&server_tunnel_function_id("srv-good"))
        .is_none());

    // Caller-owned tunnel blocks conflicting port start and is REUSED by SCP without closing
    let tunnel = rt.start_server_tunnel("srv-good", &cancel).unwrap();
    let dup_port_err = rt.start_server_tunnel("srv-dup-port", &cancel).unwrap_err();
    assert_eq!(dup_port_err.code, "CONFLICT");
    assert_eq!(dup_port_err.reason, "port_in_use");

    let dl_target = tmp.path().join("reused.txt");
    rt.scp_download(
        "srv-good",
        fx.remote_sandbox.join("oneshot.txt").to_str().unwrap(),
        dl_target.to_str().unwrap(),
        false,
        false,
        &cancel,
    )
    .unwrap();
    assert_eq!(fs::read(&dl_target).unwrap(), b"one-shot-payload");
    assert_eq!(
        rt.supervisor()
            .active_session_for_function(&tunnel.function_id)
            .as_deref(),
        Some(tunnel.session_id.as_str())
    );
    assert!(rt
        .stop_server_tunnel("srv-good", Duration::from_secs(2))
        .unwrap());
}
