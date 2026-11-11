#![cfg(target_os = "linux")]

use scp_core::profile::ProfileOwner;
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

// Every fixture lives under a fresh root and immediately execs this test binary.
// No installed Caddy, configuration, listener, or credentials are used.
struct CaddyFixture {
    root: tempfile::TempDir,
}

impl CaddyFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        fs::create_dir(root.path().join("folder")).unwrap();
        fs::write(root.path().join("folder/Caddyfile"), "fixture only").unwrap();
        let shim = root.path().join("bin/caddy");
        fs::write(&shim, "#!/bin/sh\nexec \"$SCP_ACCEPTANCE_EXE\" --ignored --exact caddy_acceptance_child --nocapture\n").unwrap();
        fs::set_permissions(shim, fs::Permissions::from_mode(0o700)).unwrap();
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn command(&self, mode: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_scpctl"));
        command
            .env("PATH", self.path("bin"))
            .env("SCP_ACCEPTANCE_EXE", std::env::current_exe().unwrap())
            .env("SCP_ACCEPTANCE_ROOT", self.root.path())
            .env("SCP_ACCEPTANCE_MODE", mode)
            .args(["--config-dir", self.path("profile").to_str().unwrap()])
            .args(["--json", "caddy", "run", "--foreground", "--folder"])
            .arg(self.path("folder"))
            .arg("--output")
            .arg(self.path("log"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn assert_released(&self) {
        assert!(ProfileOwner::acquire(&self.path("profile")).is_ok());
        assert!(self.path("profile/runtime-owner.lock").exists());
    }
}

// Reap this exact CLI on assertion failures as well as watchdog expiry. The
// direct-child fixture has its own finite lifetime if the CLI is killed here.
struct CliChild(Option<Child>);

impl CliChild {
    fn spawn(command: &mut Command) -> Self {
        Self(Some(command.spawn().unwrap()))
    }

    fn finish(mut self) -> Output {
        wait_until(|| self.0.as_mut().unwrap().try_wait().unwrap().is_some());
        self.0.take().unwrap().wait_with_output().unwrap()
    }
}

impl Drop for CliChild {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_for_child(path: &Path, child: &mut CliChild) -> u32 {
    let mut pid = None;
    wait_until(|| {
        pid = fs::read_to_string(path)
            .ok()
            .and_then(|text| text.parse().ok());
        if pid.is_some() {
            return true;
        }
        assert!(
            child.0.as_mut().unwrap().try_wait().unwrap().is_none(),
            "CLI exited before fixture readiness"
        );
        false
    });
    pid.unwrap()
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "CLI fixture timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn real_cli_signals_cancel_reap_and_release_with_stdin_closed() {
    for signal in ["-INT", "-TERM", "-HUP"] {
        let fixture = CaddyFixture::new();
        let mut child = CliChild::spawn(&mut fixture.command("wait"));
        let pid = wait_for_child(&fixture.path("pid"), &mut child);
        assert!(ProfileOwner::acquire(&fixture.path("profile")).is_err());
        assert!(Command::new("/bin/kill")
            .args([signal, &child.0.as_ref().unwrap().id().to_string()])
            .status()
            .unwrap()
            .success());
        let output = child.finish();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            json!({"outcome":"cancelled"})
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("Caddy cancelled"));
        assert!(!Path::new("/proc").join(pid.to_string()).exists());
        fixture.assert_released();
    }
}

#[test]
fn real_cli_zero_exit_reports_one_json_result_and_flushes_raw_log() {
    let fixture = CaddyFixture::new();
    let output = CliChild::spawn(&mut fixture.command("success")).finish();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"outcome":"completed", "exitCode":0})
    );
    assert!(
        String::from_utf8_lossy(&fs::read(fixture.path("log")).unwrap())
            .contains("ACCEPTANCE-FINAL-MARKER")
    );
    fixture.assert_released();
}

#[test]
fn real_cli_launch_failure_and_owner_conflict_have_no_completion_result() {
    let fixture = CaddyFixture::new();
    let owner = ProfileOwner::acquire(&fixture.path("profile")).unwrap();
    let identity = fs::read(fixture.path("profile/profile.json")).unwrap();
    let conflict = CliChild::spawn(&mut fixture.command("success")).finish();
    assert_eq!(conflict.status.code(), Some(1), "{conflict:?}");
    assert!(conflict.stdout.is_empty());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("already owned"));
    assert!(!fixture.path("log").exists());
    assert!(!fixture.path("pid").exists());
    assert_eq!(
        fs::read(fixture.path("profile/profile.json")).unwrap(),
        identity
    );
    drop(owner);

    fs::remove_file(fixture.path("bin/caddy")).unwrap();
    let failure = CliChild::spawn(&mut fixture.command("success")).finish();
    assert_eq!(failure.status.code(), Some(1), "{failure:?}");
    assert!(failure.stdout.is_empty());
    assert!(!failure.stderr.is_empty());
    assert!(!fixture.path("pid").exists());
    assert!(fs::read(fixture.path("log")).unwrap().is_empty());
    assert_eq!(
        fs::read(fixture.path("profile/profile.json")).unwrap(),
        identity
    );
    fixture.assert_released();
}

#[test]
#[ignore = "direct PTY child exercised by the real CLI acceptance tests"]
fn caddy_acceptance_child() {
    use std::io::Write;
    let root = PathBuf::from(std::env::var_os("SCP_ACCEPTANCE_ROOT").unwrap());
    fs::write(root.join("pid"), std::process::id().to_string()).unwrap();
    println!("ACCEPTANCE-FINAL-MARKER");
    std::io::stdout().flush().unwrap();
    if std::env::var("SCP_ACCEPTANCE_MODE").unwrap() == "success" {
        std::process::exit(0);
    }
    // A test-only orphan watchdog, not a production cleanup guarantee. Sleep on
    // the fixture's main thread so even a failed parent assertion leaves no worker.
    std::thread::sleep(Duration::from_secs(6));
    std::process::exit(9);
}
