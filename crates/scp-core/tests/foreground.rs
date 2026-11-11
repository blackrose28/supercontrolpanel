use portable_pty::CommandBuilder;
use scp_core::{host::ProfileRuntime, profile::ProfileOwner};
use std::{
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};

fn command(mode: &str) -> CommandBuilder {
    let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
    command.args([
        "--ignored",
        "--exact",
        "foreground_child_fixture",
        "--nocapture",
    ]);
    command.env("SCP_FOREGROUND_FIXTURE", mode);
    command
}

#[test]
#[ignore = "subprocess fixture exercised by foreground tests"]
fn foreground_child_fixture() {
    let mode = std::env::var("SCP_FOREGROUND_FIXTURE").unwrap();
    if mode == "complete" {
        for _ in 0..2048 {
            println!("{}", "x".repeat(64));
        }
        println!("FINAL-OUTPUT-MARKER");
        io::stdout().flush().unwrap();
        std::process::exit(7);
    }
    println!("CHILD-READY");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    std::process::exit(0);
}

struct Output {
    profile: PathBuf,
    bytes: Vec<u8>,
    failure: &'static str,
    flushed: bool,
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        assert!(ProfileOwner::acquire(&self.profile).is_err());
        match self.failure {
            "write" => return Err(io::Error::other("injected output write")),
            "panic" => panic!("injected output panic"),
            _ => {}
        }
        // Exercise write_all through partial sink writes.
        let count = bytes.len().min(31);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        assert!(ProfileOwner::acquire(&self.profile).is_err());
        if self.failure == "flush" {
            return Err(io::Error::other("injected output flush"));
        }
        self.flushed = true;
        Ok(())
    }
}

// Keep a wall-clock watchdog around real children, including cleanup error paths.
fn with_deadline(test: impl FnOnce() + Send + 'static) {
    let (send, recv) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        test();
        send.send(()).unwrap();
    });
    recv.recv_timeout(Duration::from_secs(20)).unwrap();
    worker.join().unwrap();
}

#[test]
fn foreground_drains_large_output_and_flushes_before_returning_nonzero_exit() {
    with_deadline(|| {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = ProfileRuntime::acquire(dir.path()).unwrap();
        let mut output = Output {
            profile: dir.path().into(),
            bytes: vec![],
            failure: "",
            flushed: false,
        };
        let exit = runtime
            .run_foreground(
                &*portable_pty::native_pty_system(),
                80,
                24,
                command("complete"),
                &mut output,
            )
            .unwrap();
        assert_eq!(exit.exit_code, Some(7));
        assert!(exit.id.starts_with("cli-"));
        assert_ne!(exit.reason, "closed_by_command");
        assert!(output.bytes.len() > 128 * 1024);
        assert!(String::from_utf8(output.bytes)
            .unwrap()
            .contains("FINAL-OUTPUT-MARKER"));
        assert!(output.flushed);
        assert!(runtime.close(&exit.id).unwrap().is_none());
        assert!(ProfileOwner::acquire(dir.path()).is_err());
        drop(runtime);
        assert!(ProfileOwner::acquire(dir.path()).is_ok());
    });
}

#[test]
fn foreground_output_errors_and_panics_cleanup_and_allow_another_launch() {
    with_deadline(|| {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = ProfileRuntime::acquire(dir.path()).unwrap();
        for failure in ["write", "flush", "panic"] {
            let mut output = Output {
                profile: dir.path().into(),
                bytes: vec![],
                failure,
                flushed: false,
            };
            let mode = if failure == "flush" {
                "complete"
            } else {
                "wait"
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.run_foreground(
                    &*portable_pty::native_pty_system(),
                    80,
                    24,
                    command(mode),
                    &mut output,
                )
            }));
            if failure == "panic" {
                assert!(result.is_err());
            } else {
                assert!(result
                    .unwrap()
                    .err()
                    .unwrap()
                    .contains(&format!("injected output {failure}")));
            }
            assert!(ProfileOwner::acquire(dir.path()).is_err());
        }
        let exit = runtime
            .run_foreground(
                &*portable_pty::native_pty_system(),
                80,
                24,
                command("complete"),
                &mut io::sink(),
            )
            .unwrap();
        assert_eq!(exit.exit_code, Some(7));
    });
}

