use crate::SpikeError;
use portable_pty::{Child as PtyChild, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const MAX_RETAINED_BYTES: usize = 65_536;
const GRACEFUL_WAIT_CAP: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildKind {
    Pipe,
    Pty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnershipOutcome {
    Accepted(String),
    CancelledBeforeAccept,
}

#[derive(Default)]
struct BoundedOutput {
    bytes: VecDeque<u8>,
}

impl BoundedOutput {
    fn push(&mut self, chunk: &[u8]) {
        if chunk.len() >= MAX_RETAINED_BYTES {
            self.bytes.clear();
            self.bytes
                .extend(&chunk[chunk.len() - MAX_RETAINED_BYTES..]);
            return;
        }
        let overflow = (self.bytes.len() + chunk.len()).saturating_sub(MAX_RETAINED_BYTES);
        for _ in 0..overflow {
            self.bytes.pop_front();
        }
        self.bytes.extend(chunk);
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn lossy_string(&self) -> String {
        let (front, back) = self.bytes.as_slices();
        let mut combined = Vec::with_capacity(front.len() + back.len());
        combined.extend_from_slice(front);
        combined.extend_from_slice(back);
        String::from_utf8_lossy(&combined).into_owned()
    }
}

enum ChildProcess {
    Pipe(Child),
    Pty {
        child: Box<dyn PtyChild + Send + Sync>,
        master: Option<Box<dyn MasterPty + Send>>,
    },
}

impl ChildProcess {
    fn pid(&self) -> u32 {
        match self {
            Self::Pipe(child) => child.id(),
            Self::Pty { child, .. } => child.process_id().unwrap_or(0),
        }
    }

    #[cfg(unix)]
    fn signal_group_or_pid(&self, sig: i32) {
        let pid = self.pid();
        if pid == 0 {
            return;
        }
        // SAFETY: Valid positive PID converted to process-group / PID signal target.
        let pg_rc = unsafe { libc::kill(-(pid as libc::pid_t), sig) };
        if pg_rc != 0 {
            unsafe {
                libc::kill(pid as libc::pid_t, sig);
            }
        }
    }

    fn request_graceful_stop(&mut self) {
        #[cfg(unix)]
        self.signal_group_or_pid(libc::SIGTERM);
        #[cfg(not(unix))]
        match self {
            Self::Pipe(child) => {
                let _ = child.kill();
            }
            Self::Pty { child, .. } => {
                let _ = child.kill();
            }
        }
    }

    fn try_wait(&mut self) -> bool {
        match self {
            Self::Pipe(child) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
            Self::Pty { child, .. } => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
        }
    }

    fn force_kill_and_reap(&mut self) {
        #[cfg(unix)]
        self.signal_group_or_pid(libc::SIGKILL);
        match self {
            Self::Pipe(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Self::Pty { child, master } => {
                let _ = child.kill();
                drop(master.take());
                let _ = child.wait();
            }
        }
    }

    fn close_master(&mut self) {
        if let Self::Pty { master, .. } = self {
            drop(master.take());
        }
    }
}

struct ManagedSession {
    pid: u32,
    request_cancelled: Arc<AtomicBool>,
    output: Arc<Mutex<BoundedOutput>>,
    child: ChildProcess,
    readers: Vec<JoinHandle<()>>,
}

impl ManagedSession {
    fn request_graceful_stop(&mut self) {
        self.child.request_graceful_stop();
    }

    fn finish_reap(mut self, deadline: Instant) {
        while Instant::now() < deadline {
            if self.child.try_wait() {
                self.child.close_master();
                for reader in self.readers.drain(..) {
                    let _ = reader.join();
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        if !self.child.try_wait() {
            self.child.force_kill_and_reap();
        } else {
            self.child.close_master();
        }
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}

#[derive(Default)]
pub struct Supervisor {
    next_id: AtomicU64,
    shutting_down: AtomicBool,
    sessions: Mutex<HashMap<String, ManagedSession>>,
}

impl Supervisor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn_and_register(
        &self,
        kind: ChildKind,
        program: &str,
        args: &[&str],
        pre_accept_cancel: bool,
    ) -> Result<OwnershipOutcome, SpikeError> {
        let output = Arc::new(Mutex::new(BoundedOutput::default()));
        let managed = match kind {
            ChildKind::Pipe => spawn_pipe_child(program, args, Arc::clone(&output))?,
            ChildKind::Pty => spawn_pty_child(program, args, Arc::clone(&output))?,
        };

        let seq = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let id = format!("sess-{seq}");
        let mut pending = Some(managed);
        let cancelled = {
            let mut guard = self.sessions.lock().map_err(|_| {
                SpikeError::new(
                    "OPERATION_FAILED",
                    "supervisor_lock_poisoned",
                    "lock poisoned",
                    1,
                )
            })?;
            let is_cancelled = pre_accept_cancel
                || self.shutting_down.load(Ordering::SeqCst)
                || pending
                    .as_ref()
                    .is_some_and(|s| s.request_cancelled.load(Ordering::SeqCst));
            if !is_cancelled && let Some(session) = pending.take() {
                guard.insert(id.clone(), session);
            }
            is_cancelled
        };

        if cancelled {
            if let Some(mut session) = pending {
                session.request_graceful_stop();
                session.finish_reap(Instant::now() + GRACEFUL_WAIT_CAP);
            }
            return Ok(OwnershipOutcome::CancelledBeforeAccept);
        }
        Ok(OwnershipOutcome::Accepted(id))
    }

    pub fn cancel_request_token(&self, session_id: &str) {
        if let Ok(guard) = self.sessions.lock()
            && let Some(session) = guard.get(session_id)
        {
            session.request_cancelled.store(true, Ordering::SeqCst);
        }
    }

    pub fn stop_session(&self, session_id: &str) -> Result<(), SpikeError> {
        let session = {
            let mut guard = self.sessions.lock().map_err(|_| {
                SpikeError::new(
                    "OPERATION_FAILED",
                    "supervisor_lock_poisoned",
                    "lock poisoned",
                    1,
                )
            })?;
            guard.remove(session_id)
        };
        let Some(mut session) = session else {
            return Err(SpikeError::new(
                "NOT_FOUND",
                "session_not_found",
                format!("No active supervised session with id {session_id}"),
                1,
            ));
        };
        session.request_graceful_stop();
        session.finish_reap(Instant::now() + GRACEFUL_WAIT_CAP);
        Ok(())
    }

    pub fn active_sessions(&self) -> Vec<(String, u32)> {
        let Ok(guard) = self.sessions.lock() else {
            return Vec::new();
        };
        let mut out: Vec<(String, u32)> = guard.iter().map(|(id, s)| (id.clone(), s.pid)).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn output_len(&self, session_id: &str) -> Option<usize> {
        let out_arc = {
            let guard = self.sessions.lock().ok()?;
            Arc::clone(&guard.get(session_id)?.output)
        };
        let len = out_arc.lock().ok()?.len();
        Some(len)
    }

    pub fn output_snapshot(&self, session_id: &str) -> Option<String> {
        let out_arc = {
            let guard = self.sessions.lock().ok()?;
            Arc::clone(&guard.get(session_id)?.output)
        };
        let text = out_arc.lock().ok()?.lossy_string();
        Some(text)
    }

    pub fn shutdown(&self, timeout: Duration) -> usize {
        let mut drained = {
            let Ok(mut guard) = self.sessions.lock() else {
                self.shutting_down.store(true, Ordering::SeqCst);
                return 0;
            };
            self.shutting_down.store(true, Ordering::SeqCst);
            std::mem::take(&mut *guard)
        };
        let count = drained.len();
        if count == 0 {
            return 0;
        }
        for session in drained.values_mut() {
            session.request_graceful_stop();
        }
        let deadline = Instant::now() + timeout.min(GRACEFUL_WAIT_CAP);
        for (_, session) in drained {
            session.finish_reap(deadline);
        }
        count
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        let _ = self.shutdown(GRACEFUL_WAIT_CAP);
    }
}

fn spawn_reader_thread<R: Read + Send + 'static>(
    mut reader: R,
    output: Arc<Mutex<BoundedOutput>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut guard) = output.lock() {
                        guard.push(&buf[..n]);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    })
}

fn spawn_pipe_child(
    program: &str,
    args: &[&str],
    output: Arc<Mutex<BoundedOutput>>,
) -> Result<ManagedSession, SpikeError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| {
        SpikeError::new(
            "OPERATION_FAILED",
            "child_spawn_failed",
            format!("Failed to spawn pipe child `{program}`: {e}"),
            1,
        )
    })?;
    let pid = child.id();
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(spawn_reader_thread(stdout, Arc::clone(&output)));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(spawn_reader_thread(stderr, Arc::clone(&output)));
    }
    Ok(ManagedSession {
        pid,
        request_cancelled: Arc::new(AtomicBool::new(false)),
        output,
        child: ChildProcess::Pipe(child),
        readers,
    })
}

fn spawn_pty_child(
    program: &str,
    args: &[&str],
    output: Arc<Mutex<BoundedOutput>>,
) -> Result<ManagedSession, SpikeError> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize::default()).map_err(|e| {
        SpikeError::new(
            "OPERATION_FAILED",
            "pty_open_failed",
            format!("Failed to allocate PTY: {e}"),
            1,
        )
    })?;
    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    let child = pair.slave.spawn_command(cmd).map_err(|e| {
        SpikeError::new(
            "OPERATION_FAILED",
            "pty_spawn_failed",
            format!("Failed to spawn PTY child `{program}`: {e}"),
            1,
        )
    })?;
    // Close slave fd in the parent immediately so EOF propagates when child exits.
    drop(pair.slave);

    let pid = child.process_id().unwrap_or(0);
    let reader = pair.master.try_clone_reader().map_err(|e| {
        SpikeError::new(
            "OPERATION_FAILED",
            "pty_reader_failed",
            format!("Failed to open PTY reader: {e}"),
            1,
        )
    })?;
    let reader_handle = spawn_reader_thread(reader, Arc::clone(&output));
    Ok(ManagedSession {
        pid,
        request_cancelled: Arc::new(AtomicBool::new(false)),
        output,
        child: ChildProcess::Pty {
            child,
            master: Some(pair.master),
        },
        readers: vec![reader_handle],
    })
}
