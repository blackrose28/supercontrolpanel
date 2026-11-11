use super::*;
use portable_pty::CommandBuilder;
use scp_core::{
    host::{ForegroundOutcome, ProfileRuntime},
    profile::ProfileOwner,
};
use std::{
    fs,
    io::{self, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn wait_for(path: &std::path::Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "waiting for {path:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct HostChild(Child);
impl Drop for HostChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn host(root: &std::path::Path, mode: &str) -> HostChild {
    HostChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "caddy_host::tests::host_fixture",
                "--nocapture",
            ])
            .env("SCP_CADDY_TEST_ROOT", root)
            .env("SCP_CADDY_TEST_MODE", mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    )
}

fn finished(child: &mut HostChild) {
    let start = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "fixture status: {status}");
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "host did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_child_reaped(root: &std::path::Path) {
    let pid = fs::read_to_string(root.join("child-pid")).unwrap();
    assert!(
        !std::path::Path::new("/proc").join(pid).exists(),
        "direct child survived"
    );
    assert!(ProfileOwner::acquire(&root.join("profile")).is_ok());
}

#[test]
fn termination_signals_cancel_real_child_and_release_profile_even_with_stdin_eof() {
    for signal in ["-INT", "-TERM", "-HUP"] {
        let root = tempfile::tempdir().unwrap();
        let mut child = host(root.path(), "wait");
        wait_for(&root.path().join("child-pid"));
        // stdin is already EOF; this foreground Caddy host deliberately ignores it.
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(ProfileOwner::acquire(&root.path().join("profile")).is_err());
        assert!(Command::new("kill")
            .args([signal, &child.0.id().to_string()])
            .status()
            .unwrap()
            .success());
        finished(&mut child);
        assert_eq!(
            fs::read_to_string(root.path().join("outcome")).unwrap(),
            "cancelled"
        );
        assert_child_reaped(root.path());
    }
}

#[test]
fn completed_caddy_host_preserves_large_final_output_and_nonzero_exit() {
    let root = tempfile::tempdir().unwrap();
    let mut child = host(root.path(), "complete");
    finished(&mut child);
    let bytes = fs::read(root.path().join("output")).unwrap();
    assert!(bytes.len() > 128 * 1024);
    assert!(String::from_utf8_lossy(&bytes).contains("CADDY-FINAL-MARKER"));
    assert_eq!(
        fs::read_to_string(root.path().join("outcome")).unwrap(),
        "exit:7"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(root.path().join("output"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_child_reaped(root.path());
}

#[test]
fn caddy_host_output_errors_cleanup_real_child_and_keep_primary_error() {
    for mode in ["write-error", "flush-error", "pre-cancel"] {
        let root = tempfile::tempdir().unwrap();
        let mut child = host(root.path(), mode);
        finished(&mut child);
        let outcome = fs::read_to_string(root.path().join("outcome")).unwrap();
        if mode == "pre-cancel" {
            assert_eq!(outcome, "cancelled");
            assert!(!root.path().join("child-pid").exists());
        } else {
            assert!(outcome.contains(mode), "{outcome}");
            assert_child_reaped(root.path());
        }
    }
}

#[test]
fn caddy_json_summary_never_contains_raw_output_and_failure_stays_failure() {
    let mut bytes = Vec::new();
    let result = report(
        ForegroundOutcome::Completed(scp_core::runtime::PtyExitPayload {
            id: "cli-test".into(),
            exit_code: Some(7),
            reason: "private raw child reason".into(),
        }),
        true,
        &mut bytes,
    );
    assert!(result.is_err());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::json!({"outcome":"completed", "exitCode":7})
    );
    bytes.clear();
    assert!(report(ForegroundOutcome::Cancelled, true, &mut bytes).is_err());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::json!({"outcome":"cancelled"})
    );
}

#[test]
#[ignore = "subprocess fixture only"]
fn child_fixture() {
    let root = std::path::PathBuf::from(std::env::var_os("SCP_CADDY_TEST_ROOT").unwrap());
    fs::write(root.join("child-pid"), std::process::id().to_string()).unwrap();
    let mode = std::env::var("SCP_CADDY_TEST_MODE").unwrap();
    if mode == "complete" || mode == "flush-error" {
        for _ in 0..2048 {
            println!("{}", "x".repeat(64));
        }
        println!("CADDY-FINAL-MARKER");
        io::stdout().flush().unwrap();
        std::process::exit(7);
    }
    println!("CADDY-READY");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    std::process::exit(0);
}

#[test]
#[ignore = "subprocess fixture only"]
fn host_fixture() {
    let root = std::path::PathBuf::from(std::env::var_os("SCP_CADDY_TEST_ROOT").unwrap());
    let mode = std::env::var("SCP_CADDY_TEST_MODE").unwrap();
    let shutdown = Shutdown::install().unwrap();
    if mode == "pre-cancel" {
        // Deliver repeated signals synchronously before launch, so the test cannot
        // race a second external kill against successful cleanup/process exit.
        for _ in 0..2 {
            signal_hook::low_level::raise(signal_hook::consts::SIGTERM).unwrap();
        }
    }
    let mut runtime = ProfileRuntime::acquire(&root.join("profile")).unwrap();
    let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
    command.args([
        "--ignored",
        "--exact",
        "caddy_host::tests::child_fixture",
        "--nocapture",
    ]);
    let mut file = create_output(&root.join("output")).unwrap();
    struct Failure<'a> {
        root: &'a std::path::Path,
        mode: &'a str,
    }
    impl Write for Failure<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            wait_for(&self.root.join("child-pid"));
            if self.mode == "write-error" {
                return Err(io::Error::other("write-error"));
            }
            Ok(bytes.len().min(31))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("flush-error"))
        }
    }
    let mut failure = Failure {
        root: &root,
        mode: &mode,
    };
    let sink: &mut (dyn Write + Send) = if mode.ends_with("error") {
        &mut failure
    } else {
        &mut file
    };
    let result = run_prepared(&mut runtime, command, sink, &shutdown);
    let outcome = match result {
        Ok(ForegroundOutcome::Cancelled) => "cancelled".into(),
        Ok(ForegroundOutcome::Completed(exit)) => format!("exit:{}", exit.exit_code.unwrap()),
        Err(error) => error,
    };
    drop(runtime);
    fs::write(root.join("outcome"), outcome).unwrap();
}
