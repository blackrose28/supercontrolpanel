use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use scp_core::events::{BufferedEventSink, DomainEvent};
use scp_core::file_paths::{
    normalize_remote_transfer_path, validate_local_download_target, validate_local_upload_source,
};
use scp_core::models::{now_ms, ScpTransferState, OPENVPN_FUNCTION_ID};
use scp_core::profile::ProfileLocationInput;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::MemoryKeyringBackend;
use scp_core::supervisor::{AcceptOutcome, CancelToken, ChildLaunch, ProcessSupervisor, SpawnMode};
use scp_core::transfer_store::{
    attach_scp_transfer_session, complete_scp_transfer_session, create_scp_transfer_entry,
    get_scp_transfer_entry, handle_scp_transfer_output, list_scp_transfer_entries,
    ScpTransferStore, MAX_TRANSFER_HISTORY,
};
use tempfile::TempDir;

fn loc<'a>(
    config_dir: &'a std::path::Path,
    desktop: &'a std::path::Path,
) -> ProfileLocationInput<'a> {
    ProfileLocationInput {
        config_dir: Some(config_dir),
        env_config_dir: None,
        xdg_config_home: None,
        home_dir: None,
        desktop_config_dir: Some(desktop),
        profile: Some("slice2-sup"),
    }
}

fn pipe_launch(function_id: &str, kind: &'static str, script: &str) -> ChildLaunch {
    ChildLaunch {
        function_id: function_id.to_string(),
        program: "/bin/sh".to_string(),
        args: vec!["-c".to_string(), script.to_string()],
        cwd: None,
        env: Vec::new(),
        env_remove: Vec::new(),
        mode: SpawnMode::Pipe,
        kill_tree: true,
        kind,
    }
}

#[test]
fn pre_accept_cancellation_reaps_child_and_accepted_survives_request_cancel() {
    let tmp = TempDir::new().unwrap();
    let pid_file = tmp.path().join("cancelled.pid");
    let sup = ProcessSupervisor::new();

    let exit_calls = Arc::new(AtomicUsize::new(0));
    let exit_calls_ref = Arc::clone(&exit_calls);
    let cancel = CancelToken::new();
    let cancel_in_gate = cancel.clone();
    let launch = pipe_launch(
        "fn-cancel",
        "ssh",
        &format!("echo $$ > {}; sleep 30", pid_file.display()),
    );
    let outcome = sup
        .spawn_with_gate(
            launch,
            &cancel,
            move |_sid| {
                cancel_in_gate.cancel();
                AcceptOutcome::CancelledBeforeAccept
            },
            move |_sid, _code, _reason| {
                exit_calls_ref.fetch_add(1, Ordering::SeqCst);
            },
            None,
        )
        .unwrap();
    assert!(matches!(outcome, AcceptOutcome::CancelledBeforeAccept));
    assert_eq!(sup.session_count(), 0);
    assert_eq!(exit_calls.load(Ordering::SeqCst), 0);

    // Accepted session must outlive post-accept request token cancellation.
    let cancel_after = CancelToken::new();
    let outcome = sup
        .spawn_with_gate(
            pipe_launch("fn-live", "ssh", "sleep 30"),
            &cancel_after,
            |_sid| AcceptOutcome::Accepted(()),
            |_sid, _code, _reason| {},
            None,
        )
        .unwrap();
    let sid = match outcome {
        AcceptOutcome::Accepted(sid) => sid,
        AcceptOutcome::CancelledBeforeAccept => panic!("expected accepted session"),
    };
    cancel_after.cancel();
    assert_eq!(sup.session_count(), 1);
    assert_eq!(
        sup.active_session_for_function("fn-live").as_deref(),
        Some(sid.as_str())
    );
    assert!(sup.stop_session(&sid, Duration::from_secs(2)).unwrap());
    assert_eq!(sup.session_count(), 0);
}

