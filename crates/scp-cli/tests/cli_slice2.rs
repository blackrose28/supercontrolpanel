#[path = "../../scp-core/tests/support/ssh_fixture.rs"]
mod ssh_fixture;

use serde_json::{json, Value};
use ssh_fixture::{free_port, SshdFixture};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const TEST_TOTP_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

fn scpctl_bin() -> &'static str {
    env!("CARGO_BIN_EXE_scpctl")
}

struct TestEnv {
    _tmp: TempDir,
    root: PathBuf,
    desktop: PathBuf,
    keyring_file: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("cli_root");
        let desktop = tmp.path().join("desktop");
        let keyring_file = tmp.path().join("test-keyring.json");
        Self {
            _tmp: tmp,
            root,
            desktop,
            keyring_file,
        }
    }

    fn path(&self) -> &Path {
        self._tmp.path()
    }

    fn write_script(&self, name: &str, body: &str) -> PathBuf {
        let p = self.path().join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn seed_profile(&self, profile: &str, cfg: Value, sync: Value, servers: Value) {
        ssh_fixture::seed_named_profile(&self.root, profile, Some(cfg), sync, servers);
    }

    fn run(&self, profile: Option<&str>, args: &[&str], extra: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(scpctl_bin());
        if let Some(prof) = profile {
            cmd.args([
                "--config-dir",
                self.root.to_str().unwrap(),
                "--profile",
                prof,
            ]);
        }
        cmd.args(args)
            .env("SCPCTL_DESKTOP_CONFIG_DIR", &self.desktop)
            .env("SCPCTL_TEST_KEYRING_FILE", &self.keyring_file)
            .stdin(Stdio::null());
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn spawn(&self, profile: &str, args: &[&str], extra: &[(&str, &str)]) -> Child {
        let mut cmd = Command::new(scpctl_bin());
        cmd.args([
            "--config-dir",
            self.root.to_str().unwrap(),
            "--profile",
            profile,
        ])
        .args(args)
        .env("SCPCTL_DESKTOP_CONFIG_DIR", &self.desktop)
        .env("SCPCTL_TEST_KEYRING_FILE", &self.keyring_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.spawn().unwrap()
    }
}

fn parse_single_json(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.bytes().filter(|b| *b == b'\n').count(),
        1,
        "expected single NDJSON line, got stdout={stdout} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str(stdout.trim()).unwrap()
}

fn wait_port_ready(port: u16, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "port {port} did not open in time"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_port_freed(port: u16, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while TcpListener::bind(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "port {port} stayed bound");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn cli_openvpn_detect_and_boundary_target_search() {
    let env = TestEnv::new();
    let empty_cfg = env.path().join("must_not_exist");
    let absent = Command::new(scpctl_bin())
        .args([
            "--config-dir",
            empty_cfg.to_str().unwrap(),
            "openvpn",
            "detect",
            "--json",
        ])
        .env("PATH", "")
        .env("SCPCTL_OPENVPN_CANDIDATES", "")
        .env("SCPCTL_DESKTOP_CONFIG_DIR", &env.desktop)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(absent.status.code(), Some(6));
    assert!(!empty_cfg.exists());
    assert_eq!(
        parse_single_json(&absent)["error"]["reason"],
        "openvpn_not_found"
    );

    let bin_dir = env.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let fake_ovpn = bin_dir.join("openvpn");
    fs::write(&fake_ovpn, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&fake_ovpn, fs::Permissions::from_mode(0o755)).unwrap();
    let path_env = format!("{}:/usr/bin:/bin", bin_dir.display());
    let found = env.run(
        None,
        &["openvpn", "detect", "--json"],
        &[("PATH", &path_env), ("SCPCTL_OPENVPN_CANDIDATES", "")],
    );
    assert!(found.status.success());
    let found_json = parse_single_json(&found);
    assert_eq!(
        found_json["data"]["executablePath"].as_str().unwrap(),
        fake_ovpn.to_str().unwrap()
    );

    let boundary_cli = env.write_script(
        "fake-boundary",
        r#"#!/usr/bin/env python3
import json, sys
if sys.argv[1:3] == ["targets", "list"]:
    print(json.dumps({"items": [
        {"id": "t1", "name": "Prod-DB", "type": "tcp"},
        {"id": "t2", "name": "Prod-API", "type": "ssh"},
        {"id": "t3", "name": "Prod-Extra", "type": "tcp"},
        {"id": "t4", "name": "Dev-Box", "type": "ssh"}
    ]}))
else:
    sys.exit(2)
"#,
    );
    env.seed_profile(
        "default",
        json!({ "app": { "defaultTerminalCols": 120, "defaultTerminalRows": 36 }, "boundary": { "addr": "https://b.test" } }),
        json!({ "boundaryCliPath": boundary_cli }),
        json!({ "servers": [] }),
    );
    let searched = env.run(
        Some("default"),
        &[
            "boundary", "targets", "search", "--filter", "prod", "--limit", "2", "--json",
        ],
        &[],
    );
    assert!(searched.status.success());
    let rows = parse_single_json(&searched)["data"]["targets"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "t1");
    assert_eq!(rows[1]["id"], "t2");
}

#[test]
fn cli_boundary_auth_modes() {
    let env = TestEnv::new();
    let boundary_cli = env.write_script(
        "fake-boundary-auth",
        "#!/usr/bin/env python3\nimport json\nprint(json.dumps({\"attributes\": {\"token\": \"tbat_cli_pw_tok\"}}))\n",
    );
    let sync = json!({ "boundaryCliPath": boundary_cli });
    env.seed_profile(
        "oidc-auto",
        json!({
            "app": { "defaultTerminalCols": 120, "defaultTerminalRows": 36 },
            "boundary": { "addr": "https://b.test", "authMethodId": "amoidc_1", "authKind": "oidc", "oidcLoginMode": "automated" }
        }),
        sync.clone(),
        json!({ "servers": [] }),
    );
    let auto_out = env.run(Some("oidc-auto"), &["boundary", "auth", "--json"], &[]);
    assert_eq!(auto_out.status.code(), Some(4));
    assert_eq!(
        parse_single_json(&auto_out)["error"]["reason"],
        "webview_oidc_desktop_only"
    );

    env.seed_profile(
        "oidc-browser",
        json!({
            "app": { "defaultTerminalCols": 120, "defaultTerminalRows": 36 },
            "boundary": { "addr": "https://b.test", "authMethodId": "amoidc_1", "authKind": "oidc", "oidcLoginMode": "browser" }
        }),
        sync.clone(),
        json!({ "servers": [] }),
    );
    let br_out = env.run(
        Some("oidc-browser"),
        &["boundary", "auth", "--interactive", "--json"],
        &[],
    );
    assert_eq!(br_out.status.code(), Some(4));
    assert_eq!(
        parse_single_json(&br_out)["error"]["reason"],
        "browser_oidc_unverified"
    );

    env.seed_profile(
        "pw",
        json!({
            "app": { "defaultTerminalCols": 120, "defaultTerminalRows": 36 },
            "boundary": {
                "addr": "https://b.test",
                "authMethodId": "ampw_1",
                "loginName": "alice",
                "password": "secret-pw-must-not-leak",
                "authKind": "password"
            }
        }),
        sync,
        json!({ "servers": [] }),
    );
    let pw_out = env.run(Some("pw"), &["boundary", "auth", "--json"], &[]);
    assert!(
        pw_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&pw_out.stderr)
    );
    let body = parse_single_json(&pw_out);
    assert_eq!(body["data"]["state"], "authenticated");
    assert_eq!(body["data"]["boundaryTokenSource"], "app");
    let raw_all = format!(
        "{}{}",
        String::from_utf8_lossy(&pw_out.stdout),
        String::from_utf8_lossy(&pw_out.stderr)
    );
    assert!(!raw_all.contains("secret-pw-must-not-leak"));
    assert!(!raw_all.contains("tbat_cli_pw_tok"));
    let kr_text = fs::read_to_string(&env.keyring_file).unwrap();
    assert!(kr_text.contains("tbat_cli_pw_tok"));
}

#[test]
fn cli_one_shot_scp_upload_and_download_with_temporary_boundary_tunnel() {
    let fx = SshdFixture::start();
    let b_port = free_port();
    fx.write_known_hosts(&fx.known_hosts, &[fx.port, b_port]);

    let env = TestEnv::new();
    env.seed_profile(
        "default",
        json!({ "app": { "defaultTerminalCols": 120, "defaultTerminalRows": 36 }, "boundary": { "addr": "https://b.test" } }),
        json!({ "boundaryCliPath": fx.boundary_script }),
        json!({
            "servers": [{
                "id": "srv-b",
                "name": "BoundaryBox",
                "host": "127.0.0.1",
                "os": "linux",
                "connectionMethod": "boundary",
                "sshPort": 22,
                "sshUser": fx.user,
                "boundaryTargetId": "ttcp_ok",
                "boundaryListenPort": b_port
            }]
        }),
    );

    let local_src = env.path().join("payload.bin");
    fs::write(&local_src, b"cli-slice2-boundary-scp-999").unwrap();
    let kh_str = fx.known_hosts.to_str().unwrap();
    let id_str = fx.user_key.to_str().unwrap();
    let ssh_env = [
        ("SCPCTL_SSH_KNOWN_HOSTS", kh_str),
        ("SCPCTL_SSH_IDENTITY_FILE", id_str),
    ];

    let up = env.run(
        Some("default"),
        &[
            "scp",
            "upload",
            "--server-id",
            "srv-b",
            "--local",
            local_src.to_str().unwrap(),
            "--remote",
            fx.remote_sandbox.to_str().unwrap(),
            "--json",
            "--non-interactive",
        ],
        &ssh_env,
    );
    assert!(
        up.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&up.stderr)
    );
    let up_json = parse_single_json(&up);
    assert_eq!(up_json["data"]["state"], "success");
    let uploaded = fx.remote_sandbox.join("payload.bin");
    assert_eq!(fs::read(&uploaded).unwrap(), b"cli-slice2-boundary-scp-999");
    wait_port_freed(b_port, Duration::from_secs(2));

    let dl_dest = env.path().join("downloaded.bin");
    let dl = env.run(
        Some("default"),
        &[
            "scp",
            "download",
            "--server-id",
            "srv-b",
            "--remote",
            uploaded.to_str().unwrap(),
            "--local",
            dl_dest.to_str().unwrap(),
            "--json",
            "--non-interactive",
        ],
        &ssh_env,
    );
    assert!(
        dl.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&dl.stdout),
        String::from_utf8_lossy(&dl.stderr)
    );
    assert_eq!(fs::read(&dl_dest).unwrap(), b"cli-slice2-boundary-scp-999");
    wait_port_freed(b_port, Duration::from_secs(2));

    // Existing file conflicts unless --overwrite is explicitly set.
    let conflict = env.run(
        Some("default"),
        &[
            "scp",
            "download",
            "--server-id",
            "srv-b",
            "--remote",
            uploaded.to_str().unwrap(),
            "--local",
            dl_dest.to_str().unwrap(),
            "--json",
            "--non-interactive",
        ],
        &ssh_env,
    );
    assert_eq!(conflict.status.code(), Some(5));
    assert_eq!(
        parse_single_json(&conflict)["error"]["reason"],
        "destination_exists"
    );

    let overwrite = env.run(
        Some("default"),
        &[
            "scp",
            "download",
            "--server-id",
            "srv-b",
            "--remote",
            uploaded.to_str().unwrap(),
            "--local",
            dl_dest.to_str().unwrap(),
            "--overwrite",
            "--json",
            "--non-interactive",
        ],
        &ssh_env,
    );
    assert!(overwrite.status.success());

    // Untrusted known_hosts is rejected strictly.
    let empty_kh = env.path().join("empty_kh");
    fs::write(&empty_kh, "").unwrap();
    let untrusted = env.run(
        Some("default"),
        &[
            "scp",
            "upload",
            "--server-id",
            "srv-b",
            "--local",
            local_src.to_str().unwrap(),
            "--remote",
            fx.remote_sandbox.to_str().unwrap(),
            "--json",
            "--non-interactive",
        ],
        &[
            ("SCPCTL_SSH_KNOWN_HOSTS", empty_kh.to_str().unwrap()),
            ("SCPCTL_SSH_IDENTITY_FILE", id_str),
        ],
    );
    assert_eq!(untrusted.status.code(), Some(4));
    assert_eq!(
        parse_single_json(&untrusted)["error"]["reason"],
        "host_key_untrusted"
    );
}

#[cfg(unix)]
#[test]
fn cli_foreground_tunnel_caddy_openvpn_lifecycle_and_signals() {
    let env = TestEnv::new();
    for bad in [
        vec![
            "server",
            "tunnel",
            "start",
            "--server-id",
            "srv-b",
            "--json",
        ],
        vec!["caddy", "start", "--json"],
        vec!["openvpn", "start", "--json"],
    ] {
        let out = env.run(Some("missing"), &bad, &[]);
        assert_eq!(out.status.code(), Some(2), "args={bad:?}");
        assert_eq!(
            parse_single_json(&out)["error"]["reason"],
            "foreground_required"
        );
    }

    // 1. Foreground Boundary tunnel + owner lock + SIGTERM graceful exit (code 0).
    let fx = SshdFixture::start();
    let t_port = free_port();
    let caddy_port = free_port();
    let site_dir = env.path().join("site");
    fs::create_dir_all(&site_dir).unwrap();
    fs::write(
        site_dir.join("Caddyfile"),
        format!(":{caddy_port} {{\n  respond \"ok\" 200\n}}\n"),
    )
    .unwrap();
    let caddy_bin = env.write_script(
        "fake-caddy",
        &format!(
            "#!/usr/bin/env python3\nimport signal, socket, sys, time\nsignal.signal(signal.SIGTERM, lambda *_: sys.exit(0))\ns = socket.socket()\ns.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\ns.bind(('127.0.0.1', {caddy_port}))\ns.listen(4)\nprint('INFO serving initial configuration', flush=True)\ntime.sleep(30)\n"
        ),
    );
    let ovpn_file = env.path().join("c.ovpn");
    fs::write(&ovpn_file, "client\nremote 127.0.0.1 1194\n").unwrap();
    let sudo_bin = env.write_script(
        "fake-sudo",
        "#!/usr/bin/env python3\nimport os, sys\nargs = sys.argv[1:]\nif args == ['-n', 'true']:\n    sys.exit(0)\nif args and args[0] == '-n':\n    args = args[1:]\nos.execvp(args[0], args)\n",
    );
    let vpn_bin = env.write_script(
        "fake-ovpn",
        "#!/usr/bin/env python3\nimport signal, sys, time\nsignal.signal(signal.SIGTERM, lambda *_: sys.exit(0))\nprint('Initialization Sequence Completed', flush=True)\ntime.sleep(30)\n",
    );
    let shared_vpn_lock = env.path().join("shared-vpn.lock");

    env.seed_profile(
        "default",
        json!({
            "app": { "defaultTerminalCols": 120, "defaultTerminalRows": 36 },
            "boundary": { "addr": "https://b.test" },
            "openvpn": {
                "executablePath": vpn_bin,
                "ovpnPath": ovpn_file,
                "username": "vpn-user",
                "password": "vpn-password",
                "otpSecret": TEST_TOTP_SECRET
            }
        }),
        json!({ "boundaryCliPath": fx.boundary_script, "caddyFolder": site_dir }),
        json!({
            "servers": [{
                "id": "srv-b",
                "name": "BoundaryBox",
                "host": "127.0.0.1",
                "os": "linux",
                "connectionMethod": "boundary",
                "sshPort": 22,
                "sshUser": fx.user,
                "boundaryTargetId": "ttcp_ok",
                "boundaryListenPort": t_port
            }]
        }),
    );

    let tun_child = env.spawn(
        "default",
        &[
            "server",
            "tunnel",
            "start",
            "--server-id",
            "srv-b",
            "--foreground",
            "--json",
        ],
        &[],
    );
    wait_port_ready(t_port, Duration::from_secs(5));
    let busy_status = env.run(Some("default"), &["status", "--json"], &[]);
    assert_eq!(busy_status.status.code(), Some(5));
    assert_eq!(
        parse_single_json(&busy_status)["error"]["reason"],
        "profile_owner_conflict"
    );

    unsafe { libc::kill(tun_child.id() as libc::pid_t, libc::SIGTERM) };
    let tun_out = tun_child.wait_with_output().unwrap();
    assert!(
        tun_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&tun_out.stderr)
    );
    let tun_json = parse_single_json(&tun_out);
    assert_eq!(tun_json["ok"], true);
    assert_eq!(tun_json["data"]["state"], "stopped");
    wait_port_freed(t_port, Duration::from_secs(2));

    // 2. Foreground Caddy + SIGINT (exit 130 + INTERRUPTED).
    let caddy_child = env.spawn(
        "default",
        &["caddy", "start", "--foreground", "--json"],
        &[("SCPCTL_CADDY_PATH", caddy_bin.to_str().unwrap())],
    );
    wait_port_ready(caddy_port, Duration::from_secs(5));
    unsafe { libc::kill(caddy_child.id() as libc::pid_t, libc::SIGINT) };
    let caddy_out = caddy_child.wait_with_output().unwrap();
    assert_eq!(caddy_out.status.code(), Some(130));
    let caddy_json = parse_single_json(&caddy_out);
    assert_eq!(caddy_json["ok"], false);
    assert_eq!(caddy_json["error"]["code"], "INTERRUPTED");
    wait_port_freed(caddy_port, Duration::from_secs(2));

    // 3. Foreground OpenVPN + cross-profile vpn.lock conflict + SIGTERM exit 0.
    env.seed_profile(
        "second",
        json!({
            "app": { "defaultTerminalCols": 120, "defaultTerminalRows": 36 },
            "openvpn": {
                "executablePath": vpn_bin,
                "ovpnPath": ovpn_file,
                "username": "vpn-user-2",
                "password": "vpn-password-2"
            }
        }),
        json!({}),
        json!({ "servers": [] }),
    );
    let vpn_extra = [
        ("SCPCTL_SUDO_PATH", sudo_bin.to_str().unwrap()),
        ("SCPCTL_VPN_LOCK_PATH", shared_vpn_lock.to_str().unwrap()),
    ];
    let mut vpn_child = env.spawn(
        "default",
        &["openvpn", "start", "--foreground", "--json"],
        &vpn_extra,
    );
    let stderr_reader = BufReader::new(vpn_child.stderr.take().unwrap());
    let mut ready_seen = false;
    for line in stderr_reader.lines().map_while(Result::ok) {
        if line.contains("Started openvpn-connect") {
            ready_seen = true;
            break;
        }
    }
    assert!(ready_seen, "openvpn foreground never announced start");

    let conflict_vpn = env.run(
        Some("second"),
        &["openvpn", "start", "--foreground", "--json"],
        &vpn_extra,
    );
    assert_eq!(conflict_vpn.status.code(), Some(5));
    assert_eq!(
        parse_single_json(&conflict_vpn)["error"]["reason"],
        "vpn_owner_conflict"
    );

    unsafe { libc::kill(vpn_child.id() as libc::pid_t, libc::SIGTERM) };
    let vpn_out = vpn_child.wait_with_output().unwrap();
    assert!(vpn_out.status.success());
    assert_eq!(parse_single_json(&vpn_out)["data"]["state"], "stopped");
}