#[test]
fn cancellable_foreground_drains_completed_output_and_cancels_a_waiting_child() {
    with_deadline(|| {
        use scp_core::host::ForegroundOutcome;
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = ProfileRuntime::acquire(dir.path()).unwrap();
        let cancelled = AtomicBool::new(false);
        let mut output = Output {
            profile: dir.path().into(),
            bytes: vec![],
            failure: "",
            flushed: false,
        };
        let result = runtime
            .run_foreground_cancellable(
                &*portable_pty::native_pty_system(),
                (80, 24),
                command("complete"),
                &mut output,
                &cancelled,
            )
            .unwrap();
        let ForegroundOutcome::Completed(exit) = result else {
            panic!("unexpected cancellation")
        };
        assert_eq!(exit.exit_code, Some(7));
        assert!(output.flushed);
        assert!(output.bytes.len() > 128 * 1024);
        assert!(String::from_utf8_lossy(&output.bytes).contains("FINAL-OUTPUT-MARKER"));
        assert!(runtime.close(&exit.id).unwrap().is_none());

        struct CancelOnOutput<'a>(&'a AtomicBool);
        impl Write for CancelOnOutput<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.store(true, Ordering::Release);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(matches!(
            runtime
                .run_foreground_cancellable(
                    &*portable_pty::native_pty_system(),
                    (80, 24),
                    command("wait"),
                    &mut CancelOnOutput(&cancelled),
                    &cancelled,
                )
                .unwrap(),
            ForegroundOutcome::Cancelled
        ));
        cancelled.store(false, Ordering::Release);
        output.failure = "flush";
        assert!(runtime
            .run_foreground_cancellable(
                &*portable_pty::native_pty_system(),
                (80, 24),
                command("complete"),
                &mut output,
                &cancelled,
            )
            .err()
            .unwrap()
            .contains("injected output flush"));
        assert!(ProfileOwner::acquire(dir.path()).is_err());
        drop(runtime);
        assert!(ProfileOwner::acquire(dir.path()).is_ok());
    });
}

#[test]
fn cancellation_joins_blocked_sink_before_releasing_profile() {
    with_deadline(|| {
        use scp_core::host::ForegroundOutcome;
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().to_owned();
        let cancelled = AtomicBool::new(false);
        let (entered, ready) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let (finished, completion) = mpsc::channel();
        struct BlockedSink {
            entered: mpsc::Sender<()>,
            released: mpsc::Receiver<()>,
        }
        impl Write for BlockedSink {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.entered.send(()).unwrap();
                self.released.recv().unwrap();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        std::thread::scope(|scope| {
            let cancellation = &cancelled;
            scope.spawn(move || {
                let mut runtime = ProfileRuntime::acquire(&profile).unwrap();
                let mut sink = BlockedSink { entered, released };
                let outcome = runtime
                    .run_foreground_cancellable(
                        &*portable_pty::native_pty_system(),
                        (80, 24),
                        command("wait"),
                        &mut sink,
                        cancellation,
                    )
                    .unwrap();
                assert!(matches!(outcome, ForegroundOutcome::Cancelled));
                drop(runtime);
                finished.send(()).unwrap();
            });
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            cancelled.store(true, Ordering::Release);
            assert!(completion.recv_timeout(Duration::from_millis(150)).is_err());
            assert!(ProfileOwner::acquire(dir.path()).is_err());
            release.send(()).unwrap();
            completion.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        assert!(ProfileOwner::acquire(dir.path()).is_ok());
    });
}