#[test]
fn duplicate_function_and_port_conflicts_reject_without_spawning_second_child() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let rt = ProfileRuntime::open_with_backend(
        &loc(&cfg, &desktop),
        "holder",
        MemoryKeyringBackend::default(),
    )
    .unwrap();

    rt.reserve_port(48921, "caddy-run").unwrap();
    let port_err = rt.reserve_port(48921, "tunnel-1").unwrap_err();
    assert_eq!(port_err.code, "CONFLICT");
    assert_eq!(port_err.reason, "port_in_use");
    assert_eq!(port_err.exit_code(), 5);
    rt.release_port(48921);
    rt.reserve_port(48921, "tunnel-1").unwrap();
    rt.release_port(48921);

    let marker = tmp.path().join("spawns.log");
    let script = format!("printf 'spawn\\n' >> {}; sleep 30", marker.display());
    let cancel = CancelToken::new();
    let s1 = rt
        .start_supervised_function(
            pipe_launch("caddy-run", "caddy", &script),
            &cancel,
            None,
            "Running",
        )
        .unwrap();
    assert!(!s1.is_empty());

    let dup_err = rt
        .start_supervised_function(
            pipe_launch("caddy-run", "caddy", &script),
            &cancel,
            None,
            "Running",
        )
        .unwrap_err();
    assert_eq!(dup_err.code, "CONFLICT");
    assert_eq!(dup_err.reason, "function_already_running");
    assert_eq!(dup_err.exit_code(), 5);

    std::thread::sleep(Duration::from_millis(80));
    let lines = fs::read_to_string(&marker).unwrap_or_default();
    assert_eq!(lines.lines().count(), 1);
    assert!(rt
        .stop_supervised_function("caddy-run", Duration::from_secs(2))
        .unwrap());
}

#[test]
fn stop_is_idempotent_and_stale_session_cannot_stop_newer_child() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let rt = ProfileRuntime::open_with_backend(
        &loc(&cfg, &desktop),
        "holder",
        MemoryKeyringBackend::default(),
    )
    .unwrap();

    assert!(!rt
        .stop_supervised_function("caddy-run", Duration::from_millis(200))
        .unwrap());

    let cancel = CancelToken::new();
    let s1 = rt
        .start_supervised_function(
            pipe_launch("caddy-run", "caddy", "sleep 30"),
            &cancel,
            None,
            "Running",
        )
        .unwrap();
    assert!(rt
        .stop_supervised_function("caddy-run", Duration::from_secs(2))
        .unwrap());
    assert!(!rt
        .stop_supervised_function("caddy-run", Duration::from_millis(200))
        .unwrap());

    let s2 = rt
        .start_supervised_function(
            pipe_launch("caddy-run", "caddy", "sleep 30"),
            &cancel,
            None,
            "Running",
        )
        .unwrap();
    assert_ne!(s1, s2);
    // Stopping by stale session id must not affect newer child.
    assert!(!rt
        .supervisor()
        .stop_session(&s1, Duration::from_secs(1))
        .unwrap());
    assert_eq!(
        rt.supervisor()
            .active_session_for_function("caddy-run")
            .as_deref(),
        Some(s2.as_str())
    );
    assert!(rt
        .stop_supervised_function("caddy-run", Duration::from_secs(2))
        .unwrap());
}

#[test]
fn bounded_output_ring_buffer_caps_64kib_and_preserves_tail_lines() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let rt = ProfileRuntime::open_with_backend(
        &loc(&cfg, &desktop),
        "holder",
        MemoryKeyringBackend::default(),
    )
    .unwrap();

    let unknown_err = rt.get_function_logs("does-not-exist").unwrap_err();
    assert_eq!(unknown_err.code, "NOT_FOUND");

    let script = "i=0; while [ $i -lt 1600 ]; do printf 'line-%04d-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n' $i; i=$((i+1)); done";
    let cancel = CancelToken::new();
    let sid = rt
        .start_supervised_function(
            pipe_launch(OPENVPN_FUNCTION_ID, "vpn", script),
            &cancel,
            None,
            "Connecting",
        )
        .unwrap();
    assert!(rt
        .supervisor()
        .wait_for_session_exit(&sid, Duration::from_secs(5)));

    let logs = rt.get_function_logs(OPENVPN_FUNCTION_ID).unwrap();
    let total_bytes: usize = logs.iter().map(|l| l.len() + 1).sum();
    assert!(total_bytes <= 64 * 1024 + 256, "bytes={total_bytes}");
    assert!(logs.iter().any(|l| l.contains("line-1599-")));
    assert!(!logs.iter().any(|l| l.contains("line-0000-")));
}

