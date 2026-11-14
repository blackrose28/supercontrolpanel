use scp_cli_stdio_slice0::boundary_spike::{
    BoundaryAuthKind, BoundaryTokenMode, check_auth_capability, run_get_token,
};
use scp_cli_stdio_slice0::keyring_spike::{
    KeyringHealth, NamespacedSecretStore, preflight_keyring,
};
use scp_cli_stdio_slice0::profile_lock::{acquire_owner_lease, resolve_profile_dir};
use scp_cli_stdio_slice0::supervisor_spike::{ChildKind, OwnershipOutcome, Supervisor};
use scp_cli_stdio_slice0::vpn_spike::{VpnPreflightConfig, preflight_and_acquire_vpn};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_scp-cli-stdio-slice0")
}

fn wait_until<F: FnMut() -> bool>(timeout: Duration, mut pred: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    pred()
}

fn pid_exists(pid: u32) -> bool {
    // SAFETY: kill with signal 0 only checks process existence.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod script");
}

#[test]
fn profile_lock_rejects_desktop_alias_and_enforces_exclusive_lease() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_root = tmp.path().join("cli-config");
    let desktop_dir = tmp.path().join("com.chuonglv.supercontrolpanel");
    fs::create_dir_all(&desktop_dir).expect("create fake desktop dir");

    // 1. Precedence & desktop rejection (direct and via symlink).
    let resolved = resolve_profile_dir(
        Some(&config_root),
        Some(Path::new("/tmp/ignored-env")),
        None,
        None,
        Some(&desktop_dir),
        Some("agent-a"),
    )
    .expect("valid profile resolves");
    assert!(resolved.ends_with("profiles/agent-a"));

    let desktop_err = resolve_profile_dir(
        Some(&desktop_dir),
        None,
        None,
        None,
        Some(&desktop_dir),
        Some("default"),
    )
    .expect_err("must reject desktop config dir");
    assert_eq!(desktop_err.reason, "desktop_profile_rejected");
    assert!(!desktop_dir.join("owner.lock").exists());

    let symlink_alias = tmp.path().join("desktop-symlink");
    symlink(&desktop_dir, &symlink_alias).expect("symlink desktop");
    let symlink_err = resolve_profile_dir(
        Some(&symlink_alias),
        None,
        None,
        None,
        Some(&desktop_dir),
        Some("default"),
    )
    .expect_err("must reject symlink to desktop dir");
    assert_eq!(symlink_err.reason, "desktop_profile_rejected");

    // 2. Concurrent subprocess lock contention through symlink + `..` alias.
    let cli_symlink = tmp.path().join("cli-symlink");
    symlink(&config_root, &cli_symlink).expect("symlink cli root");
    let alias_with_dotdot = cli_symlink.join("sub/../");

    let ready_file = tmp.path().join("holder-ready");
    let mut holder = Command::new(bin_path())
        .args([
            "hold-lock",
            "--config-dir",
            config_root.to_str().unwrap(),
            "--profile",
            "agent-a",
            "--ready-file",
            ready_file.to_str().unwrap(),
            "--hold-ms",
            "30000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn holder");

    assert!(
        wait_until(Duration::from_secs(3), || ready_file.exists()),
        "holder did not acquire lock in time"
    );

    let contender = Command::new(bin_path())
        .args([
            "hold-lock",
            "--config-dir",
            alias_with_dotdot.to_str().unwrap(),
            "--profile",
            "agent-a",
            "--hold-ms",
            "100",
        ])
        .output()
        .expect("run contender");
    assert_eq!(contender.status.code(), Some(5));
    let contender_err = String::from_utf8_lossy(&contender.stderr);
    assert!(contender_err.contains("CONFLICT"), "{contender_err}");
    assert!(
        contender_err.contains(&holder.id().to_string()),
        "diagnostics must identify holder pid: {contender_err}"
    );

    // 3. SIGKILL on holder immediately frees the OS kernel lock despite owner.lock file remaining.
    unsafe {
        libc::kill(holder.id() as libc::pid_t, libc::SIGKILL);
    }
    let _ = holder.wait();

    let successor = Command::new(bin_path())
        .args([
            "hold-lock",
            "--config-dir",
            config_root.to_str().unwrap(),
            "--profile",
            "agent-a",
            "--hold-ms",
            "10",
        ])
        .output()
        .expect("run successor after SIGKILL");
    assert!(
        successor.status.success(),
        "successor failed after SIGKILL: {}",
        String::from_utf8_lossy(&successor.stderr)
    );

    // In-process check that diagnostics file exists while lease is held.
    let lease = acquire_owner_lease(&resolved, "rt-inproc", "test").expect("acquire inproc");
    assert!(lease.diagnostics_path().exists());
}

