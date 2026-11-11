use portable_pty::{
    Child, ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtyPair, PtySize, PtySystem,
    SlavePty,
};
use scp_core::sessions::{spawn_pty, SessionPoll, SessionStore};
use scp_core::{host::ProfileRuntime, profile::ProfileOwner};
use std::{
    io::{self, Read, Write},
    sync::{Arc, Barrier, Mutex},
};

#[derive(Default, Debug)]
struct Trace {
    calls: Vec<&'static str>,
    input: Vec<u8>,
}

#[derive(Clone, Default)]
struct FakePty {
    trace: Arc<Mutex<Trace>>,
    fail: &'static str,
    barrier: Option<Arc<Barrier>>,
    registry: Option<std::sync::Weak<SessionStore>>,
    poll_gate: Option<Arc<(Barrier, Barrier)>>,
    owned_profile: Option<std::path::PathBuf>,
}
impl FakePty {
    fn calls(&self) -> Vec<&'static str> {
        self.trace.lock().unwrap().calls.clone()
    }
    fn record(&self, call: &'static str) -> anyhow::Result<()> {
        if let Some(path) = &self.owned_profile {
            assert!(
                ProfileOwner::acquire(path).is_err(),
                "profile must stay owned during {call}"
            );
        }
        if let Some(store) = self.registry.as_ref().and_then(std::sync::Weak::upgrade) {
            assert!(
                !matches!(
                    store.sessions.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                "platform call {call} must run outside registry lock"
            );
        }
        self.trace.lock().unwrap().calls.push(call);
        if self.fail == call {
            anyhow::bail!("injected {call}");
        }
        Ok(())
    }
}
impl PtySystem for FakePty {
    fn openpty(&self, size: PtySize) -> anyhow::Result<PtyPair> {
        assert_eq!((size.cols, size.rows), (80, 24));
        self.record("open")?;
        Ok(PtyPair {
            master: Box::new(self.clone()),
            slave: Box::new(self.clone()),
        })
    }
}
impl MasterPty for FakePty {
    fn resize(&self, _: PtySize) -> anyhow::Result<()> {
        Ok(())
    }
    fn get_size(&self) -> anyhow::Result<PtySize> {
        Ok(PtySize::default())
    }
    fn try_clone_reader(&self) -> anyhow::Result<Box<dyn Read + Send>> {
        self.record("reader")?;
        if self.fail == "read" {
            return Ok(Box::new(FailedReader));
        }
        Ok(Box::new(io::Cursor::new(b"output")))
    }
    fn take_writer(&self) -> anyhow::Result<Box<dyn Write + Send>> {
        self.record("writer")?;
        Ok(Box::new(self.clone()))
    }
    #[cfg(unix)]
    fn process_group_leader(&self) -> Option<i32> {
        None
    }
    #[cfg(unix)]
    fn as_raw_fd(&self) -> Option<std::os::fd::RawFd> {
        None
    }
}
struct FailedReader;
impl Read for FailedReader {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("injected read"))
    }
}
impl Write for FakePty {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.record("write").map_err(io::Error::other)?;
        self.trace.lock().unwrap().input.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.record("flush").map_err(io::Error::other)
    }
}
impl SlavePty for FakePty {
    fn spawn_command(&self, _: CommandBuilder) -> anyhow::Result<Box<dyn Child + Send + Sync>> {
        self.record("spawn")?;
        if let Some(barrier) = &self.barrier {
            barrier.wait();
        }
        if self.fail == "publish" {
            let store = self.registry.as_ref().unwrap().upgrade().unwrap();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = store.sessions.lock().unwrap();
                panic!("poison publication fixture");
            }));
        }
        Ok(Box::new(FakeChild {
            pty: self.clone(),
            exited: false,
        }))
    }
}
#[derive(Debug)]
struct FakeChild {
    pty: FakePty,
    exited: bool,
}
impl std::fmt::Debug for FakePty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FakePty")
    }
}
impl ChildKiller for FakeChild {
    fn kill(&mut self) -> io::Result<()> {
        self.pty.record("kill").map_err(io::Error::other)
    }
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Self {
            pty: self.pty.clone(),
            exited: self.exited,
        })
    }
}
impl Child for FakeChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(gate) = self.pty.poll_gate.take() {
            gate.0.wait();
            gate.1.wait();
        }
        self.pty.record("poll").map_err(io::Error::other)?;
        Ok(self.exited.then(|| ExitStatus::with_exit_code(0)))
    }
    fn wait(&mut self) -> io::Result<ExitStatus> {
        self.pty.record("wait").map_err(io::Error::other)?;
        self.exited = true;
        Ok(ExitStatus::with_exit_code(0))
    }
    fn process_id(&self) -> Option<u32> {
        None
    }
    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        None
    }
}
fn launch(pty: &FakePty, store: &SessionStore, id: &str) -> Result<Box<dyn Read + Send>, String> {
    spawn_pty(pty, store, id, 80, 24, CommandBuilder::new("unused")).map(|launched| launched.reader)
}

