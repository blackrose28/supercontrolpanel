use portable_pty::{
    Child, ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtyPair, PtySize, PtySystem,
    SlavePty,
};
use scp_core::sessions::{spawn_pty, SessionStore};
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
}
impl FakePty {
    fn calls(&self) -> Vec<&'static str> {
        self.trace.lock().unwrap().calls.clone()
    }
    fn record(&self, call: &'static str) -> anyhow::Result<()> {
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
    spawn_pty(pty, store, id, 80, 24, CommandBuilder::new("unused"))
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
    store
        .watchers
        .lock()
        .unwrap()
        .insert("one".into(), vec![sender]);
    let exit = store.close("one").unwrap().unwrap();
    assert_eq!(
        (exit.id.as_str(), exit.exit_code, exit.reason.as_str()),
        ("one", Some(0), "closed_by_command")
    );
    assert!(receiver.recv().is_err());
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
    .unwrap();
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