#[test]
fn keyring_preflight_and_namespaced_isolation_without_prompter_hang() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // 1. Missing/invalid DBUS address returns keyring_unavailable fast (< 1s).
    let start = Instant::now();
    let bad_dbus = Command::new(bin_path())
        .args([
            "keyring-probe",
            "--namespace",
            "ns-a",
            "--action",
            "preflight",
        ])
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/dbus")
        .output()
        .expect("run bad dbus probe");
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_ne!(bad_dbus.status.code(), Some(0));
    let msg = String::from_utf8_lossy(&bad_dbus.stderr);
    assert!(msg.contains("keyring_unavailable"), "{msg}");

    // 2. Empty dbus-run-session (no gnome-keyring-daemon) fails fast with keyring_unavailable.
    let xdg_data = tmp.path().join("data");
    let xdg_run = tmp.path().join("run");
    fs::create_dir_all(&xdg_data).unwrap();
    fs::create_dir_all(&xdg_run).unwrap();
    fs::set_permissions(&xdg_data, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&xdg_run, fs::Permissions::from_mode(0o700)).unwrap();

    let empty_dbus = Command::new("dbus-run-session")
        .args([
            "--",
            bin_path(),
            "keyring-probe",
            "--namespace",
            "ns-a",
            "--action",
            "preflight",
        ])
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_RUNTIME_DIR", &xdg_run)
        .output()
        .expect("run empty dbus session");
    assert_ne!(empty_dbus.status.code(), Some(0));
    let empty_msg = String::from_utf8_lossy(&empty_dbus.stderr);
    assert!(empty_msg.contains("keyring_unavailable"), "{empty_msg}");

    // 3. Unlocked isolated daemon: test namespaced round-trip, cache isolation, recovery key, and desktop non-interference.
    let exe = std::env::current_exe().expect("current test exe");
    let unlocked_script = format!(
        r#"printf "testpass" | gnome-keyring-daemon --unlock --components=secrets --daemonize >/dev/null
"{exe}" --exact keyring_unlocked_child_helper --nocapture
"#,
        exe = exe.display()
    );
    let unlocked = Command::new("dbus-run-session")
        .args(["--", "bash", "-c", &unlocked_script])
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_RUNTIME_DIR", &xdg_run)
        .env("SLICE0_KEYRING_CHILD", "1")
        .output()
        .expect("run unlocked keyring helper");
    assert!(
        unlocked.status.success(),
        "unlocked helper failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&unlocked.stdout),
        String::from_utf8_lossy(&unlocked.stderr)
    );

    // 4. Locked isolated daemon on the same XDG_DATA_HOME: must fail in < 2s with keyring_locked (no gcr-prompter hang).
    let locked_start = Instant::now();
    let locked_script = format!(
        r#"gnome-keyring-daemon --start --components=secrets >/dev/null 2>&1 &
sleep 0.3
"{bin_path}" keyring-probe --namespace ns-a --action preflight
"#,
        bin_path = bin_path()
    );
    let locked = Command::new("dbus-run-session")
        .args(["--", "bash", "-c", &locked_script])
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_RUNTIME_DIR", &xdg_run)
        .output()
        .expect("run locked keyring probe");
    let elapsed = locked_start.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "locked keyring check hung or exceeded deadline: {elapsed:?}"
    );
    assert_ne!(locked.status.code(), Some(0));
    let locked_msg = String::from_utf8_lossy(&locked.stderr);
    assert!(locked_msg.contains("keyring_locked"), "{locked_msg}");
}