#[test]
fn setup_failures_never_start_a_child_or_publish_a_session() {
    for (stage, message) in [
        ("open", "failed to open pty"),
        ("reader", "failed to clone pty reader"),
        ("writer", "failed to get pty writer"),
        ("spawn", "failed to spawn process in pty"),
    ] {
        let pty = FakePty {
            fail: stage,
            ..FakePty::default()
        };
        let store = SessionStore::default();
        assert!(launch(&pty, &store, "one")
            .err()
            .unwrap()
            .starts_with(message));
        assert!(store.sessions.lock().unwrap().is_empty());
        if stage != "spawn" {
            assert!(!pty.calls().contains(&"spawn"));
        }
        assert!(!pty.calls().contains(&"kill"));
    }
}

#[test]
fn duplicate_id_is_rejected_before_any_platform_work() {
    let store = SessionStore::default();
    let pty = FakePty::default();
    let _reader = launch(&pty, &store, "one").unwrap();
    let rejected = FakePty::default();
    assert_eq!(
        launch(&rejected, &store, "one").err().unwrap(),
        "session id already in use: one"
    );
    assert!(rejected.calls().is_empty());
    assert_eq!(store.sessions.lock().unwrap().len(), 1);
}

#[test]
fn competing_launches_do_not_replace_the_winner_and_reap_the_loser() {
    let store = Arc::new(SessionStore::default());
    let barrier = Arc::new(Barrier::new(2));
    let ptys = [
        FakePty {
            barrier: Some(barrier.clone()),
            registry: Some(Arc::downgrade(&store)),
            ..FakePty::default()
        },
        FakePty {
            barrier: Some(barrier),
            registry: Some(Arc::downgrade(&store)),
            ..FakePty::default()
        },
    ];
    let threads: Vec<_> = ptys
        .iter()
        .cloned()
        .map(|pty| {
            let store = store.clone();
            std::thread::spawn(move || launch(&pty, &store, "same"))
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    let winner = results.iter().position(Result::is_ok).unwrap();
    let loser = 1 - winner;
    assert_eq!(
        results[loser].as_ref().err().unwrap(),
        "session id already in use: same"
    );
    assert_eq!(store.sessions.lock().unwrap().len(), 1);
    assert!(!ptys[winner].calls().contains(&"kill"));
    assert!(ptys[loser].calls().ends_with(&["poll", "kill", "wait"]));
    store.close("same").unwrap();
    assert!(ptys[winner].calls().contains(&"wait"));
}

#[test]
fn io_watchers_and_close_keep_existing_contracts() {
    let store = SessionStore::default();
    let pty = FakePty::default();
    let mut reader = launch(&pty, &store, "one").unwrap();
    let mut output = String::new();
    reader.read_to_string(&mut output).unwrap();
    assert_eq!(output, "output");
    store.write("one", "hello\r").unwrap();
    assert_eq!(pty.trace.lock().unwrap().input, b"hello\r");
    let (sender, receiver) = std::sync::mpsc::channel();
    store.add_watcher("one", sender).unwrap();
    let exit = store.close("one").unwrap().unwrap();
    assert_eq!(
        (exit.id.as_str(), exit.exit_code, exit.reason.as_str()),
        ("one", Some(0), "closed_by_command")
    );
    assert!(receiver.recv().is_err());
    let (sender, receiver) = std::sync::mpsc::channel();
    assert_eq!(
        store.add_watcher("one", sender).unwrap_err(),
        "session not found: one"
    );
    assert!(receiver.recv().is_err());
    store.broadcast_watchers("one", "ignored").unwrap();
    assert!(store.close("one").unwrap().is_none());
    assert_eq!(
        store.write("one", "x").unwrap_err(),
        "session not found: one"
    );
    assert_eq!(pty.calls().iter().filter(|&&c| c == "kill").count(), 1);
}

#[test]
fn io_errors_are_explicit() {
    for (stage, message) in [
        ("write", "failed to write pty input"),
        ("flush", "failed to flush pty input"),
        ("wait", "failed to wait child"),
    ] {
        let pty = FakePty {
            fail: stage,
            ..FakePty::default()
        };
        let store = SessionStore::default();
        let _reader = launch(&pty, &store, "one").unwrap();
        let error = if stage == "wait" {
            store.close("one").err().unwrap()
        } else {
            store.write("one", "x").unwrap_err()
        };
        assert!(error.starts_with(message), "{error}");
    }
}

#[test]
fn last_owner_drop_reaps_live_children_even_with_a_poisoned_child_lock() {
    let pty = FakePty::default();
    let store = SessionStore::default();
    let _reader = launch(&pty, &store, "one").unwrap();
    let session = store.sessions.lock().unwrap()["one"].clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _child = session.child.lock().unwrap();
        panic!("poison fixture");
    }));
    drop(store);
    assert!(
        !pty.calls().contains(&"kill"),
        "remaining Arc still owns child"
    );
    drop(session);
    assert!(pty.calls().ends_with(&["poll", "kill", "wait"]));
}

