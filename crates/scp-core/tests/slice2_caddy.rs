use serde_json::json;
use std::fs;
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tempfile::TempDir;

use scp_core::models::CADDY_FUNCTION_ID;
use scp_core::profile::ProfileLocationInput;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::MemoryKeyringBackend;
use scp_core::supervisor::CancelToken;

fn open_caddy_runtime(
    tmp: &TempDir,
    caddy_folder: Option<&std::path::Path>,
    script_name: &str,
    script_body: Option<&str>,
) -> ProfileRuntime<MemoryKeyringBackend> {
    let cfg_dir = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let pdir = cfg_dir.join("profiles").join("default");
    fs::create_dir_all(&pdir).unwrap();
    if let Some(folder) = caddy_folder {
        fs::write(
            pdir.join("sync-local.json"),
            json!({ "caddyFolder": folder.to_string_lossy() }).to_string(),
        )
        .unwrap();
    }
    let rt = ProfileRuntime::open_with_backend(
        &ProfileLocationInput {
            config_dir: Some(&cfg_dir),
            desktop_config_dir: Some(&desktop),
            profile: Some("default"),
            ..Default::default()
        },
        "test",
        MemoryKeyringBackend::default(),
    )
    .unwrap();
    if let Some(body) = script_body {
        let exe_path: PathBuf = tmp.path().join(script_name);
        fs::write(&exe_path, body).unwrap();
        fs::set_permissions(&exe_path, fs::Permissions::from_mode(0o755)).unwrap();
        rt.set_caddy_executable(Some(exe_path.to_string_lossy().into_owned()));
    }
    rt
}

fn free_loopback_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn fake_caddy_script(port: u16) -> String {
    format!(
        r#"#!/usr/bin/env python3
import signal, socket, sys, time
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
assert sys.argv[1:] == ["run", "--config", "Caddyfile"], sys.argv[1:]
with open("Caddyfile") as f:
    assert str({port}) in f.read()
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", {port}))
s.listen(8)
print("INFO serving initial configuration", flush=True)
time.sleep(30)
"#
    )
}

#[test]
fn caddy_listener_ownership_cleanup() {
    let tmp = TempDir::new().unwrap();
    let cancel = CancelToken::new();

    let empty_dir = tmp.path().join("no-caddyfile");
    fs::create_dir_all(&empty_dir).unwrap();
    let valid_dir = tmp.path().join("caddy-site");
    fs::create_dir_all(&valid_dir).unwrap();
    let port = free_loopback_port();
    fs::write(
        valid_dir.join("Caddyfile"),
        format!(":{port} {{\n  respond \"ok\" 200\n}}\n"),
    )
    .unwrap();

    let rt_invalid = open_caddy_runtime(&tmp, Some(&valid_dir), "unused", None);
    rt_invalid.set_caddy_executable(Some("/nonexistent/fake-caddy-bin".into()));

    assert_eq!(
        rt_invalid.start_caddy(Some(""), &cancel).unwrap_err().code,
        "INVALID_ARGUMENTS"
    );
    let missing_dir = tmp.path().join("missing-folder");
    assert_eq!(
        rt_invalid
            .start_caddy(Some(missing_dir.to_str().unwrap()), &cancel)
            .unwrap_err()
            .code,
        "INVALID_ARGUMENTS"
    );
    assert_eq!(
        rt_invalid
            .start_caddy(Some(empty_dir.to_str().unwrap()), &cancel)
            .unwrap_err()
            .code,
        "INVALID_ARGUMENTS"
    );
    let missing_bin = rt_invalid
        .start_caddy(Some(valid_dir.to_str().unwrap()), &cancel)
        .unwrap_err();
    assert_eq!(missing_bin.code, "CAPABILITY_UNAVAILABLE");
    assert_eq!(missing_bin.reason, "executable_not_found");
    drop(rt_invalid);

    // Early-exit failure is reported explicitly.
    let tmp_early = TempDir::new().unwrap();
    let site_early = tmp_early.path().join("site");
    fs::create_dir_all(&site_early).unwrap();
    fs::write(site_early.join("Caddyfile"), ":0 {\n}\n").unwrap();
    let rt_early = open_caddy_runtime(
        &tmp_early,
        Some(&site_early),
        "caddy-early",
        Some("#!/bin/sh\necho 'Error: invalid Caddyfile' >&2\nexit 1\n"),
    );
    let early_err = rt_early.start_caddy(None, &cancel).unwrap_err();
    assert_eq!(early_err.code, "OPERATION_FAILED");
    assert_eq!(early_err.reason, "caddy_exited_early");
    drop(rt_early);

    // Live listener + default sync-local caddyFolderPath + duplicate conflict + shutdown cleanup.
    let tmp_live = TempDir::new().unwrap();
    let rt = open_caddy_runtime(
        &tmp_live,
        Some(&valid_dir),
        "caddy-live",
        Some(&fake_caddy_script(port)),
    );
    let started = rt.start_caddy(None, &cancel).unwrap();
    assert_eq!(started.function_id, CADDY_FUNCTION_ID);

    let st = rt.get_function_status(CADDY_FUNCTION_ID).unwrap();
    assert_eq!(st.state, "running");
    assert_eq!(st.session_id.as_deref(), Some(started.session_id.as_str()));
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());

    let dup = rt.start_caddy(None, &cancel).unwrap_err();
    assert_eq!(dup.code, "CONFLICT");
    assert_eq!(dup.reason, "function_already_running");

    rt.shutdown(Duration::from_secs(2));
    assert_eq!(rt.supervisor().session_count(), 0);
    let deadline = Instant::now() + Duration::from_secs(2);
    while TcpListener::bind(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "listener port {port} stayed bound"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn caddy_owned_stop() {
    let tmp = TempDir::new().unwrap();
    let site = tmp.path().join("site");
    fs::create_dir_all(&site).unwrap();
    let port = free_loopback_port();
    fs::write(
        site.join("Caddyfile"),
        format!(":{port} {{\n  respond \"ok\" 200\n}}\n"),
    )
    .unwrap();

    let rt = open_caddy_runtime(&tmp, None, "fake-caddy", Some(&fake_caddy_script(port)));
    let cancel = CancelToken::new();
    let _started = rt
        .start_caddy(Some(site.to_str().unwrap()), &cancel)
        .unwrap();
    assert!(rt.stop_caddy(Duration::from_secs(2)).unwrap());
    assert!(!rt.stop_caddy(Duration::from_secs(1)).unwrap());

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let st = rt.get_function_status(CADDY_FUNCTION_ID).unwrap();
        if st.session_id.is_none() {
            assert_eq!(st.state, "idle");
            assert_eq!(st.message, "Stopped");
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(15));
    }
}