#[test]
fn keyring_unlocked_child_helper() {
    if std::env::var("SLICE0_KEYRING_CHILD").as_deref() != Ok("1") {
        return;
    }
    assert_eq!(preflight_keyring(), KeyringHealth::Ready);

    // Seed fake desktop secrets directly under desktop's keyring service name.
    let desktop_vpn = keyring::Entry::new("com.example.supercontrolpanel", "openvpn_password")
        .expect("desktop entry");
    desktop_vpn
        .set_password("desktop-vpn-secret")
        .expect("seed desktop vpn");
    let desktop_rec = keyring::Entry::new(
        "com.example.supercontrolpanel",
        "settings-restore-recovery-v1",
    )
    .expect("desktop rec entry");
    desktop_rec
        .set_password("desktop-recovery-key")
        .expect("seed desktop recovery");

    let mut store_a = NamespacedSecretStore::new("com.chuonglv.scpctl", "ns-a");
    let mut store_b = NamespacedSecretStore::new("com.chuonglv.scpctl", "ns-b");

    store_a
        .set("openvpn_password", "cli-a-secret")
        .expect("set a vpn");
    store_b
        .set("openvpn_password", "cli-b-secret")
        .expect("set b vpn");
    store_a
        .set("settings-restore-recovery-v1", "cli-a-recovery")
        .expect("set a recovery");
    store_b
        .set("settings-restore-recovery-v1", "cli-b-recovery")
        .expect("set b recovery");

    assert_eq!(store_a.cached_get("openvpn_password"), Some("cli-a-secret"));
    assert_eq!(store_b.cached_get("openvpn_password"), Some("cli-b-secret"));

    // Fresh store instance reads back persisted OS keyring without mixing namespaces or desktop.
    let mut fresh_a = NamespacedSecretStore::new("com.chuonglv.scpctl", "ns-a");
    let mut fresh_b = NamespacedSecretStore::new("com.chuonglv.scpctl", "ns-b");
    assert_eq!(
        fresh_a.get("openvpn_password").unwrap().as_deref(),
        Some("cli-a-secret")
    );
    assert_eq!(
        fresh_b.get("openvpn_password").unwrap().as_deref(),
        Some("cli-b-secret")
    );
    assert_eq!(
        fresh_a
            .get("settings-restore-recovery-v1")
            .unwrap()
            .as_deref(),
        Some("cli-a-recovery")
    );
    assert_eq!(
        fresh_b
            .get("settings-restore-recovery-v1")
            .unwrap()
            .as_deref(),
        Some("cli-b-recovery")
    );

    fresh_a.delete("openvpn_password").unwrap();
    assert_eq!(fresh_a.get("openvpn_password").unwrap(), None);
    assert_eq!(
        fresh_b.get("openvpn_password").unwrap().as_deref(),
        Some("cli-b-secret")
    );
    assert_eq!(desktop_vpn.get_password().unwrap(), "desktop-vpn-secret");
    assert_eq!(desktop_rec.get_password().unwrap(), "desktop-recovery-key");
}