#[cfg(unix)]
#[test]
fn cli_interactive_ssh_tty_gate_and_pty_session() {
    use std::io::Write;
    use std::os::unix::io::{AsRawFd, FromRawFd};
    use std::os::unix::process::CommandExt;

    let fx = SshdFixture::start();
    let b_port = free_port();
    fx.write_known_hosts(&fx.known_hosts, &[fx.port, b_port]);

    let env = TestEnv::new();
    env.seed_profile(
        "default",
        json!({ "app": { "defaultTerminalCols": 120, "defaultTerminalRows": 36 }, "boundary": { "addr": "https://b.test" } }),
        json!({ "boundaryCliPath": fx.boundary_script }),
        json!({
            "servers": [{
                "id": "srv-b",
                "name": "BoundaryBox",
                "host": "127.0.0.1",
                "os": "linux",
                "connectionMethod": "boundary",
                "sshPort": 22,
                "sshUser": fx.user,
                "boundaryTargetId": "ttcp_ok",
                "boundaryListenPort": b_port
            }]
        }),
    );

    // Argument gates: missing --interactive, --json, or --non-interactive -> exit 2.
    for bad in [
        vec!["server", "ssh", "--server-id", "srv-b"],
        vec![
            "server",
            "ssh",
            "--server-id",
            "srv-b",
            "--interactive",
            "--json",
        ],
        vec![
            "server",
            "ssh",
            "--server-id",
            "srv-b",
            "--interactive",
            "--non-interactive",
        ],
    ] {
        let out = env.run(Some("default"), &bad, &[]);
        assert_eq!(out.status.code(), Some(2), "args={bad:?}");
    }

    // Non-TTY stdio is rejected before spawn -> exit 4 interactive_tty_required.
    let no_tty = env.run(
        Some("default"),
        &["server", "ssh", "--server-id", "srv-b", "--interactive"],
        &[],
    );
    assert_eq!(no_tty.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&no_tty.stderr).contains("interactive terminal"),
        "stderr={}",
        String::from_utf8_lossy(&no_tty.stderr)
    );

    // Real PTY execution via Boundary temporary tunnel.
    let mut master_fd = -1;
    let mut slave_fd = -1;
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    let mut orig_termios = unsafe { std::mem::zeroed::<libc::termios>() };
    assert_eq!(unsafe { libc::tcgetattr(slave_fd, &mut orig_termios) }, 0);

    let mut master = unsafe { fs::File::from_raw_fd(master_fd) };
    let slave_in = unsafe { fs::File::from_raw_fd(slave_fd) };
    let slave_out = slave_in.try_clone().unwrap();
    let slave_err = slave_in.try_clone().unwrap();

    let mut cmd = Command::new(scpctl_bin());
    cmd.args([
        "--config-dir",
        env.root.to_str().unwrap(),
        "--profile",
        "default",
        "server",
        "ssh",
        "--server-id",
        "srv-b",
        "--interactive",
    ])
    .env("SCPCTL_DESKTOP_CONFIG_DIR", &env.desktop)
    .env("SCPCTL_TEST_KEYRING_FILE", &env.keyring_file)
    .env("SCPCTL_SSH_KNOWN_HOSTS", &fx.known_hosts)
    .env("SCPCTL_SSH_IDENTITY_FILE", &fx.user_key)
    .stdin(Stdio::from(slave_in))
    .stdout(Stdio::from(slave_out))
    .stderr(Stdio::from(slave_err));
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    drop(cmd);

    master.write_all(b"echo READY_SSH_OK\nexit\n").unwrap();
    master.flush().unwrap();

    let reader_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = master.read_to_end(&mut buf);
        let mut after_termios = unsafe { std::mem::zeroed::<libc::termios>() };
        let rc = unsafe { libc::tcgetattr(master.as_raw_fd(), &mut after_termios) };
        (
            String::from_utf8_lossy(&buf).into_owned(),
            rc,
            after_termios.c_lflag,
        )
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        assert!(Instant::now() < deadline, "interactive ssh child timed out");
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(status.success(), "ssh exited with {:?}", status.code());
    wait_port_freed(b_port, Duration::from_secs(2));
    let (pty_text, after_rc, after_lflag) = reader_thread.join().unwrap();
    assert!(
        pty_text.contains("READY_SSH_OK"),
        "expected PTY output to contain READY_SSH_OK, got: {pty_text:?}"
    );
    if after_rc == 0 {
        assert_eq!(after_lflag & libc::ECHO, orig_termios.c_lflag & libc::ECHO);
    }
}

