use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

struct HeadlessEnv(TempDir);

impl HeadlessEnv {
    fn new() -> Self {
        Self(TempDir::new().unwrap())
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    fn base_cmd(&self, args: &[&str], extra: &[(&str, &str)]) -> Command {
        let root: PathBuf = self.path().join("cli-root");
        let desktop: PathBuf = self.path().join("desktop");
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_scpctl"));
        cmd.args([
            "--config-dir",
            root.to_str().unwrap(),
            "--profile",
            "headless",
        ])
        .args(args)
        .env_remove("SCPCTL_TEST_KEYRING_FILE")
        .env("SCPCTL_DESKTOP_CONFIG_DIR", desktop)
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/scpctl-no-dbus",
        )
        .env("DISPLAY", "")
        .env("WAYLAND_DISPLAY", "");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd
    }

    fn exec(&self, args: &[&str]) -> Output {
        self.base_cmd(args, &[])
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn exec_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Output {
        self.base_cmd(args, extra)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn pipe_stdin(&self, args: &[&str], payload: &[u8]) -> Output {
        let mut child = self
            .base_cmd(args, &[])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.as_mut().unwrap().write_all(payload).unwrap();
        child.wait_with_output().unwrap()
    }
}

fn parse_single_json(out: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "expected single JSON line on stdout, got stdout={stdout:?} stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_str(lines[0]).unwrap()
}

fn assert_capability_unavailable(out: &Output, expected_reason: Option<&str>) {
    assert_eq!(
        out.status.code(),
        Some(4),
        "expected exit 4 CAPABILITY_UNAVAILABLE, stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let body = parse_single_json(out);
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"]["code"], "CAPABILITY_UNAVAILABLE");
    assert_eq!(body["error"]["outcome"], "not_started");
    if let Some(reason) = expected_reason {
        assert_eq!(body["error"]["reason"], reason);
    }
}

fn verify_headless_capabilities(env: &HeadlessEnv) {
    let caps = env.exec(&["capabilities", "--json", "--non-interactive"]);
    assert!(
        caps.status.success(),
        "capabilities stderr={}",
        String::from_utf8_lossy(&caps.stderr)
    );
    let caps_json = parse_single_json(&caps);
    assert_eq!(caps_json["ok"], true);
    if cfg!(target_os = "linux") {
        assert_eq!(caps_json["data"]["keyring"]["state"], "unavailable");
    }
}

fn verify_non_secret_ops(env: &HeadlessEnv, repo_root: &Path) {
    let server_payload = serde_json::to_vec(&json!({
        "id": "srv-headless",
        "name": "Headless SSH Box",
        "host": "10.20.30.40",
        "os": "linux",
        "connectionMethod": "ssh",
        "sshPort": 22,
        "sshUser": "ops"
    }))
    .unwrap();
    let preview = env.pipe_stdin(
        &[
            "server",
            "upsert",
            "--stdin",
            "--preview",
            "--json",
            "--non-interactive",
        ],
        &server_payload,
    );
    assert!(preview.status.success());
    let rev = parse_single_json(&preview)["data"]["revision"]
        .as_str()
        .unwrap()
        .to_string();

    let commit = env.pipe_stdin(
        &[
            "server",
            "upsert",
            "--stdin",
            "--expected-revision",
            &rev,
            "--json",
            "--non-interactive",
        ],
        &server_payload,
    );
    assert!(commit.status.success());

    let listed = env.exec(&["server", "list", "--json", "--non-interactive"]);
    assert!(listed.status.success());
    assert_eq!(
        parse_single_json(&listed)["data"]["servers"][0]["id"],
        "srv-headless"
    );

    let got = env.exec(&[
        "server",
        "get",
        "--server-id",
        "srv-headless",
        "--json",
        "--non-interactive",
    ]);
    assert!(got.status.success());
    assert_eq!(parse_single_json(&got)["data"]["host"], "10.20.30.40");

    let ovpn_cfg = env.path().join("client.ovpn");
    let fake_ovpn = env.path().join("fake-openvpn");
    fs::write(&ovpn_cfg, "client\nremote 127.0.0.1 1194\n").unwrap();
    fs::write(&fake_ovpn, "#!/bin/sh\nexit 0\n").unwrap();
    let patch_bytes = serde_json::to_vec(&json!({
        "openvpn": {
            "executablePath": fake_ovpn,
            "ovpnPath": ovpn_cfg,
            "username": "vpn-user"
        }
    }))
    .unwrap();
    assert!(env
        .pipe_stdin(
            &["config", "patch", "--stdin", "--json", "--non-interactive"],
            &patch_bytes
        )
        .status
        .success());
    assert!(env
        .exec(&["settings", "show", "--json", "--non-interactive"])
        .status
        .success());

    let sync_push = env.exec(&[
        "config",
        "sync",
        "push",
        "--repo-root",
        repo_root.to_str().unwrap(),
        "--json",
        "--non-interactive",
    ]);
    assert!(sync_push.status.success());
    assert!(repo_root.join("app-config/servers.json").is_file());
}

fn verify_keyring_unavailable_errors(env: &HeadlessEnv, export_path: &Path) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let cfg_secret = env.pipe_stdin(
        &[
            "config",
            "secret",
            "set",
            "--key",
            "openvpn-password",
            "--secret-stdin",
            "--json",
            "--non-interactive",
        ],
        b"secret-pw\n",
    );
    assert_capability_unavailable(&cfg_secret, Some("keyring_unavailable"));
    assert_eq!(
        parse_single_json(&cfg_secret)["error"]["nextAction"],
        "configure_credentials"
    );

    let srv_secret = env.pipe_stdin(
        &[
            "server",
            "credentials",
            "set",
            "--server-id",
            "srv-headless",
            "--secret-stdin",
            "--json",
            "--non-interactive",
        ],
        b"ssh-pw\n",
    );
    assert_capability_unavailable(&srv_secret, Some("keyring_unavailable"));
    assert_eq!(
        parse_single_json(&srv_secret)["error"]["nextAction"],
        "configure_credentials"
    );

    let exported = env.pipe_stdin(
        &[
            "settings",
            "export",
            "--output",
            export_path.to_str().unwrap(),
            "--passphrase-stdin",
            "--json",
            "--non-interactive",
        ],
        b"backup-passphrase\n",
    );
    assert_eq!(exported.status.code(), Some(1));
    let exp_body = parse_single_json(&exported);
    assert_eq!(exp_body["ok"], false);
    assert_eq!(exp_body["error"]["code"], "OPERATION_FAILED");
    assert_eq!(exp_body["error"]["reason"], "backup_snapshot_failed");
    assert_eq!(exp_body["error"]["nextAction"], "unlock_keyring_and_retry");
    assert_eq!(exp_body["error"]["outcome"], "not_started");
    assert!(!export_path.exists());
}

fn verify_interactive_and_elevation_gates(env: &HeadlessEnv, export_path: &Path) {
    let vpn_lock = env.path().join("vpn.lock");
    let vpn_out = env.exec_env(
        &[
            "openvpn",
            "start",
            "--foreground",
            "--json",
            "--non-interactive",
        ],
        &[
            ("SCPCTL_SUDO_PATH", "/bin/false"),
            ("SCPCTL_VPN_LOCK_PATH", vpn_lock.to_str().unwrap()),
        ],
    );
    assert_capability_unavailable(&vpn_out, None);

    let rdp_srv = serde_json::to_vec(&json!({
        "id": "srv-win",
        "name": "Win Box",
        "host": "10.20.30.41",
        "os": "windows",
        "connectionMethod": "ssh",
        "sshPort": 22,
        "sshUser": "Administrator"
    }))
    .unwrap();
    assert!(env
        .pipe_stdin(
            &["server", "upsert", "--stdin", "--json", "--non-interactive"],
            &rdp_srv
        )
        .status
        .success());

    let rdp_nonint = env.exec(&[
        "server",
        "rdp",
        "--server-id",
        "srv-win",
        "--non-interactive",
        "--json",
    ]);
    assert_capability_unavailable(&rdp_nonint, Some("interactive_tty_required"));

    let rdp_no_tty = env.exec(&["server", "rdp", "--server-id", "srv-win", "--interactive"]);
    assert_eq!(rdp_no_tty.status.code(), Some(4));

    let import_nonint = env.exec(&[
        "settings",
        "import",
        "--input",
        export_path.to_str().unwrap(),
        "--apply",
        "--non-interactive",
    ]);
    assert_eq!(import_nonint.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&import_nonint.stderr).contains("CAPABILITY_UNAVAILABLE"),
        "expected CAPABILITY_UNAVAILABLE on stderr, got: {}",
        String::from_utf8_lossy(&import_nonint.stderr)
    );
}

#[test]
fn headless_no_secret_service_contract() {
    let env = HeadlessEnv::new();
    let repo_root = env.path().join("sync-repo");
    let export_path = env.path().join("headless.scpbak");
    fs::create_dir_all(&repo_root).unwrap();

    verify_headless_capabilities(&env);
    verify_non_secret_ops(&env, &repo_root);
    verify_keyring_unavailable_errors(&env, &export_path);
    verify_interactive_and_elevation_gates(&env, &export_path);
}

#[test]
fn release_assets_and_completion_bundle_exist() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    for rel in [
        "scripts/package-scpctl.mjs",
        "scripts/install-scpctl.sh",
        "Dockerfile.cli",
        ".github/workflows/standalone-cli.yml",
        "examples/standalone-cli/shell-agent-workflow.mjs",
        "examples/standalone-cli/stdio-mcp-agent-workflow.mjs",
        "docs/mcp/STANDALONE_CLI_AGENT_GUIDE.md",
    ] {
        let p = repo_root.join(rel);
        assert!(
            p.is_file(),
            "missing Slice 5 release asset: {}",
            p.display()
        );
        let meta = fs::metadata(&p).unwrap();
        assert!(meta.len() > 0, "empty Slice 5 asset: {}", p.display());
    }
}