#[test]
fn failed_publication_reaps_the_unregistered_child_outside_the_registry_lock() {
    let store = Arc::new(SessionStore::default());
    let pty = FakePty {
        fail: "publish",
        registry: Some(Arc::downgrade(&store)),
        ..FakePty::default()
    };
    assert_eq!(launch(&pty, &store, "one").err().unwrap(), "mutex poisoned");
    assert!(pty.calls().ends_with(&["poll", "kill", "wait"]));
    assert!(store.sessions.lock().err().unwrap().into_inner().is_empty());
}

#[test]
#[cfg(unix)]
fn native_pty_prepared_before_spawn_round_trips_input_and_reaps_on_close() {
    let store = SessionStore::default();
    let mut cmd = CommandBuilder::new("sh");
    cmd.args(["-c", "read line; printf 'received:%s\\n' \"$line\""]);
    let mut reader = spawn_pty(
        &*portable_pty::native_pty_system(),
        &store,
        "native",
        80,
        24,
        cmd,
    )
    .unwrap()
    .reader;
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut output = String::new();
        let result = reader.read_to_string(&mut output).map(|_| output);
        let _ = sender.send(result);
    });
    store.write("native", "hello\r").unwrap();
    let result = receiver.recv_timeout(std::time::Duration::from_secs(5));
    let session = store.sessions.lock().unwrap()["native"].clone();
    let exit = store.close("native").unwrap().unwrap();
    worker.join().unwrap();
    assert!(result.unwrap().unwrap().contains("received:hello"));
    assert_eq!(exit.reason, "closed_by_command");
    assert!(session.child.lock().unwrap().try_wait().unwrap().is_some());
}

#[test]
fn monitor_is_bound_to_launch_and_does_not_retain_child_ownership() {
    let store = SessionStore::default();
    let old = FakePty::default();
    let launched = spawn_pty(&old, &store, "same", 80, 24, CommandBuilder::new("unused")).unwrap();
    assert!(matches!(
        launched.monitor.poll(&store).unwrap(),
        SessionPoll::Running
    ));
    let retained_old = store.sessions.lock().unwrap()["same"].clone();
    store.close("same").unwrap();
    let replacement = FakePty::default();
    let _reader = launch(&replacement, &store, "same").unwrap();
    let before = replacement.calls();
    assert!(matches!(
        launched.monitor.poll(&store).unwrap(),
        SessionPoll::Detached
    ));
    assert_eq!(
        replacement.calls(),
        before,
        "stale monitor must not poll replacement"
    );
    assert_eq!(store.sessions.lock().unwrap().len(), 1);
    assert!(old.calls().contains(&"wait"));
    drop(retained_old);

    let live = FakePty::default();
    let launched = spawn_pty(&live, &store, "live", 80, 24, CommandBuilder::new("unused")).unwrap();
    drop(store);
    assert!(
        live.calls().ends_with(&["poll", "kill", "wait"]),
        "monitor must be weak"
    );
    drop(launched);
}