#[test]
fn cli_schema_and_capabilities_slice2_surface() {
    let env = TestEnv::new();
    let schema = env.run(None, &["schema", "--json"], &[]);
    assert!(schema.status.success());
    let sbody = parse_single_json(&schema);
    let cmds = sbody["data"]["commands"].as_array().unwrap();
    assert_eq!(cmds.len(), 31);
    for name in [
        "openvpn.detect",
        "openvpn.start",
        "boundary.targets.search",
        "boundary.auth",
        "caddy.start",
        "server.tunnel.start",
        "server.ssh",
        "scp.upload",
        "scp.download",
    ] {
        assert!(
            cmds.iter().any(|c| c["command"] == name),
            "missing CLI schema {name}"
        );
    }
    let ops = sbody["data"]["mcpOperations"].as_array().unwrap();
    assert_eq!(ops.len(), 45);
    assert_eq!(ops.iter().filter(|o| o["available"] == true).count(), 41);
    assert_eq!(ops.iter().filter(|o| o["available"] == false).count(), 4);
    assert_eq!(sbody["data"]["mcpResources"].as_array().unwrap().len(), 6);

    let caps = env.run(None, &["capabilities", "--no-profile", "--json"], &[]);
    assert!(caps.status.success());
    let cbody = parse_single_json(&caps);
    assert_eq!(
        cbody["data"]["availableOperations"]
            .as_array()
            .unwrap()
            .len(),
        11
    );
    assert!(cbody["data"]["runtimeFamilies"].is_object());
    assert_eq!(cbody["data"]["runtimeFamilies"]["backup"], "cli_local");
    assert_eq!(
        cbody["data"]["runtimeFamilies"]["sftp"],
        "stdio_persistent_browser"
    );
    assert_eq!(cbody["data"]["unavailableFamilies"], json!({}));
}
