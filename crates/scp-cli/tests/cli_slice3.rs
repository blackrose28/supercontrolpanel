use scp_core::backup::{transaction, Changes, SecretStore};
use scp_core::models::{KEYRING_OPENVPN_OTP_SECRET, KEYRING_OPENVPN_PASSWORD, KEYRING_SERVICE};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

fn scpctl_bin() -> &'static str {
    env!("CARGO_BIN_EXE_scpctl")
}

struct Slice3Env(TempDir);

impl Slice3Env {
    fn new() -> Self {
        Self(TempDir::new().unwrap())
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    fn root(&self) -> PathBuf {
        self.path().join("cli_root")
    }

    fn desktop(&self) -> PathBuf {
        self.path().join("desktop")
    }

    fn keyring_file(&self) -> PathBuf {
        self.path().join("test-keyring.json")
    }

    fn profile_dir(&self, profile: &str) -> PathBuf {
        self.root().join("profiles").join(profile)
    }

    fn run_with_stdin(
        &self,
        profile: Option<&str>,
        args: &[&str],
        stdin_bytes: Option<&[u8]>,
        extra: &[(&str, &str)],
    ) -> Output {
        let mut cmd = Command::new(scpctl_bin());
        if let Some(prof) = profile {
            cmd.args([
                "--config-dir",
                self.root().to_str().unwrap(),
                "--profile",
                prof,
            ]);
        }
        cmd.args(args)
            .env("SCPCTL_DESKTOP_CONFIG_DIR", self.desktop())
            .env("SCPCTL_TEST_KEYRING_FILE", self.keyring_file());
        for (k, v) in extra {
            cmd.env(k, v);
        }
        if let Some(bytes) = stdin_bytes {
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = cmd.spawn().unwrap();
            {
                let stdin = child.stdin.as_mut().unwrap();
                stdin.write_all(bytes).unwrap();
            }
            child.wait_with_output().unwrap()
        } else {
            cmd.stdin(Stdio::null());
            cmd.output().unwrap()
        }
    }

    fn run(&self, profile: Option<&str>, args: &[&str], extra: &[(&str, &str)]) -> Output {
        self.run_with_stdin(profile, args, None, extra)
    }

    fn read_keyring_map(&self) -> BTreeMap<String, String> {
        let raw = fs::read_to_string(self.keyring_file()).unwrap_or_else(|_| "{}".into());
        let nested: BTreeMap<String, BTreeMap<String, String>> =
            serde_json::from_str(&raw).unwrap();
        nested.get(KEYRING_SERVICE).cloned().unwrap_or_default()
    }
}

fn parse_single_json(out: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "expected single JSON line on stdout, got: {stdout:?}\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_str(lines[0]).unwrap()
}

fn keyring_entry_key(account: &str) -> String {
    account.to_string()
}

struct TestKeyringStore {
    file: PathBuf,
    namespace: String,
}

impl SecretStore for TestKeyringStore {
    fn read(&mut self, account: &str) -> Result<Option<String>, String> {
        let raw = fs::read_to_string(&self.file).unwrap_or_else(|_| "{}".into());
        let nested: BTreeMap<String, BTreeMap<String, String>> =
            serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        Ok(nested
            .get(KEYRING_SERVICE)
            .and_then(|svc| svc.get(&format!("profile:{}:{account}", self.namespace)))
            .cloned())
    }