#[test]
fn monitor_reports_exit_once_and_releases_only_its_watchers() {
    let store = Arc::new(SessionStore::default());
    let pty = FakePty {
        registry: Some(Arc::downgrade(&store)),
        ..FakePty::default()
    };
    let launched = spawn_pty(&pty, &store, "one", 80, 24, CommandBuilder::new("unused")).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    store.add_watcher("one", sender).unwrap();
    store.broadcast_watchers("one", "hello").unwrap();
    assert_eq!(receiver.recv().unwrap(), "hello");
    let session = store.sessions.lock().unwrap()["one"].clone();
    session.child.lock().unwrap().wait().unwrap();
    let SessionPoll::Exited(payload) = launched.monitor.poll(&store).unwrap() else {
        panic!("expected exit")
    };
    assert_eq!(payload.id, "one");
    assert_eq!(payload.exit_code, Some(0));
    assert!(!payload.reason.is_empty());
    assert!(receiver.recv().is_err());
    assert!(store.sessions.lock().unwrap().is_empty());
    assert!(matches!(
        launched.monitor.poll(&store).unwrap(),
        SessionPoll::Detached
    ));
}

#[test]
fn monitor_wait_errors_keep_existing_payload_and_retire_session() {
    let store = SessionStore::default();
    let pty = FakePty {
        fail: "poll",
        ..FakePty::default()
    };
    let launched = spawn_pty(&pty, &store, "one", 80, 24, CommandBuilder::new("unused")).unwrap();
    let SessionPoll::Exited(payload) = launched.monitor.poll(&store).unwrap() else {
        panic!("expected wait error")
    };
    assert_eq!(payload.exit_code, None);
    assert_eq!(payload.reason, "wait_error: injected poll");
    assert!(store.sessions.lock().unwrap().is_empty());
    assert!(pty.calls().ends_with(&["poll", "kill", "wait"]));
}

#[test]
fn replacement_during_poll_keeps_its_child_and_watchers() {
    let store = Arc::new(SessionStore::default());
    let gate = Arc::new((Barrier::new(2), Barrier::new(2)));
    let old = FakePty {
        fail: "poll",
        poll_gate: Some(gate.clone()),
        ..FakePty::default()
    };
    let launched = spawn_pty(&old, &store, "same", 80, 24, CommandBuilder::new("unused")).unwrap();
    let worker_store = store.clone();
    let worker = std::thread::spawn(move || launched.monitor.poll(&worker_store));
    gate.0.wait();
    let removed = store.remove("same").unwrap();
    let replacement = FakePty::default();
    let _reader = launch(&replacement, &store, "same").unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    store.add_watcher("same", sender).unwrap();
    gate.1.wait();
    assert!(matches!(
        worker.join().unwrap().unwrap(),
        SessionPoll::Detached
    ));
    drop(removed);
    assert!(!replacement.calls().contains(&"poll"));
    assert_eq!(store.sessions.lock().unwrap().len(), 1);
    store.broadcast_watchers("same", "replacement").unwrap();
    assert_eq!(receiver.recv().unwrap(), "replacement");
    store.close("same").unwrap();
    assert!(receiver.recv().is_err());
}

#[test]
fn profile_runtime_retains_ownership_through_final_child_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = ProfileRuntime::acquire(temp.path()).unwrap();
    let namespace = runtime.credential_namespace().to_string();
    assert!(ProfileRuntime::acquire(temp.path()).is_err());
    let pty = FakePty {
        owned_profile: Some(temp.path().to_path_buf()),
        ..FakePty::default()
    };
    let launched = runtime
        .spawn(&pty, 80, 24, CommandBuilder::new("unused"))
        .unwrap();
    runtime.write(launched.id(), "hello").unwrap();
    assert!(matches!(
        runtime.poll(&launched).unwrap(),
        SessionPoll::Running
    ));
    drop(runtime);
    assert!(pty.calls().ends_with(&["poll", "kill", "wait"]));
    // Keeping a reader/weak monitor cannot extend either child or profile ownership.
    let replacement = ProfileRuntime::acquire(temp.path()).unwrap();
    assert_eq!(replacement.credential_namespace(), namespace);
    assert!(matches!(
        replacement.poll(&launched).unwrap(),
        SessionPoll::Detached
    ));
    assert!(temp.path().join("runtime-owner.lock").exists());
}

