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
    if std::env::var("SCP_ACCEPTANCE_MODE")
        .unwrap()
        .starts_with("descendant")
    {
        let mut descendant = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "caddy_detached_descendant",
                "--nocapture",
            ])
            .spawn()
            .unwrap();
        if std::env::var("SCP_ACCEPTANCE_MODE").unwrap() == "descendant-complete" {
            wait_until(|| root.join("parent-release").exists());
            std::process::exit(0);
        }
        // Retain Child for normal watchdog cleanup; cancellation kills this
        // session leader first, whereupon the test subreaper adopts its child.
        std::thread::sleep(Duration::from_secs(6));
        let _ = descendant.kill();
        let _ = descendant.wait();
        std::process::exit(9);
    }
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

#[test]
#[ignore = "detached PTY holder exercised by the descendant acceptance test"]
fn caddy_detached_descendant() {
    let root = PathBuf::from(std::env::var_os("SCP_ACCEPTANCE_ROOT").unwrap());
    // SAFETY: change only this fixture's session, retaining inherited PTY fds.
    // Detachment avoids terminal session-leader HUP hiding a surviving child.
    assert!(unsafe { libc::setsid() } > 0);
    fs::write(root.join("descendant-pid"), std::process::id().to_string()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    while !root.join("release").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if root.join("release").exists() {
        fs::write(root.join("released"), "acknowledged").unwrap();
    }
    std::process::exit(0);
}

// Adopt only this test binary's orphaned fixtures. Cleanup waits/signals exact
// child PIDs, never process groups or unrelated processes. Declared before the
// CLI guard so that the CLI is reaped before adoption cleanup on assertion failure.
struct FixtureTreeCleanup(PathBuf);

impl Drop for FixtureTreeCleanup {
    fn drop(&mut self) {
        for name in ["pid", "descendant-pid"] {
            let Some(pid) = fs::read_to_string(self.0.join(name))
                .ok()
                .and_then(|text| text.parse::<libc::pid_t>().ok())
                .filter(|pid| *pid > 1)
            else {
                continue;
            };
            // SAFETY: positive fixture PID only. waitpid confirms ownership
            // before signaling, avoiding stale/reused non-child PID targets.
            unsafe {
                if libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) == 0 {
                    libc::kill(pid, libc::SIGKILL);
                    while libc::waitpid(pid, std::ptr::null_mut(), 0) == -1
                        && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                    {
                    }
                }
            }
        }
    }
}

#[test]
fn detached_descendant_distinguishes_completion_drain_from_cancellation_cleanup() {
    // SAFETY: Linux test-process-only adoption; no production supervisor change.
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
    for mode in ["descendant", "descendant-complete"] {
        let fixture = CaddyFixture::new();
        let _tree_cleanup = FixtureTreeCleanup(fixture.root.path().to_path_buf());
        let mut child = CliChild::spawn(&mut fixture.command(mode));
        let direct_pid = wait_for_child(&fixture.path("pid"), &mut child);
        let descendant_pid = wait_for_child(&fixture.path("descendant-pid"), &mut child);
        let fd = Path::new("/proc")
            .join(descendant_pid.to_string())
            .join("fd/1");
        let terminal = fs::read_link(&fd).unwrap();
        assert!(terminal.starts_with("/dev/pts"), "{terminal:?}");
        assert!(ProfileOwner::acquire(&fixture.path("profile")).is_err());
        if mode == "descendant" {
            assert!(Command::new("/bin/kill")
                .args(["-TERM", &child.0.as_ref().unwrap().id().to_string()])
                .status()
                .unwrap()
                .success());
        } else {
            fs::write(fixture.path("parent-release"), "exit now").unwrap();
            // Completion drains before polling/reaping. Prove the direct
            // child really exited, rather than relying on a scheduling delay.
            wait_for_unreaped_exit(direct_pid);
            std::thread::sleep(Duration::from_millis(150));
            assert!(child.0.as_mut().unwrap().try_wait().unwrap().is_none());
            assert!(ProfileOwner::acquire(&fixture.path("profile")).is_err());
            assert_eq!(fs::read_link(&fd).unwrap(), terminal);
            fs::write(fixture.path("release"), "release").unwrap();
        }
        let output = child.finish();
        let (code, result) = if mode == "descendant" {
            assert!(String::from_utf8_lossy(&output.stderr).contains("Caddy cancelled"));
            (1, json!({"outcome":"cancelled"}))
        } else {
            assert!(output.stderr.is_empty(), "{output:?}");
            (0, json!({"outcome":"completed", "exitCode":0}))
        };
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            result
        );
        assert!(!Path::new("/proc").join(direct_pid.to_string()).exists());
        fixture.assert_released();
        if mode == "descendant" {
            // Cancellation closes the master and joins successfully even
            // though the detached child retains its now hung-up slave fd.
            let retained = fs::read_link(fd).unwrap();
            assert_eq!(
                retained.to_str().unwrap().trim_end_matches(" (deleted)"),
                terminal.to_str().unwrap()
            );
            fs::write(fixture.path("release"), "release").unwrap();
        }
        assert_released_descendant_reaped(&fixture, descendant_pid);
    }
}

fn wait_for_unreaped_exit(pid: u32) {
    wait_until(|| {
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        stat.rsplit_once(") ")
            .is_some_and(|(_, fields)| fields.starts_with('Z'))
    });
}

fn assert_released_descendant_reaped(fixture: &CaddyFixture, pid: u32) {
    wait_until(|| fixture.path("released").exists());
    let mut status = 0;
    // SAFETY: positive PID published by our fixture, adopted by this test.
    assert_eq!(
        unsafe { libc::waitpid(pid as i32, &mut status, 0) },
        pid as i32
    );
    assert!(libc::WIFEXITED(status));
    assert_eq!(libc::WEXITSTATUS(status), 0);
    assert!(!Path::new("/proc").join(pid.to_string()).exists());
}