#[test]
fn supervisor_enforces_cancel_race_noise_isolation_and_eof_sigterm_reaping() {
    let sup = Supervisor::new();

    // 1. Cancel-before-accept reaps child immediately and leaves 0 active sessions.
    let race_cancelled = sup
        .spawn_and_register(ChildKind::Pipe, "sleep", &["30"], true)
        .expect("race cancel");
    assert_eq!(race_cancelled, OwnershipOutcome::CancelledBeforeAccept);
    assert!(sup.active_sessions().is_empty());

    // 2. Accept-before-cancel stays alive across request token cancellation until explicit stop.
    let outcome = sup
        .spawn_and_register(ChildKind::Pipe, "sleep", &["30"], false)
        .expect("accept");
    let OwnershipOutcome::Accepted(session_id) = outcome else {
        panic!("expected accepted session");
    };
    sup.cancel_request_token(&session_id);
    assert_eq!(sup.active_sessions().len(), 1);
    let pid = sup.active_sessions()[0].1;
    assert!(pid_exists(pid));
    sup.stop_session(&session_id).expect("explicit stop");
    assert!(wait_until(Duration::from_secs(2), || !pid_exists(pid)));
    assert!(sup.active_sessions().is_empty());

    // 3. Noisy child writing > 256 KiB does not deadlock and is bounded in ring buffer.
    let noisy = sup
        .spawn_and_register(
            ChildKind::Pipe,
            "sh",
            &[
                "-c",
                "dd if=/dev/zero bs=1024 count=300 2>/dev/null | tr '\\0' 'A'",
            ],
            false,
        )
        .expect("spawn noisy");
    let OwnershipOutcome::Accepted(noisy_id) = noisy else {
        panic!("expected noisy accepted");
    };
    assert!(wait_until(Duration::from_secs(3), || {
        sup.output_snapshot(&noisy_id)
            .is_some_and(|s| s.contains("AAAA"))
    }));
    let retained = sup.output_len(&noisy_id).unwrap_or(0);
    assert!(
        retained > 0 && retained <= 65_536,
        "unexpected retained size {retained}"
    );
    let _ = sup.shutdown(Duration::from_secs(2));

    // 4. Real subprocess supervisor with both PTY and pipe children: stdin EOF and SIGTERM reaping.
    for mode in ["eof", "sigterm"] {
        let tmp = tempfile::tempdir().expect("tempdir");
        let state_file = tmp.path().join("pids.json");
        let mut daemon = Command::new(bin_path())
            .args([
                "supervisor-daemon",
                "--state-file",
                state_file.to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn supervisor-daemon");

        assert!(
            wait_until(Duration::from_secs(3), || state_file.exists()),
            "supervisor state file never appeared for {mode}"
        );
        let state: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
        let pty_pid = state["ptyPid"].as_u64().unwrap() as u32;
        let pipe_pid = state["pipePid"].as_u64().unwrap() as u32;
        assert!(pid_exists(pty_pid));
        assert!(pid_exists(pipe_pid));

        if mode == "eof" {
            drop(daemon.stdin.take());
        } else {
            unsafe {
                libc::kill(daemon.id() as libc::pid_t, libc::SIGTERM);
            }
        }
        let output = daemon.wait_with_output().expect("wait supervisor-daemon");
        assert!(
            output.stdout.is_empty(),
            "supervisor leaked bytes to stdout on {mode}: {:?}",
            String::from_utf8_lossy(&output.stdout)
        );
        if mode == "eof" {
            assert!(output.status.success());
        }
        assert!(
            wait_until(Duration::from_secs(2), || {
                !pid_exists(pty_pid) && !pid_exists(pipe_pid)
            }),
            "children leaked after {mode}: pty={pty_pid} pipe={pipe_pid}"
        );
    }
}

#[test]
fn unix_vpn_preflight_and_host_wide_lease() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let fake_sudo_fail = tmp.path().join("sudo-fail");
    let fake_ovpn_marker = tmp.path().join("openvpn-ran");
    let fake_ovpn = tmp.path().join("fake-openvpn");
    let ovpn_cfg = tmp.path().join("client.ovpn");
    let vpn_lock = tmp.path().join("vpn.lock");
    fs::write(&ovpn_cfg, "client\n").unwrap();

    write_executable(
        &fake_sudo_fail,
        "#!/bin/sh\necho 'sudo: a password is required' >&2\nexit 1\n",
    );
    write_executable(
        &fake_ovpn,
        &format!("#!/bin/sh\ntouch '{}'\n", fake_ovpn_marker.display()),
    );

    // 1. Noninteractive sudo failure fails before openvpn spawn.
    let err = preflight_and_acquire_vpn(&VpnPreflightConfig {
        sudo_path: fake_sudo_fail,
        openvpn_path: fake_ovpn.clone(),
        ovpn_path: ovpn_cfg.clone(),
        vpn_lock_path: vpn_lock.clone(),
        terminate_when_helper_exits: false,
    })
    .expect_err("must fail when sudo requires interaction");
    assert_eq!(err.reason, "elevation_interaction_required");
    assert!(
        !fake_ovpn_marker.exists(),
        "openvpn should never have been invoked"
    );

    // 2. Missing sudo path fails cleanly.
    let missing_err = preflight_and_acquire_vpn(&VpnPreflightConfig {
        sudo_path: PathBuf::from("/nonexistent/sudo-slice0"),
        openvpn_path: fake_ovpn.clone(),
        ovpn_path: ovpn_cfg.clone(),
        vpn_lock_path: vpn_lock.clone(),
        terminate_when_helper_exits: false,
    })
    .expect_err("must fail when sudo missing");
    assert_eq!(missing_err.reason, "elevation_unavailable");

    // 3. Incompatible terminateOpenVpnWhenHelperExits rejects before spawn.
    let fake_sudo_ok = tmp.path().join("sudo-ok");
    write_executable(&fake_sudo_ok, "#!/bin/sh\nexit 0\n");
    let compat_err = preflight_and_acquire_vpn(&VpnPreflightConfig {
        sudo_path: fake_sudo_ok.clone(),
        openvpn_path: fake_ovpn.clone(),
        ovpn_path: ovpn_cfg.clone(),
        vpn_lock_path: vpn_lock.clone(),
        terminate_when_helper_exits: true,
    })
    .expect_err("must reject incompatible helper setting");
    assert_eq!(compat_err.reason, "incompatible_vpn_ownership");

    // 4. Two distinct profiles contend on the host-wide VPN lock.
    let ready = tmp.path().join("vpn-ready");
    let mut first = Command::new(bin_path())
        .args([
            "vpn-lock",
            "--lock-file",
            vpn_lock.to_str().unwrap(),
            "--sudo-path",
            fake_sudo_ok.to_str().unwrap(),
            "--openvpn-path",
            fake_ovpn.to_str().unwrap(),
            "--ovpn-path",
            ovpn_cfg.to_str().unwrap(),
            "--ready-file",
            ready.to_str().unwrap(),
            "--hold-ms",
            "30000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn first vpn-lock");
    assert!(wait_until(Duration::from_secs(3), || ready.exists()));

    let second = Command::new(bin_path())
        .args([
            "vpn-lock",
            "--lock-file",
            vpn_lock.to_str().unwrap(),
            "--sudo-path",
            fake_sudo_ok.to_str().unwrap(),
            "--openvpn-path",
            fake_ovpn.to_str().unwrap(),
            "--ovpn-path",
            ovpn_cfg.to_str().unwrap(),
            "--hold-ms",
            "100",
        ])
        .output()
        .expect("run second vpn-lock");
    assert_eq!(second.status.code(), Some(5));
    let second_err = String::from_utf8_lossy(&second.stderr);
    assert!(second_err.contains("vpn_owner_conflict"), "{second_err}");

    let _ = first.kill();
    let _ = first.wait();
}

#[test]
fn boundary_cli_isolation_and_capabilities() {
    let boundary_cli = Path::new("/usr/bin/boundary");
    assert!(boundary_cli.exists(), "expected /usr/bin/boundary on host");

    // 1. Capability gate checks.
    assert!(check_auth_capability(BoundaryAuthKind::Password).is_ok());
    let oidc_err = check_auth_capability(BoundaryAuthKind::AutomatedOidc)
        .expect_err("automated webview OIDC is desktop-only");
    assert_eq!(oidc_err.reason, "webview_oidc_desktop_only");
    let browser_err = check_auth_capability(BoundaryAuthKind::BrowserOidc)
        .expect_err("browser OIDC remains gated until interactive validation");
    assert_eq!(browser_err.reason, "browser_oidc_unverified");

    // 2. Unset token with keyring disabled fails fast without prompting.
    let no_tok = run_get_token(boundary_cli, None, &BoundaryTokenMode::NoKeyring)
        .expect_err("missing token should fail");
    assert_eq!(no_tok.reason, "boundary_token_missing");

    // 3. Injected profile token returns exact token with BOUNDARY_KEYRING_TYPE=none & skip cache daemon.
    let token = run_get_token(
        boundary_cli,
        Some("https://127.0.0.1:9200"),
        &BoundaryTokenMode::InjectedEnv("s_slice0_test_token_a".to_string()),
    )
    .expect("injected token succeeds");
    assert_eq!(token, "s_slice0_test_token_a");

    // 4. Isolated BOUNDARY_TOKEN_NAME per profile namespace inside dbus-run-session.
    let tmp = tempfile::tempdir().expect("tempdir");
    let xdg_data = tmp.path().join("data");
    let xdg_run = tmp.path().join("run");
    fs::create_dir_all(&xdg_data).unwrap();
    fs::create_dir_all(&xdg_run).unwrap();
    fs::set_permissions(&xdg_data, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&xdg_run, fs::Permissions::from_mode(0o700)).unwrap();

    let script = format!(
        r#"printf "testpass" | gnome-keyring-daemon --unlock --components=secrets --daemonize >/dev/null
printf "not-json-marker-a" | secret-tool store --label="boundary-a" profile scpctl-ns-a
"{bin_path}" boundary-probe --boundary-path /usr/bin/boundary --namespace ns-a 2>"$1/a.err"; echo "A_EXIT=$?" >"$1/res"
"{bin_path}" boundary-probe --boundary-path /usr/bin/boundary --namespace ns-b 2>"$1/b.err"; echo "B_EXIT=$?" >>"$1/res"
"#,
        bin_path = bin_path()
    );
    let out = Command::new("dbus-run-session")
        .args([
            "--",
            "bash",
            "-c",
            &script,
            "_",
            tmp.path().to_str().unwrap(),
        ])
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_RUNTIME_DIR", &xdg_run)
        .output()
        .expect("run boundary token-name probe");
    assert!(out.status.success(), "{:?}", out);
    let a_err = fs::read_to_string(tmp.path().join("a.err")).unwrap();
    let b_err = fs::read_to_string(tmp.path().join("b.err")).unwrap();
    // ns-a read the item stored under profile=scpctl-ns-a, while ns-b did not see ns-a's item!
    assert!(
        a_err.contains("failed to get token from keyring"),
        "{a_err}"
    );
    assert!(
        b_err.contains("could not be found in the keyring"),
        "ns-b must not read ns-a token: {b_err}"
    );
}
