#[path = "support/ssh_fixture.rs"]
mod ssh_fixture;

use serde_json::json;
use ssh_fixture::{free_port, SshdFixture};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};
use tempfile::TempDir;

use scp_core::models::{ScpDownloadRequest, ScpTransferState};
use scp_core::profile::ProfileLocationInput;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::MemoryKeyringBackend;
use scp_core::supervisor::CancelToken;

fn open_test_runtime(
    cfg: &Path,
    desktop: &Path,
    fx: &SshdFixture,
) -> ProfileRuntime<MemoryKeyringBackend> {
    let rt = ProfileRuntime::open_with_backend(
        &ProfileLocationInput {
            config_dir: Some(cfg),
            desktop_config_dir: Some(desktop),
            profile: Some("default"),
            ..Default::default()
        },
        "test",
        MemoryKeyringBackend::default(),
    )
    .unwrap();
    rt.set_ssh_paths(Some(fx.known_hosts.clone()), Some(fx.user_key.clone()));
    rt
}

fn wait_transfer_terminal(
    rt: &ProfileRuntime<MemoryKeyringBackend>,
    transfer_id: &str,
) -> scp_core::models::ScpTransferEntry {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let entry = rt.get_transfer(transfer_id).unwrap().unwrap();
        if entry.state != ScpTransferState::Pending {
            return entry;
        }
        assert!(Instant::now() < deadline, "transfer did not finish in time");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn scp_ownership_registration_cancel_race() {
    let fx = SshdFixture::start();
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let b_port = free_port();
    let kh = tmp.path().join("known_hosts");
    fx.write_known_hosts(&kh, &[fx.port, b_port]);
    seed_two_servers(&cfg, &fx, b_port);
    let rt = open_test_runtime(&cfg, &desktop, &fx);
    rt.set_ssh_paths(Some(kh), Some(fx.user_key.clone()));

    let local_file = tmp.path().join("async.bin");
    fs::write(&local_file, b"async-scp-payload").unwrap();
    verify_precancel_and_tunnel_guard(&rt, &local_file, &fx);
    verify_async_upload_and_download(&rt, &tmp, &local_file, &fx);
}

fn seed_two_servers(cfg: &Path, fx: &SshdFixture, b_port: u16) {
    ssh_fixture::seed_default_boundary_profile(
        cfg,
        json!({
            "servers": [
                {
                    "id": "srv-direct", "name": "Direct", "host": "127.0.0.1",
                    "connectionMethod": "ssh", "sshPort": fx.port, "sshUser": fx.user
                },
                {
                    "id": "srv-b", "name": "Boundary", "host": "ignored",
                    "connectionMethod": "boundary", "sshPort": 22, "sshUser": fx.user,
                    "boundaryTargetId": "ttcp_ok", "boundaryListenPort": b_port
                }
            ]
        }),
        &fx.boundary_script,
    );
}

fn verify_precancel_and_tunnel_guard(
    rt: &ProfileRuntime<MemoryKeyringBackend>,
    local_file: &Path,
    fx: &SshdFixture,
) {
    let pre_cancel = CancelToken::new();
    pre_cancel.cancel();
    let err_cancel = rt
        .start_scp_upload(
            "srv-direct",
            local_file.to_str().unwrap(),
            fx.remote_sandbox.to_str().unwrap(),
            false,
            &pre_cancel,
        )
        .unwrap_err();
    assert_eq!(err_cancel.code, "INTERRUPTED");
    assert_eq!(rt.supervisor().session_count(), 0);
    assert!(rt
        .list_transfers()
        .unwrap()
        .iter()
        .all(|t| t.state != ScpTransferState::Pending));

    let cancel = CancelToken::new();
    let err_tunnel = rt
        .start_scp_upload(
            "srv-b",
            local_file.to_str().unwrap(),
            fx.remote_sandbox.to_str().unwrap(),
            false,
            &cancel,
        )
        .unwrap_err();
    assert_eq!(err_tunnel.code, "CONFLICT");
    assert_eq!(err_tunnel.reason, "tunnel_not_running");
    assert_eq!(err_tunnel.next_action, "start_server_tunnel");
}

fn verify_async_upload_and_download(
    rt: &ProfileRuntime<MemoryKeyringBackend>,
    tmp: &TempDir,
    local_file: &Path,
    fx: &SshdFixture,
) {
    let cancel = CancelToken::new();
    let started = rt
        .start_scp_upload(
            "srv-direct",
            local_file.to_str().unwrap(),
            fx.remote_sandbox.to_str().unwrap(),
            false,
            &cancel,
        )
        .unwrap();
    let initial = rt.get_transfer(&started.transfer_id).unwrap().unwrap();
    assert_eq!(
        initial.session_id.as_deref(),
        Some(started.session_id.as_str())
    );
    assert_eq!(
        wait_transfer_terminal(rt, &started.transfer_id).state,
        ScpTransferState::Success
    );
    assert_eq!(
        fs::read(fx.remote_sandbox.join("async.bin")).unwrap(),
        b"async-scp-payload"
    );

    rt.start_server_tunnel("srv-b", &cancel).unwrap();
    let dl_dest = tmp.path().join("async_dl.bin");
    let remote_bin = fx.remote_sandbox.join("async.bin");
    let dl_req = ScpDownloadRequest {
        server_id: "srv-b",
        remote_path: remote_bin.to_str().unwrap(),
        local_target: dl_dest.to_str().unwrap(),
        recursive: false,
        overwrite: false,
    };
    let dl_started = rt.start_scp_download(dl_req, &cancel).unwrap();
    assert_eq!(
        wait_transfer_terminal(rt, &dl_started.transfer_id).state,
        ScpTransferState::Success
    );
    assert_eq!(fs::read(&dl_dest).unwrap(), b"async-scp-payload");
    let conflict = rt.start_scp_download(dl_req, &cancel).unwrap_err();
    assert_eq!(conflict.code, "CONFLICT");
    assert_eq!(conflict.reason, "destination_exists");
}