    fn write(&mut self, account: &str, value: Option<&str>) -> Result<(), String> {
        let raw = fs::read_to_string(&self.file).unwrap_or_else(|_| "{}".into());
        let mut nested: BTreeMap<String, BTreeMap<String, String>> =
            serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let svc = nested.entry(KEYRING_SERVICE.to_string()).or_default();
        let full = format!("profile:{}:{account}", self.namespace);
        match value {
            Some(v) => {
                svc.insert(full, v.to_string());
            }
            None => {
                svc.remove(&full);
            }
        }
        fs::write(&self.file, serde_json::to_string_pretty(&nested).unwrap())
            .map_err(|e| e.to_string())
    }
}

#[test]
fn completion_generates_scripts_without_opening_profile_or_exposing_secret_flags() {
    let env = Slice3Env::new();
    let cfg_root = env.path().join("never_created");

    for shell in ["bash", "zsh", "fish", "powershell"] {
        let out = env.run(
            None,
            &[
                "--config-dir",
                cfg_root.to_str().unwrap(),
                "--profile",
                "agent",
                "completion",
                shell,
            ],
            &[],
        );
        assert!(
            out.status.success(),
            "completion {shell} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("scpctl"), "{shell}: {text}");
        assert!(text.contains("completion"), "{shell}: {text}");
        assert!(text.contains("settings"), "{shell}: {text}");
        for forbidden in [
            "--password",
            "--secret",
            "--passphrase",
            "--recover-identity",
        ] {
            assert!(
                !text
                    .lines()
                    .any(|l| l.split_whitespace().any(|w| w == forbidden)),
                "{shell} completion leaked unsafe flag {forbidden}"
            );
        }
    }

    let flag_form = env.run(None, &["completion", "--shell", "bash"], &[]);
    assert!(flag_form.status.success());

    let bad_shell = env.run(None, &["completion", "nushell"], &[]);
    assert_eq!(bad_shell.status.code(), Some(2));

    let bad_json = env.run(None, &["completion", "bash", "--json"], &[]);
    assert_eq!(bad_json.status.code(), Some(2));

    assert!(
        !cfg_root.exists(),
        "completion must never create config or profile directories"
    );
    assert!(
        !env.keyring_file().exists(),
        "completion must never unlock or touch OS keyring"
    );
}

#[test]
fn unsafe_secret_flags_and_conflicting_inputs_fail_fast_with_exit_2() {
    let env = Slice3Env::new();
    let pw_file = env.path().join("pw.txt");
    fs::write(&pw_file, "secret-123\n").unwrap();

    for bad_args in [
        vec![
            "config",
            "secret",
            "set",
            "--key",
            "openvpn-password",
            "--password",
            "inline",
            "--json",
        ],
        vec![
            "config",
            "secret",
            "set",
            "--key",
            "openvpn-password",
            "--secret=inline",
            "--json",
        ],
        vec![
            "settings",
            "export",
            "--output",
            "out.scpbak",
            "--passphrase",
            "inline-passphrase",
            "--json",
        ],
        vec![
            "settings",
            "import",
            "--input",
            "in.scpbak",
            "--preview",
            "--passphrase=inline",
            "--json",
        ],
        vec![
            "settings",
            "import",
            "--input",
            "in.scpbak",
            "--preview",
            "--recover-identity",
            "--passphrase-stdin",
            "--json",
        ],
        vec![
            "settings",
            "import",
            "--input",
            "in.scpbak",
            "--preview",
            "--recover-identity=true",
            "--passphrase-stdin",
            "--json",
        ],
        vec![
            "server",
            "credentials",
            "set",
            "--server-id",
            "s1",
            "--secret-file",
            pw_file.to_str().unwrap(),
            "--secret-stdin",
            "--json",
        ],
        vec![
            "server",
            "upsert",
            "--file",
            pw_file.to_str().unwrap(),
            "--stdin",
            "--json",
        ],
        vec![
            "server",
            "rdp",
            "--server-id",
            "s1",
            "--interactive",
            "--non-interactive",
        ],
        vec![
            "server",
            "rdp",
            "--server-id",
            "s1",
            "--interactive",
            "--json",
        ],
        vec![
            "settings",
            "import",
            "--input",
            "in.scpbak",
            "--apply",
            "--interactive",
            "--json",
        ],
        vec![
            "config",
            "secret",
            "set",
            "--key",
            "openvpn-password",
            "--interactive",
            "--json",
        ],
        vec![
            "settings",
            "export",
            "--output",
            "out.scpbak",
            "--interactive",
            "--json",
        ],
        vec![
            "settings",
            "import",
            "--input",
            "in.scpbak",
            "--preview",
            "--apply",
            "--interactive",
            "--passphrase-stdin",
        ],
    ] {
        let out = env.run(Some("default"), &bad_args, &[]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "args={bad_args:?} stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if bad_args.contains(&"--json") {
            let body = parse_single_json(&out);
            assert_eq!(body["error"]["code"], "INVALID_ARGUMENTS");
        }
    }

    // Non-interactive fast-fail (exit 4 CAPABILITY_UNAVAILABLE) without touching files.
    let no_tty = env.run(
        Some("default"),
        &["server", "rdp", "--server-id", "s1", "--interactive"],
        &[],
    );
    assert_eq!(no_tty.status.code(), Some(4));

    for cap_args in [
        vec!["server", "rdp", "--server-id", "s1"],
        vec!["server", "rdp", "--server-id", "s1", "--non-interactive"],
        vec![
            "server",
            "rdp",
            "--server-id",
            "s1",
            "--non-interactive",
            "--json",
        ],
        vec!["settings", "import", "--input", "x.scpbak", "--apply"],
        vec![
            "settings",
            "import",
            "--input",
            "x.scpbak",
            "--apply",
            "--interactive",
        ],
        vec![
            "config",
            "secret",
            "set",
            "--key",
            "openvpn-password",
            "--interactive",
        ],
        vec![
            "settings",
            "export",
            "--output",
            "out.scpbak",
            "--interactive",
        ],
    ] {
        let out = env.run(Some("default"), &cap_args, &[]);
        assert_eq!(
            out.status.code(),
            Some(4),
            "expected CAPABILITY_UNAVAILABLE (4) for {cap_args:?}, got {:?} stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn server_config_settings_credentials_and_sync_push_preserve_previews_revisions_and_isolation() {
    let env = Slice3Env::new();

    // Seed desktop keyring entry that must remain untouched.
    fs::write(
        env.keyring_file(),
        serde_json::to_string_pretty(&json!({
            KEYRING_SERVICE: {
                KEYRING_OPENVPN_PASSWORD: "desktop-vpn-secret"
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let server_file = env.path().join("srv.json");
    fs::write(
        &server_file,
        serde_json::to_string_pretty(&json!({
            "id": "srv-1",
            "name": "Win $(host)",
            "host": "10.20.30.40",
            "os": "windows",
            "connectionMethod": "ssh",
            "sshPort": 22,
            "sshUser": "admin"
        }))
        .unwrap(),
    )
    .unwrap();

    // Preview server upsert does not write servers.json.
    let preview = env.run(
        Some("work"),
        &[
            "server",
            "upsert",
            "--file",
            server_file.to_str().unwrap(),
            "--preview",
            "--json",
        ],
        &[],
    );
    assert!(preview.status.success());
    let pbody = parse_single_json(&preview);
    assert_eq!(pbody["data"]["preview"], true);
    assert_eq!(pbody["data"]["action"], "create");
    let rev = pbody["data"]["revision"].as_str().unwrap().to_string();
    assert!(!env.profile_dir("work").join("servers.json").exists());

    // Commit with expected revision succeeds; reusing stale revision fails with exit 5 CONFLICT.
    let commit = env.run(
        Some("work"),
        &[
            "server",
            "upsert",
            "--file",
            server_file.to_str().unwrap(),
            "--expected-revision",
            &rev,
            "--json",
        ],
        &[],
    );
    assert!(commit.status.success());
    let cbody = parse_single_json(&commit);
    assert_eq!(cbody["data"]["preview"], false);

    let conflict = env.run(
        Some("work"),
        &[
            "server",
            "upsert",
            "--file",
            server_file.to_str().unwrap(),
            "--expected-revision",
            &rev,
            "--json",
        ],
        &[],
    );
    assert_eq!(conflict.status.code(), Some(5));
    let confl_body = parse_single_json(&conflict);
    assert_eq!(confl_body["error"]["code"], "CONFLICT");
    assert_eq!(confl_body["error"]["reason"], "revision_mismatch");

    // Set & clear server SSH password via file and stdin.
    let ssh_pw_file = env.path().join("ssh-pw.txt");
    fs::write(&ssh_pw_file, "ssh-secret-1\r\n").unwrap();
    let set_srv_pw = env.run(
        Some("work"),
        &[
            "server",
            "credentials",
            "set",
            "--server-id",
            "srv-1",
            "--secret-file",
            ssh_pw_file.to_str().unwrap(),
            "--json",
        ],
        &[],
    );
    assert!(set_srv_pw.status.success());

    let meta: Value = serde_json::from_slice(
        &fs::read(env.profile_dir("work").join("profile-meta.json")).unwrap(),
    )
    .unwrap();
    let ns = meta["credentialNamespace"].as_str().unwrap().to_string();
    let kmap = env.read_keyring_map();
    assert_eq!(
        kmap.get(&keyring_entry_key(&format!(
            "profile:{ns}:server_ssh_password:srv-1"
        )))
        .map(String::as_str),
        Some("ssh-secret-1")
    );
    assert_eq!(
        kmap.get(&keyring_entry_key(KEYRING_OPENVPN_PASSWORD))
            .map(String::as_str),
        Some("desktop-vpn-secret")
    );

    let set_srv_stdin = env.run_with_stdin(
        Some("work"),
        &[
            "server",
            "credentials",
            "set",
            "--server-id",
            "srv-1",
            "--secret-stdin",
            "--json",
        ],
        Some(b"ssh-secret-2\n"),
        &[],
    );
    assert!(set_srv_stdin.status.success());
    assert_eq!(
        env.read_keyring_map()
            .get(&keyring_entry_key(&format!(
                "profile:{ns}:server_ssh_password:srv-1"
            )))
            .map(String::as_str),
        Some("ssh-secret-2")
    );

    // Config save rejects inline secrets; config patch merges cleanly via stdin.
    let bad_cfg = env.run_with_stdin(
        Some("work"),
        &["config", "save", "--file", "-", "--json"],
        Some(br#"{"app":{"defaultTerminalCols":120},"openvpn":{"password":"bad"}}"#),
        &[],
    );
    assert_eq!(bad_cfg.status.code(), Some(2));

    let patch_cfg = env.run_with_stdin(
        Some("work"),
        &["config", "patch", "--stdin", "--json"],
        Some(br#"{"app":{"defaultTerminalCols":140},"boundary":{"addr":"https://b.example"}}"#),
        &[],
    );
    assert!(patch_cfg.status.success());

    // Config secret set + clear via stdin.
    let set_cfg_sec = env.run_with_stdin(
        Some("work"),
        &[
            "config",
            "secret",
            "set",
            "--key",
            "openvpn-password",
            "--secret-stdin",
            "--json",
        ],
        Some(b"vpn-pass-work\n"),
        &[],
    );
    assert!(set_cfg_sec.status.success());

    let set_otp = env.run_with_stdin(
        Some("work"),
        &[
            "config",
            "secret",
            "set",
            "--key",
            "openvpn-otp-secret",
            "--secret-stdin",
            "--json",
        ],
        Some(b"JBSWY3DPEHPK3PXP\n"),
        &[],
    );
    assert!(set_otp.status.success());
    assert_eq!(
        env.read_keyring_map()
            .get(&keyring_entry_key(&format!(
                "profile:{ns}:{KEYRING_OPENVPN_OTP_SECRET}"
            )))
            .map(String::as_str),
        Some("JBSWY3DPEHPK3PXP")
    );
    let clr_otp = env.run(
        Some("work"),
        &[
            "config",
            "secret",
            "clear",
            "--key",
            "openvpn-otp-secret",
            "--json",
        ],
        &[],
    );
    assert!(clr_otp.status.success());
    assert!(!env
        .read_keyring_map()
        .contains_key(&keyring_entry_key(&format!(
            "profile:{ns}:{KEYRING_OPENVPN_OTP_SECRET}"
        ))));

    // Settings patch rejects desktop services activation; allows repoRoot.
    let bad_sync = env.run_with_stdin(
        Some("work"),
        &["settings", "patch", "--stdin", "--json"],
        Some(br#"{"mcpEnabled":true}"#),
        &[],
    );
    assert_eq!(bad_sync.status.code(), Some(2));

    let repo = env.path().join("sync-repo");
    fs::create_dir_all(&repo).unwrap();
    let ok_sync = env.run_with_stdin(
        Some("work"),
        &["settings", "patch", "--stdin", "--json"],
        Some(
            format!(
                r#"{{"repoRoot":"{}","caddyFolder":"/tmp/c"}}"#,
                repo.display()
            )
            .as_bytes(),
        ),
        &[],
    );
    assert!(ok_sync.status.success());

    // Config sync push preview & commit.
    let push_prev = env.run(
        Some("work"),
        &["config", "sync", "push", "--preview", "--json"],
        &[],
    );
    assert!(push_prev.status.success());
    let push_rev = parse_single_json(&push_prev)["data"]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!repo.join("app-config").join("config.json").exists());

    let push_ok = env.run(
        Some("work"),
        &[
            "config",
            "sync",
            "push",
            "--expected-revision",
            &push_rev,
            "--json",
        ],
        &[],
    );
    assert!(push_ok.status.success());
    let pushed_cfg = fs::read_to_string(repo.join("app-config").join("config.json")).unwrap();
    assert!(!pushed_cfg.contains("vpn-pass-work"));
    assert!(repo.join("app-config").join("servers.json").exists());
    assert!(!repo.join("app-config").join("sync-local.json").exists());
    assert!(!repo.join("app-config").join("stdio-policy.json").exists());
    assert!(!repo.join("app-config").join("profile-meta.json").exists());

    // Clear credentials and delete server; missing server -> exit 6 NOT_FOUND.
    let clr_srv = env.run(
        Some("work"),
        &[
            "server",
            "credentials",
            "clear",
            "--server-id",
            "srv-1",
            "--json",
        ],
        &[],
    );
    assert!(clr_srv.status.success());
    assert!(!env
        .read_keyring_map()
        .contains_key(&keyring_entry_key(&format!(
            "profile:{ns}:server_ssh_password:srv-1"
        ))));

    let del_ok = env.run(
        Some("work"),
        &["server", "delete", "--server-id", "srv-1", "--json"],
        &[],
    );
    assert!(del_ok.status.success());

    let del_missing = env.run(
        Some("work"),
        &["server", "delete", "--server-id", "srv-1", "--json"],
        &[],
    );
    assert_eq!(del_missing.status.code(), Some(6));
    assert_eq!(
        parse_single_json(&del_missing)["error"]["code"],
        "NOT_FOUND"
    );

    // Desktop secret stayed untouched throughout.
    assert_eq!(
        env.read_keyring_map()
            .get(&keyring_entry_key(KEYRING_OPENVPN_PASSWORD))
            .map(String::as_str),
        Some("desktop-vpn-secret")
    );
}

#[test]
fn cli_startup_recovers_interrupted_journal_automatically() {
    let env = Slice3Env::new();
    let pdir = env.profile_dir("recover");
    assert!(env
        .run(Some("recover"), &["status", "--json"], &[])
        .status
        .success());
    let meta: Value =
        serde_json::from_slice(&fs::read(pdir.join("profile-meta.json")).unwrap()).unwrap();
    let ns = meta["credentialNamespace"].as_str().unwrap().to_string();

    let orig_servers = br#"{"servers":[{"id":"orig","name":"Orig","host":"1.1.1.1","os":"linux","connectionMethod":"ssh","sshPort":22,"sshUser":"ops"}]}"#;
    fs::write(pdir.join("servers.json"), orig_servers).unwrap();

    let mut next_files = BTreeMap::new();
    let corrupted: Value = serde_json::json!({"servers":[{"id":"corrupted-halfway","name":"Bad","host":"2.2.2.2","os":"linux","connectionMethod":"ssh","sshPort":22,"sshUser":"ops"}]});
    next_files.insert("servers.json".into(), Some(corrupted.clone()));
    let changes = Changes {
        files: next_files,
        credentials: BTreeMap::new(),
    };
    let mut store = TestKeyringStore {
        file: env.keyring_file().clone(),
        namespace: ns,
    };
    transaction::prepare(&pdir, &changes, &mut store).unwrap();
    // Simulate crash right after writing servers.json while journal remains uncommitted.
    fs::write(
        pdir.join("servers.json"),
        serde_json::to_vec(&corrupted).unwrap(),
    )
    .unwrap();

    let list_out = env.run(Some("recover"), &["server", "list", "--json"], &[]);
    assert!(
        list_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&list_out.stderr)
    );
    let lbody = parse_single_json(&list_out);
    assert_eq!(lbody["data"]["servers"][0]["id"], "orig");
}

#[cfg(unix)]
mod unix_pty_tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::time::{Duration, Instant};

    struct PtyHarness {
        master: fs::File,
        orig_termios: libc::termios,
        child: std::process::Child,
    }

    fn spawn_in_pty(
        env: &Slice3Env,
        profile: &str,
        args: &[&str],
        extra: &[(&str, &str)],
    ) -> PtyHarness {
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

        let master = unsafe { fs::File::from_raw_fd(master_fd) };
        let slave_in = unsafe { fs::File::from_raw_fd(slave_fd) };
        let slave_out = slave_in.try_clone().unwrap();
        let slave_err = slave_in.try_clone().unwrap();

        let mut cmd = Command::new(scpctl_bin());
        cmd.args([
            "--config-dir",
            env.root().to_str().unwrap(),
            "--profile",
            profile,
        ])
        .args(args)
        .env("SCPCTL_DESKTOP_CONFIG_DIR", env.desktop())
        .env("SCPCTL_TEST_KEYRING_FILE", env.keyring_file())
        .stdin(Stdio::from(slave_in))
        .stdout(Stdio::from(slave_out))
        .stderr(Stdio::from(slave_err));
        for (k, v) in extra {
            cmd.env(k, v);
        }
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        let child = cmd.spawn().unwrap();
        PtyHarness {
            master,
            orig_termios,
            child,
        }
    }

    fn set_nonblocking(file: &fs::File) {
        let fd = file.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    }

    fn wait_for_pty_text(master: &mut fs::File, acc: &mut String, needle: &str, timeout: Duration) {
        wait_for_pty_text_from(master, acc, 0, needle, timeout);
    }

    fn wait_for_pty_text_from(
        master: &mut fs::File,
        acc: &mut String,
        from: usize,
        needle: &str,
        timeout: Duration,
    ) {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 4096];
        while !acc.get(from..).is_some_and(|s| s.contains(needle)) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}; captured PTY text:\n{acc}"
            );
            match master.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => acc.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(15));
                }
                Err(_) => break,
            }
        }
        assert!(
            acc.get(from..).is_some_and(|s| s.contains(needle)),
            "PTY closed before seeing {needle:?}; captured PTY text:\n{acc}"
        );
    }

    fn finish_pty(mut h: PtyHarness, acc: &mut String) -> (i32, libc::c_ulong) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            let mut buf = [0u8; 4096];
            if let Ok(n) = h.master.read(&mut buf) {
                if n > 0 {
                    acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            }
            if let Some(st) = h.child.try_wait().unwrap() {
                break st;
            }
            assert!(Instant::now() < deadline, "PTY child timed out:\n{acc}");
            std::thread::sleep(Duration::from_millis(15));
        };
        let mut buf = [0u8; 4096];
        while let Ok(n) = h.master.read(&mut buf) {
            if n == 0 {
                break;
            }
            acc.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        let mut after_termios = unsafe { std::mem::zeroed::<libc::termios>() };
        assert_eq!(
            unsafe { libc::tcgetattr(h.master.as_raw_fd(), &mut after_termios) },
            0
        );
        (status.code().unwrap_or(-1), after_termios.c_lflag as _)
    }

    #[test]
    fn interactive_secret_prompt_disables_echo_and_restores_on_commit_and_sigint() {
        let env = Slice3Env::new();

        // 1. Normal secret entry: secret must never appear in PTY transcript, ECHO restored.
        let mut h1 = spawn_in_pty(
            &env,
            "sec",
            &[
                "config",
                "secret",
                "set",
                "--key",
                "openvpn-password",
                "--interactive",
            ],
            &[],
        );
        set_nonblocking(&h1.master);
        let mut out1 = String::new();
        wait_for_pty_text(
            &mut h1.master,
            &mut out1,
            "Enter secret",
            Duration::from_secs(5),
        );
        // Wait briefly so no-echo termios takes effect before sending input.
        std::thread::sleep(Duration::from_millis(60));
        h1.master.write_all(b"super-hidden-secret-99\n").unwrap();
        h1.master.flush().unwrap();
        let (code1, lflag1) = finish_pty(h1, &mut out1);
        assert_eq!(code1, 0, "pty out: {out1}");
        assert!(
            !out1.contains("super-hidden-secret-99"),
            "secret echoed in PTY: {out1}"
        );
        assert_eq!(
            lflag1 & (libc::ECHO as libc::c_ulong),
            (h1_orig_echo_ref() & (libc::ECHO as libc::c_ulong))
        );

        let meta: Value = serde_json::from_slice(
            &fs::read(env.profile_dir("sec").join("profile-meta.json")).unwrap(),
        )
        .unwrap();
        let ns = meta["credentialNamespace"].as_str().unwrap();
        assert_eq!(
            env.read_keyring_map()
                .get(&keyring_entry_key(&format!(
                    "profile:{ns}:{KEYRING_OPENVPN_PASSWORD}"
                )))
                .map(String::as_str),
            Some("super-hidden-secret-99")
        );

        // 2. SIGINT (\x03) during no-echo prompt restores ECHO and exits 130 without changing secret.
        let mut h2 = spawn_in_pty(
            &env,
            "sec",
            &[
                "config",
                "secret",
                "set",
                "--key",
                "openvpn-password",
                "--interactive",
            ],
            &[],
        );
        let orig_echo = (h2.orig_termios.c_lflag as libc::c_ulong) & (libc::ECHO as libc::c_ulong);
        set_nonblocking(&h2.master);
        let mut out2 = String::new();
        wait_for_pty_text(
            &mut h2.master,
            &mut out2,
            "Enter secret",
            Duration::from_secs(5),
        );
        std::thread::sleep(Duration::from_millis(60));
        h2.master.write_all(b"\x03").unwrap();
        h2.master.flush().unwrap();
        let (code2, lflag2) = finish_pty(h2, &mut out2);
        assert_eq!(code2, 130, "expected SIGINT exit 130, pty out: {out2}");
        assert_eq!(lflag2 & (libc::ECHO as libc::c_ulong), orig_echo);
        assert_eq!(
            env.read_keyring_map()
                .get(&keyring_entry_key(&format!(
                    "profile:{ns}:{KEYRING_OPENVPN_PASSWORD}"
                )))
                .map(String::as_str),
            Some("super-hidden-secret-99")
        );
    }

    fn h1_orig_echo_ref() -> libc::c_ulong {
        libc::ECHO as libc::c_ulong
    }

    #[test]
    fn server_rdp_interactive_enforces_display_and_launches_safely() {
        let env = Slice3Env::new();
        let srv_file = env.path().join("win.json");
        fs::write(
            &srv_file,
            serde_json::to_string_pretty(&json!({
                "id": "win-1",
                "name": "Win RDP",
                "host": "win.internal",
                "os": "windows",
                "connectionMethod": "ssh",
                "sshPort": 22,
                "sshUser": "admin"
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(env
            .run(
                Some("rdp"),
                &[
                    "server",
                    "upsert",
                    "--file",
                    srv_file.to_str().unwrap(),
                    "--json"
                ],
                &[],
            )
            .status
            .success());

        // Missing DISPLAY even inside PTY fails with exit 4.
        let h_no_disp = spawn_in_pty(
            &env,
            "rdp",
            &["server", "rdp", "--server-id", "win-1", "--interactive"],
            &[("DISPLAY", ""), ("WAYLAND_DISPLAY", "")],
        );
        set_nonblocking(&h_no_disp.master);
        let mut out_no = String::new();
        let (code_no, _) = finish_pty(h_no_disp, &mut out_no);
        assert_eq!(code_no, 4, "out={out_no}");

        // Valid DISPLAY + custom launcher inside PTY exits 0 and receives rdp://win.internal.
        let capture = env.path().join("rdp-captured.txt");
        let launcher = env.path().join("fake-rdp.sh");
        fs::write(
            &launcher,
            format!(
                "#!/usr/bin/env sh\nprintf '%s' \"$1\" > \"{}\"\n",
                capture.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();

        let h_ok = spawn_in_pty(
            &env,
            "rdp",
            &["server", "rdp", "--server-id", "win-1", "--interactive"],
            &[
                ("DISPLAY", ":99"),
                ("SCPCTL_RDP_LAUNCHER", launcher.to_str().unwrap()),
            ],
        );
        set_nonblocking(&h_ok.master);
        let mut out_ok = String::new();
        let (code_ok, _) = finish_pty(h_ok, &mut out_ok);
        assert_eq!(code_ok, 0, "out={out_ok}");
        assert_eq!(fs::read_to_string(&capture).unwrap(), "rdp://win.internal");
    }

    #[test]
    fn settings_export_preview_and_interactive_apply_preserve_isolation_and_ttl() {
        let env = Slice3Env::new();

        // Source profile "src" to export .scpbak from.
        let srv_file = env.path().join("src-srv.json");
        fs::write(
            &srv_file,
            serde_json::to_string_pretty(&json!({
                "id": "srv-exported",
                "name": "Exported Host",
                "host": "10.9.9.9",
                "os": "linux",
                "connectionMethod": "ssh",
                "sshPort": 22,
                "sshUser": "ops"
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(env
            .run(
                Some("src"),
                &[
                    "server",
                    "upsert",
                    "--file",
                    srv_file.to_str().unwrap(),
                    "--json"
                ],
                &[],
            )
            .status
            .success());
        assert!(env
            .run_with_stdin(
                Some("src"),
                &[
                    "config",
                    "secret",
                    "set",
                    "--key",
                    "openvpn-password",
                    "--secret-stdin",
                    "--json",
                ],
                Some(b"exported-vpn-pass\n"),
                &[],
            )
            .status
            .success());
        assert!(env
            .run_with_stdin(
                Some("src"),
                &["config", "patch", "--stdin", "--json"],
                Some(br#"{"app":{"defaultTerminalCols":150}}"#),
                &[],
            )
            .status
            .success());

        let scpbak = env.path().join("export.scpbak");
        let pfile = env.path().join("passphrase.txt");
        fs::write(&pfile, "correct horse battery staple\n").unwrap();

        let exp = env.run(
            Some("src"),
            &[
                "settings",
                "export",
                "--output",
                scpbak.to_str().unwrap(),
                "--passphrase-file",
                pfile.to_str().unwrap(),
                "--json",
            ],
            &[],
        );
        assert!(
            exp.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&exp.stderr)
        );
        assert_eq!(parse_single_json(&exp)["data"]["complete"], true);

        // Destination profile "dst" starts with its own namespace and read-only stdio policy + desktop secret untouched.
        assert!(env
            .run(Some("dst"), &["status", "--json"], &[])
            .status
            .success());
        let dst_policy_path = env.profile_dir("dst").join("stdio-policy.json");
        let dst_policy_before =
            r#"{"sensitiveActionPolicy":"read-only","allowedToolCategories":["read-only"]}"#
                .to_string();
        fs::write(&dst_policy_path, &dst_policy_before).unwrap();
        let dst_meta_before: Value = serde_json::from_slice(
            &fs::read(env.profile_dir("dst").join("profile-meta.json")).unwrap(),
        )
        .unwrap();
        let dst_ns = dst_meta_before["credentialNamespace"]
            .as_str()
            .unwrap()
            .to_string();

        // Non-mutating --preview via passphrase stdin.
        let prev = env.run_with_stdin(
            Some("dst"),
            &[
                "settings",
                "import",
                "--input",
                scpbak.to_str().unwrap(),
                "--preview",
                "--passphrase-stdin",
                "--json",
            ],
            Some(b"correct horse battery staple\n"),
            &[],
        );
        assert!(
            prev.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&prev.stderr)
        );
        let prev_body = parse_single_json(&prev);
        assert!(prev_body["data"]["id"].as_str().is_some());
        assert_eq!(prev_body["data"]["servers"], 1);
        assert!(env
            .run(Some("dst"), &["server", "list", "--json"], &[])
            .status
            .success());
        let after_prev_list =
            parse_single_json(&env.run(Some("dst"), &["server", "list", "--json"], &[]));
        assert_eq!(after_prev_list["data"]["total"], 0);

        // Copy scpbak to test deletion after single load + interactive section change + commit.
        let scpbak_once = env.path().join("delete-after-load.scpbak");
        fs::copy(&scpbak, &scpbak_once).unwrap();

        let mut h = spawn_in_pty(
            &env,
            "dst",
            &[
                "settings",
                "import",
                "--input",
                scpbak_once.to_str().unwrap(),
                "--apply",
                "--interactive",
            ],
            &[],
        );
        set_nonblocking(&h.master);
        let mut tty_out = String::new();
        wait_for_pty_text(
            &mut h.master,
            &mut tty_out,
            "passphrase",
            Duration::from_secs(5),
        );
        std::thread::sleep(Duration::from_millis(60));
        h.master
            .write_all(b"correct horse battery staple\n")
            .unwrap();
        h.master.flush().unwrap();

        wait_for_pty_text(
            &mut h.master,
            &mut tty_out,
            "Preview ID:",
            Duration::from_secs(5),
        );
        let first_id = tty_out
            .lines()
            .find_map(|l| l.strip_prefix("Preview ID: "))
            .unwrap()
            .trim()
            .to_string();

        // Delete the .scpbak file immediately after initial preview to prove single in-memory load!
        fs::remove_file(&scpbak_once).unwrap();

        // Change selection interactively (`sections shared,local` excludes servers+secrets) -> must mint a new Preview ID.
        let mark = tty_out.len();
        h.master.write_all(b"sections shared,local\n").unwrap();
        h.master.flush().unwrap();
        wait_for_pty_text_from(
            &mut h.master,
            &mut tty_out,
            mark,
            "Preview ID:",
            Duration::from_secs(5),
        );
        wait_for_pty_text_from(
            &mut h.master,
            &mut tty_out,
            mark,
            "Apply",
            Duration::from_secs(5),
        );
        let second_id = tty_out
            .lines()
            .filter_map(|l| l.strip_prefix("Preview ID: "))
            .next_back()
            .unwrap()
            .trim()
            .to_string();
        assert_ne!(
            second_id, first_id,
            "interactive option edit must mint fresh preview ID"
        );

        // Re-enable all sections on the in-memory pending restore and commit.
        let mark2 = tty_out.len();
        h.master
            .write_all(b"sections shared,servers,local,secrets\n")
            .unwrap();
        h.master.flush().unwrap();
        wait_for_pty_text_from(
            &mut h.master,
            &mut tty_out,
            mark2,
            "Preview ID:",
            Duration::from_secs(5),
        );
        wait_for_pty_text_from(
            &mut h.master,
            &mut tty_out,
            mark2,
            "Apply",
            Duration::from_secs(5),
        );
        let third_id = tty_out
            .lines()
            .filter_map(|l| l.strip_prefix("Preview ID: "))
            .next_back()
            .unwrap()
            .trim()
            .to_string();
        assert_ne!(third_id, second_id);

        h.master.write_all(b"yes\n").unwrap();
        h.master.flush().unwrap();
        let (code, _) = finish_pty(h, &mut tty_out);
        assert_eq!(code, 0, "pty out:\n{tty_out}");
        assert!(!tty_out.contains("correct horse battery staple"));

        // Verify shared config + servers + namespaced secrets imported, profile-meta & stdio-policy preserved,
        // and MCP/CC forced disabled.
        let show = parse_single_json(&env.run(Some("dst"), &["settings", "show", "--json"], &[]));
        assert_eq!(show["data"]["appConfig"]["app"]["defaultTerminalCols"], 150);
        assert_eq!(show["data"]["syncSettings"]["mcpEnabled"], false);
        assert_eq!(show["data"]["syncSettings"]["ccEnabled"], false);
        assert_eq!(
            env.read_keyring_map()
                .get(&format!("profile:{dst_ns}:{KEYRING_OPENVPN_PASSWORD}"))
                .map(String::as_str),
            Some("exported-vpn-pass")
        );
        assert_eq!(
            fs::read_to_string(&dst_policy_path).unwrap(),
            dst_policy_before
        );
        let dst_meta_after: Value = serde_json::from_slice(
            &fs::read(env.profile_dir("dst").join("profile-meta.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(dst_meta_after["credentialNamespace"], dst_ns);

        // Now test TTL expiration rejection inside PTY.
        let mut h_ttl = spawn_in_pty(
            &env,
            "dst",
            &[
                "settings",
                "import",
                "--input",
                scpbak.to_str().unwrap(),
                "--apply",
                "--interactive",
            ],
            &[("SCPCTL_TEST_PREVIEW_TTL_MS", "25")],
        );
        set_nonblocking(&h_ttl.master);
        let mut ttl_out = String::new();
        wait_for_pty_text(
            &mut h_ttl.master,
            &mut ttl_out,
            "passphrase",
            Duration::from_secs(5),
        );
        std::thread::sleep(Duration::from_millis(60));
        h_ttl
            .master
            .write_all(b"correct horse battery staple\n")
            .unwrap();
        h_ttl.master.flush().unwrap();
        wait_for_pty_text(
            &mut h_ttl.master,
            &mut ttl_out,
            "Preview ID:",
            Duration::from_secs(5),
        );
        std::thread::sleep(Duration::from_millis(90));
        h_ttl.master.write_all(b"yes\n").unwrap();
        h_ttl.master.flush().unwrap();
        let (ttl_code, _) = finish_pty(h_ttl, &mut ttl_out);
        assert_eq!(
            ttl_code, 5,
            "expected expired preview conflict (5), out:\n{ttl_out}"
        );

        // Now test destination-changed rejection between preview and yes.
        let mut h_mut = spawn_in_pty(
            &env,
            "dst",
            &[
                "settings",
                "import",
                "--input",
                scpbak.to_str().unwrap(),
                "--apply",
                "--interactive",
            ],
            &[],
        );
        set_nonblocking(&h_mut.master);
        let mut mut_out = String::new();
        wait_for_pty_text(
            &mut h_mut.master,
            &mut mut_out,
            "passphrase",
            Duration::from_secs(5),
        );
        std::thread::sleep(Duration::from_millis(60));
        h_mut
            .master
            .write_all(b"correct horse battery staple\n")
            .unwrap();
        h_mut.master.flush().unwrap();
        wait_for_pty_text(
            &mut h_mut.master,
            &mut mut_out,
            "Preview ID:",
            Duration::from_secs(5),
        );

        // Mutate destination servers.json while apply confirmation is waiting.
        fs::write(
            env.profile_dir("dst").join("servers.json"),
            r#"{"servers":[{"id":"concurrent","name":"Concurrent","host":"1.2.3.4","os":"linux","connectionMethod":"ssh","sshPort":22,"sshUser":"ops"}]}"#,
        )
        .unwrap();
        h_mut.master.write_all(b"yes\n").unwrap();
        h_mut.master.flush().unwrap();
        let (mut_code, _) = finish_pty(h_mut, &mut mut_out);
        assert_eq!(
            mut_code, 5,
            "expected destination change conflict (5), out:\n{mut_out}"
        );
        let dst_servers_text =
            fs::read_to_string(env.profile_dir("dst").join("servers.json")).unwrap();
        assert!(dst_servers_text.contains("concurrent"));
        assert!(!dst_servers_text.contains("srv-exported"));
    }
}