#[test]
fn profile_runtime_allocates_fresh_ids_and_preserves_close_contract() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = ProfileRuntime::acquire(temp.path()).unwrap();
    let pty = FakePty::default();
    let first = runtime
        .spawn(&pty, 80, 24, CommandBuilder::new("unused"))
        .unwrap();
    let exit = runtime.close(first.id()).unwrap().unwrap();
    assert_eq!(exit.id, first.id());
    assert_eq!(exit.reason, "closed_by_command");
    assert!(runtime.close(first.id()).unwrap().is_none());
    assert!(runtime
        .write(first.id(), "late")
        .unwrap_err()
        .contains("session not found"));
    let second = runtime
        .spawn(&pty, 80, 24, CommandBuilder::new("unused"))
        .unwrap();
    assert_ne!(first.id(), second.id());
    let calls = pty.calls();
    assert!(matches!(
        runtime.poll(&first).unwrap(),
        SessionPoll::Detached
    ));
    assert_eq!(pty.calls(), calls);
    drop(runtime);
    let mut next_runtime = ProfileRuntime::acquire(temp.path()).unwrap();
    let third = next_runtime
        .spawn(&pty, 80, 24, CommandBuilder::new("unused"))
        .unwrap();
    assert_ne!(second.id(), third.id());
    assert!(matches!(
        next_runtime.poll(&second).unwrap(),
        SessionPoll::Detached
    ));
}

#[test]
fn profile_runtime_failed_launch_retains_lock_and_can_retry() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = ProfileRuntime::acquire(temp.path()).unwrap();
    let failing = FakePty {
        fail: "spawn",
        ..FakePty::default()
    };
    assert!(runtime
        .spawn(&failing, 80, 24, CommandBuilder::new("unused"))
        .err()
        .unwrap()
        .starts_with("failed to spawn"));
    assert!(ProfileOwner::acquire(temp.path()).is_err());
    let pty = FakePty {
        fail: "poll",
        ..FakePty::default()
    };
    let launched = runtime
        .spawn(&pty, 80, 24, CommandBuilder::new("unused"))
        .unwrap();
    let SessionPoll::Exited(exit) = runtime.poll(&launched).unwrap() else {
        panic!("expected exit")
    };
    assert_eq!(exit.reason, "wait_error: injected poll");
    assert!(matches!(
        runtime.poll(&launched).unwrap(),
        SessionPoll::Detached
    ));
    drop(runtime);
    assert!(ProfileOwner::acquire(temp.path()).is_ok());
}

#[test]
fn foreground_failures_reap_before_return_or_unwind_and_report_cleanup_errors() {
    struct RejectedOutput(bool);
    impl Write for RejectedOutput {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            if self.0 {
                panic!("output panic fixture");
            }
            Err(io::Error::other("output rejected"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = ProfileRuntime::acquire(dir.path()).unwrap();
    for (panic, fail) in [(false, ""), (true, ""), (false, "wait"), (false, "read")] {
        let pty = FakePty {
            fail,
            owned_profile: Some(dir.path().to_owned()),
            ..FakePty::default()
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.run_foreground(
                &pty,
                80,
                24,
                CommandBuilder::new("unused"),
                &mut RejectedOutput(panic),
            )
        }));
        if panic {
            assert!(result.is_err());
        } else {
            let error = result.unwrap().err().unwrap();
            assert!(error.contains(if fail == "read" {
                "injected read"
            } else {
                "output rejected"
            }));
            assert_eq!(
                error.contains("cleanup failed: failed to wait child"),
                fail == "wait"
            );
        }
        let calls = pty.calls();
        assert!(calls.contains(&"kill"));
        assert!(calls.contains(&"wait"));
        assert!(ProfileOwner::acquire(dir.path()).is_err());
    }
}