#[test]
fn transfer_store_and_file_paths_preserve_desktop_parity() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("file.txt");
    fs::write(&src, b"hello").unwrap();
    assert!(validate_local_upload_source(src.to_str().unwrap()).is_ok());
    let missing_err =
        validate_local_upload_source(tmp.path().join("nope.txt").to_str().unwrap()).unwrap_err();
    assert_eq!(missing_err.reason_code, "invalid_source_path");
    assert_eq!(missing_err.into_core_error().code, "INVALID_ARGUMENTS");

    let dst = tmp.path().join("out.txt");
    assert!(validate_local_download_target(dst.to_str().unwrap()).is_ok());
    assert_eq!(
        normalize_remote_transfer_path("rel/file.txt", Some("/var/data"), "bad_remote", false)
            .unwrap(),
        "/var/data/rel/file.txt"
    );
    assert!(
        normalize_remote_transfer_path("../../etc/passwd", Some("/var"), "bad_remote", false)
            .is_err()
    );

    let store = ScpTransferStore::default();
    for i in 0..(MAX_TRANSFER_HISTORY + 5) {
        let tid = create_scp_transfer_entry(
            &store,
            now_ms,
            format!("Upload {i}"),
            "Starting",
            Some(src.to_string_lossy().into_owned()),
            Some("/tmp/out".to_string()),
        )
        .unwrap();
        let sid = format!("pty-test-{i}");
        attach_scp_transfer_session(&store, &tid, &sid).unwrap();
        handle_scp_transfer_output(&store, &sid, "\u{1b}[32m45%\u{1b}[0m transferring", now_ms)
            .unwrap();
        let pending = get_scp_transfer_entry(&store, &tid).unwrap().unwrap();
        assert_eq!(pending.progress_percent, Some(45));
        assert_eq!(pending.state, ScpTransferState::Pending);
        complete_scp_transfer_session(&store, &sid, Some(0), "ok", now_ms).unwrap();
        let done = get_scp_transfer_entry(&store, &tid).unwrap().unwrap();
        assert_eq!(done.state, ScpTransferState::Success);
        assert_eq!(done.progress_percent, Some(100));
    }
    assert_eq!(
        list_scp_transfer_entries(&store).unwrap().len(),
        MAX_TRANSFER_HISTORY
    );
}

#[test]
fn runtime_shutdown_orders_teardown_and_emits_buffered_events() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let sink = Arc::new(BufferedEventSink::new());
    let rt = ProfileRuntime::open_with_backend_and_sink(
        &loc(&cfg, &desktop),
        "holder",
        MemoryKeyringBackend::default(),
        sink.clone(),
    )
    .unwrap();

    let cancel = CancelToken::new();
    let ssh_sid = rt
        .start_supervised_function(
            pipe_launch("server-ssh-1", "ssh", "sleep 30"),
            &cancel,
            None,
            "Running",
        )
        .unwrap();
    let tun_sid = rt
        .start_supervised_function(
            pipe_launch("server-tunnel-1", "tunnel", "sleep 30"),
            &cancel,
            None,
            "Running",
        )
        .unwrap();
    let vpn_sid = rt
        .start_supervised_function(
            pipe_launch(OPENVPN_FUNCTION_ID, "vpn", "sleep 30"),
            &cancel,
            None,
            "Running",
        )
        .unwrap();

    let started = Instant::now();
    rt.shutdown(Duration::from_secs(3));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(rt.supervisor().session_count(), 0);

    let events = sink.snapshot();
    let exited_ids: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::ProcessExited { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    let pos_ssh = exited_ids.iter().position(|id| id == &ssh_sid).unwrap();
    let pos_tun = exited_ids.iter().position(|id| id == &tun_sid).unwrap();
    let pos_vpn = exited_ids.iter().position(|id| id == &vpn_sid).unwrap();
    assert!(
        pos_ssh < pos_tun && pos_tun < pos_vpn,
        "exited order: {exited_ids:?}"
    );

    let post_err = rt
        .start_supervised_function(
            pipe_launch("caddy-run", "caddy", "sleep 1"),
            &cancel,
            None,
            "Running",
        )
        .unwrap_err();
    assert_eq!(post_err.code, "CONFLICT");
    assert_eq!(post_err.reason, "shutting_down");
}
