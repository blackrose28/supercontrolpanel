//! Thread-safe child process and PTY supervisor with bounded logs and ordered shutdown.
use crate::error::CoreError;
use parking_lot::{Condvar, Mutex};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Maximum log bytes retained per function in the in-memory ring buffer.
pub const MAX_LOG_RING_BYTES: usize = 64 * 1024;
static NEXT_SESSION_SEQ: AtomicU64 = AtomicU64::new(1);

/// Cooperative cancellation token shared across pre-accept launch gates.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Creates an active, uncancelled token.
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Marks the token cancelled; safe to call multiple times.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Child I/O mode managed by `ProcessSupervisor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
    Pty { cols: u16, rows: u16 },
    Pipe,
}

/// Launch specification for a supervised child process.
#[derive(Debug, Clone)]
pub struct ChildLaunch {
    pub function_id: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
    pub mode: SpawnMode,
    pub kill_tree: bool,
    pub kind: &'static str,
}

/// Outcome returned by the caller's atomic accept gate after spawning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptOutcome<T> {
    Accepted(T),
    CancelledBeforeAccept,
}

pub type ExitCallback = Arc<dyn Fn(&str, Option<i32>, &str) + Send + Sync>;
pub type OutputCallback = Arc<dyn Fn(&str, &str, bool) + Send + Sync>;

#[derive(Default)]
struct FunctionRing {
    bytes: Vec<u8>,
}

impl FunctionRing {
    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > MAX_LOG_RING_BYTES {
            let trim = self.bytes.len() - MAX_LOG_RING_BYTES;
            self.bytes.drain(..trim);
        }
    }

    fn tail_lines(&self) -> Vec<String> {
        let mut start = 0usize;
        while start < self.bytes.len() && (self.bytes[start] & 0b1100_0000) == 0b1000_0000 {
            start += 1;
        }
        let text = String::from_utf8_lossy(&self.bytes[start..]);
        text.lines().map(ToOwned::to_owned).collect()
    }
}

#[derive(Default)]
struct SupervisorState {
    sessions: HashMap<String, SessionEntry>,
    function_sessions: HashMap<String, String>,
    function_logs: HashMap<String, FunctionRing>,
}

struct ExitSignal {
    done: Mutex<Option<(Option<i32>, String)>>,
    cv: Condvar,
}

impl ExitSignal {
    fn new() -> Self {
        Self {
            done: Mutex::new(None),
            cv: Condvar::new(),
        }
    }

    fn mark_exited(&self, code: Option<i32>, reason: String) {
        *self.done.lock() = Some((code, reason));
        self.cv.notify_all();
    }

    fn wait_timeout(&self, timeout: Duration) -> Option<(Option<i32>, String)> {
        let deadline = Instant::now() + timeout;
        let mut guard = self.done.lock();
        while guard.is_none() {
            let Some(rem) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            if self.cv.wait_for(&mut guard, rem).timed_out() && guard.is_none() {
                break;
            }
        }
        guard.clone()
    }
}

struct SessionEntry {
    kind: &'static str,
    pid: Option<u32>,
    kill_tree: bool,
    control: SessionControl,
    exit_signal: Arc<ExitSignal>,
}

enum SessionControl {
    Pipe {
        stdin: Arc<Mutex<Option<ChildStdin>>>,
    },
    Pty {
        writer: Arc<Mutex<Box<dyn Write + Send>>>,
        master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
        killer: Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>,
    },
}

enum SpawnedChild {
    Pipe {
        child: Child,
        stdin: Option<ChildStdin>,
        stdout: Option<ChildStdout>,
        stderr: Option<ChildStderr>,
        pid: u32,
        kill_tree: bool,
    },
    Pty {
        child: Box<dyn portable_pty::Child + Send + Sync>,
        killer: Box<dyn ChildKiller + Send + Sync>,
        master: Box<dyn MasterPty + Send>,
        writer: Box<dyn Write + Send>,
        reader: Box<dyn Read + Send>,
        pid: Option<u32>,
        kill_tree: bool,
    },
}

/// Supervises active child processes and PTYs across runtime families.
#[derive(Default)]
pub struct ProcessSupervisor {
    state: Arc<Mutex<SupervisorState>>,
}

impl ProcessSupervisor {
    /// Creates an empty supervisor with no active child sessions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawns a child, evaluates `on_accept`, and reaps immediately if cancelled before accept.
    pub fn spawn_with_gate<G, E>(
        &self,
        launch: ChildLaunch,
        cancel: &CancelToken,
        on_accept: G,
        on_exit: E,
        on_output: Option<OutputCallback>,
    ) -> Result<AcceptOutcome<String>, CoreError>
    where
        G: FnOnce(&str) -> AcceptOutcome<()>,
        E: Fn(&str, Option<i32>, &str) + Send + Sync + 'static,
    {
        if cancel.is_cancelled() {
            return Ok(AcceptOutcome::CancelledBeforeAccept);
        }
        let session_id = next_session_id();
        let spawned = match launch.mode {
            SpawnMode::Pipe => spawn_pipe_child(&launch)?,
            SpawnMode::Pty { cols, rows } => spawn_pty_child(&launch, cols, rows)?,
        };
        match on_accept(&session_id) {
            AcceptOutcome::CancelledBeforeAccept => {
                terminate_unregistered_child(spawned);
                Ok(AcceptOutcome::CancelledBeforeAccept)
            }
            AcceptOutcome::Accepted(()) => {
                self.activate_spawned(
                    session_id.clone(),
                    launch,
                    spawned,
                    Arc::new(on_exit),
                    on_output,
                );
                Ok(AcceptOutcome::Accepted(session_id))
            }
        }
    }

    fn activate_spawned(
        &self,
        session_id: String,
        launch: ChildLaunch,
        spawned: SpawnedChild,
        on_exit: ExitCallback,
        on_output: Option<OutputCallback>,
    ) {
        let exit_signal = Arc::new(ExitSignal::new());
        let (entry, readers, waiter) = build_session_workers(
            &session_id,
            &launch,
            spawned,
            Arc::clone(&self.state),
            Arc::clone(&exit_signal),
            on_exit,
            on_output,
        );
        {
            let mut st = self.state.lock();
            st.function_logs
                .insert(launch.function_id.clone(), FunctionRing::default());
            st.function_sessions
                .insert(launch.function_id.clone(), session_id.clone());
            st.sessions.insert(session_id, entry);
        }
        let _waiter_handle = spawn_exit_waiter(waiter, readers);
    }

    /// Writes input bytes to an active PTY or pipe stdin.
    pub fn write(&self, session_id: &str, data: &str) -> Result<(), CoreError> {
        let ctrl = {
            let st = self.state.lock();
            let entry = st.sessions.get(session_id).ok_or_else(|| {
                CoreError::not_found(
                    "session_not_found",
                    format!("Unknown or closed session: {session_id}"),
                    "inspect_status",
                )
            })?;
            match &entry.control {
                SessionControl::Pipe { stdin } => Ok(Arc::clone(stdin)),
                SessionControl::Pty { writer, .. } => Err(Arc::clone(writer)),
            }
        };
        match ctrl {
            Ok(stdin_arc) => write_pipe_stdin(&stdin_arc, data.as_bytes()),
            Err(writer_arc) => write_pty_writer(&writer_arc, data.as_bytes()),
        }
    }

    /// Resizes an active PTY session; no-ops cleanly for pipe children.
    pub fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), CoreError> {
        let master = {
            let st = self.state.lock();
            let entry = st.sessions.get(session_id).ok_or_else(|| {
                CoreError::not_found(
                    "session_not_found",
                    format!("Unknown or closed session: {session_id}"),
                    "inspect_status",
                )
            })?;
            match &entry.control {
                SessionControl::Pipe { .. } => return Ok(()),
                SessionControl::Pty { master, .. } => Arc::clone(master),
            }
        };
        let res = master
            .lock()
            .resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| {
                CoreError::operation_failed(
                    "pty_resize_failed",
                    e.to_string(),
                    false,
                    "retry_later",
                )
            });
        res
    }

    /// Stops one session by ID without affecting any newer session for the same function.
    pub fn stop_session(&self, session_id: &str, timeout: Duration) -> Result<bool, CoreError> {
        let stop_ctx = {
            let st = self.state.lock();
            st.sessions.get(session_id).map(|entry| {
                let killer = match &entry.control {
                    SessionControl::Pty { killer, .. } => Some(Arc::clone(killer)),
                    SessionControl::Pipe { .. } => None,
                };
                (
                    entry.pid,
                    entry.kill_tree,
                    killer,
                    Arc::clone(&entry.exit_signal),
                )
            })
        };
        let Some((pid, kill_tree, killer, exit_signal)) = stop_ctx else {
            return Ok(false);
        };
        terminate_target(pid, kill_tree, killer.as_ref(), &exit_signal, timeout);
        Ok(true)
    }

    /// Stops the active session currently owned by `function_id`, if any.
    pub fn stop_function(&self, function_id: &str, timeout: Duration) -> Result<bool, CoreError> {
        let session_id = {
            let st = self.state.lock();
            st.function_sessions.get(function_id).cloned()
        };
        match session_id {
            Some(sid) => self.stop_session(&sid, timeout),
            None => Ok(false),
        }
    }

    /// Waits for `session_id` to exit, returning `true` if exited within `timeout`.
    pub fn wait_for_session_exit(&self, session_id: &str, timeout: Duration) -> bool {
        let signal = {
            let st = self.state.lock();
            st.sessions
                .get(session_id)
                .map(|entry| Arc::clone(&entry.exit_signal))
        };
        match signal {
            Some(sig) => sig.wait_timeout(timeout).is_some(),
            None => true,
        }
    }

    /// Stops active sessions in caller-supplied `kind_order`, then any remaining sessions.
    pub fn shutdown_all(&self, kind_order: &[&str], timeout: Duration) {
        let per_phase = timeout.max(Duration::from_millis(200));
        for kind in kind_order {
            let ids = self.session_ids_by_kind(Some(kind));
            self.stop_id_list(ids, per_phase);
        }
        let leftover = self.session_ids_by_kind(None);
        self.stop_id_list(leftover, per_phase);
    }

    fn session_ids_by_kind(&self, kind: Option<&str>) -> Vec<String> {
        let st = self.state.lock();
        st.sessions
            .iter()
            .filter(|(_, entry)| kind.is_none_or(|k| entry.kind == k))
            .map(|(sid, _)| sid.clone())
            .collect()
    }

    fn stop_id_list(&self, ids: Vec<String>, timeout: Duration) {
        for sid in ids {
            let _ = self.stop_session(&sid, timeout);
        }
    }

    /// Returns the number of currently registered active sessions.
    pub fn session_count(&self) -> usize {
        self.state.lock().sessions.len()
    }

    /// Returns the currently active session ID for `function_id`, if any.
    pub fn active_session_for_function(&self, function_id: &str) -> Option<String> {
        self.state
            .lock()
            .function_sessions
            .get(function_id)
            .cloned()
    }

    /// Returns the retained tail log lines for `function_id`.
    pub fn get_function_logs(&self, function_id: &str) -> Vec<String> {
        self.state
            .lock()
            .function_logs
            .get(function_id)
            .map(FunctionRing::tail_lines)
            .unwrap_or_default()
    }
}

impl Drop for ProcessSupervisor {
    fn drop(&mut self) {
        self.shutdown_all(&[], Duration::from_millis(300));
    }
}

fn next_session_id() -> String {
    let seq = NEXT_SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    format!("pty-{}-{seq}", &nonce[..8])
}

fn spawn_pipe_child(launch: &ChildLaunch) -> Result<SpawnedChild, CoreError> {
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &launch.cwd {
        cmd.current_dir(cwd);
    }
    for key in &launch.env_remove {
        cmd.env_remove(key);
    }
    for (k, v) in &launch.env {
        cmd.env(k, v);
    }
    #[cfg(unix)]
    if launch.kill_tree {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| map_spawn_error(&launch.program, e))?;
    let pid = child.id();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    Ok(SpawnedChild::Pipe {
        child,
        stdin,
        stdout,
        stderr,
        pid,
        kill_tree: launch.kill_tree,
    })
}

fn spawn_pty_child(launch: &ChildLaunch, cols: u16, rows: u16) -> Result<SpawnedChild, CoreError> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| {
            CoreError::operation_failed("pty_open_failed", e.to_string(), false, "retry_later")
        })?;
    let mut cmd = CommandBuilder::new(&launch.program);
    cmd.args(&launch.args);
    if let Some(cwd) = &launch.cwd {
        cmd.cwd(cwd);
    }
    for key in &launch.env_remove {
        cmd.env_remove(key);
    }
    for (k, v) in &launch.env {
        cmd.env(k, v);
    }
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| map_spawn_error(&launch.program, std::io::Error::other(e.to_string())))?;
    drop(pair.slave);
    let pid = child.process_id();
    let killer = child.clone_killer();
    let writer = pair.master.take_writer().map_err(|e| {
        CoreError::operation_failed("pty_writer_failed", e.to_string(), false, "retry_later")
    })?;
    let reader = pair.master.try_clone_reader().map_err(|e| {
        CoreError::operation_failed("pty_reader_failed", e.to_string(), false, "retry_later")
    })?;
    Ok(SpawnedChild::Pty {
        child,
        killer,
        master: pair.master,
        writer,
        reader,
        pid,
        kill_tree: launch.kill_tree,
    })
}

fn map_spawn_error(program: &str, err: std::io::Error) -> CoreError {
    if err.kind() == std::io::ErrorKind::NotFound {
        CoreError::capability_unavailable(
            "executable_not_found",
            format!("Required executable not found: {program}"),
            "install_or_configure_executable",
        )
    } else {
        CoreError::operation_failed(
            "spawn_failed",
            format!("Failed to launch {program}: {err}"),
            false,
            "inspect_configuration",
        )
    }
}

struct WaiterContext {
    session_id: String,
    function_id: String,
    state: Arc<Mutex<SupervisorState>>,
    exit_signal: Arc<ExitSignal>,
    on_exit: ExitCallback,
    wait_fn: Box<dyn FnOnce() -> (Option<i32>, String) + Send>,
}

fn build_session_workers(
    session_id: &str,
    launch: &ChildLaunch,
    spawned: SpawnedChild,
    state: Arc<Mutex<SupervisorState>>,
    exit_signal: Arc<ExitSignal>,
    on_exit: ExitCallback,
    on_output: Option<OutputCallback>,
) -> (SessionEntry, Vec<JoinHandle<()>>, WaiterContext) {
    match spawned {
        SpawnedChild::Pipe {
            child,
            stdin,
            stdout,
            stderr,
            pid,
            kill_tree,
        } => build_pipe_workers(
            session_id,
            launch,
            child,
            stdin,
            stdout,
            stderr,
            pid,
            kill_tree,
            state,
            exit_signal,
            on_exit,
            on_output,
        ),
        SpawnedChild::Pty {
            child,
            killer,
            master,
            writer,
            reader,
            pid,
            kill_tree,
        } => build_pty_workers(
            session_id,
            launch,
            child,
            killer,
            master,
            writer,
            reader,
            pid,
            kill_tree,
            state,
            exit_signal,
            on_exit,
            on_output,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_pipe_workers(
    session_id: &str,
    launch: &ChildLaunch,
    mut child: Child,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    pid: u32,
    kill_tree: bool,
    state: Arc<Mutex<SupervisorState>>,
    exit_signal: Arc<ExitSignal>,
    on_exit: ExitCallback,
    on_output: Option<OutputCallback>,
) -> (SessionEntry, Vec<JoinHandle<()>>, WaiterContext) {
    let mut readers = Vec::new();
    if let Some(out) = stdout {
        readers.push(spawn_reader(
            session_id.to_string(),
            launch.function_id.clone(),
            out,
            false,
            Arc::clone(&state),
            on_output.clone(),
        ));
    }
    if let Some(err) = stderr {
        readers.push(spawn_reader(
            session_id.to_string(),
            launch.function_id.clone(),
            err,
            false,
            Arc::clone(&state),
            on_output,
        ));
    }
    let entry = SessionEntry {
        kind: launch.kind,
        pid: Some(pid),
        kill_tree,
        control: SessionControl::Pipe {
            stdin: Arc::new(Mutex::new(stdin)),
        },
        exit_signal: Arc::clone(&exit_signal),
    };
    let wait_fn = Box::new(move || match child.wait() {
        Ok(status) => (status.code(), format!("exited: {status}")),
        Err(err) => (None, format!("wait error: {err}")),
    });
    let waiter = WaiterContext {
        session_id: session_id.to_string(),
        function_id: launch.function_id.clone(),
        state,
        exit_signal,
        on_exit,
        wait_fn,
    };
    (entry, readers, waiter)
}

#[allow(clippy::too_many_arguments)]
fn build_pty_workers(
    session_id: &str,
    launch: &ChildLaunch,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    reader: Box<dyn Read + Send>,
    pid: Option<u32>,
    kill_tree: bool,
    state: Arc<Mutex<SupervisorState>>,
    exit_signal: Arc<ExitSignal>,
    on_exit: ExitCallback,
    on_output: Option<OutputCallback>,
) -> (SessionEntry, Vec<JoinHandle<()>>, WaiterContext) {
    let readers = vec![spawn_reader(
        session_id.to_string(),
        launch.function_id.clone(),
        reader,
        true,
        Arc::clone(&state),
        on_output,
    )];
    let entry = SessionEntry {
        kind: launch.kind,
        pid,
        kill_tree,
        control: SessionControl::Pty {
            writer: Arc::new(Mutex::new(writer)),
            master: Arc::new(Mutex::new(master)),
            killer: Arc::new(Mutex::new(killer)),
        },
        exit_signal: Arc::clone(&exit_signal),
    };
    let wait_fn = Box::new(move || match child.wait() {
        Ok(status) => (
            Some(status.exit_code() as i32),
            format!("exited with code {}", status.exit_code()),
        ),
        Err(err) => (None, format!("wait error: {err}")),
    });
    let waiter = WaiterContext {
        session_id: session_id.to_string(),
        function_id: launch.function_id.clone(),
        state,
        exit_signal,
        on_exit,
        wait_fn,
    };
    (entry, readers, waiter)
}

fn spawn_reader<R: Read + Send + 'static>(
    session_id: String,
    function_id: String,
    mut reader: R,
    is_pty: bool,
    state: Arc<Mutex<SupervisorState>>,
    on_output: Option<OutputCallback>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    {
                        let mut st = state.lock();
                        st.function_logs
                            .entry(function_id.clone())
                            .or_default()
                            .push(chunk);
                    }
                    if let Some(cb) = &on_output {
                        let text = String::from_utf8_lossy(chunk);
                        cb(&session_id, &text, is_pty);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    })
}

fn spawn_exit_waiter(waiter: WaiterContext, readers: Vec<JoinHandle<()>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let (code, reason) = (waiter.wait_fn)();
        {
            let mut st = waiter.state.lock();
            st.sessions.remove(&waiter.session_id);
            if st
                .function_sessions
                .get(&waiter.function_id)
                .map(String::as_str)
                == Some(waiter.session_id.as_str())
            {
                st.function_sessions.remove(&waiter.function_id);
            }
        }
        for handle in readers {
            let _ = handle.join();
        }
        (waiter.on_exit)(&waiter.session_id, code, &reason);
        waiter.exit_signal.mark_exited(code, reason);
    })
}

fn terminate_unregistered_child(spawned: SpawnedChild) {
    match spawned {
        SpawnedChild::Pipe {
            mut child,
            pid,
            kill_tree,
            ..
        } => {
            #[cfg(unix)]
            signal_unix_pid(pid, kill_tree, libc::SIGKILL);
            #[cfg(not(unix))]
            let _ = (pid, kill_tree);
            let _ = child.kill();
            let _ = child.wait();
        }
        SpawnedChild::Pty {
            mut child,
            mut killer,
            pid,
            kill_tree,
            ..
        } => {
            #[cfg(unix)]
            if let Some(p) = pid {
                signal_unix_pid(p, kill_tree, libc::SIGKILL);
            }
            #[cfg(not(unix))]
            let _ = (pid, kill_tree);
            let _ = killer.kill();
            let _ = child.wait();
        }
    }
}

fn terminate_target(
    pid: Option<u32>,
    kill_tree: bool,
    killer: Option<&Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>>,
    exit_signal: &ExitSignal,
    timeout: Duration,
) {
    let term_wait = (timeout / 2).max(Duration::from_millis(50));
    #[cfg(unix)]
    if let Some(p) = pid {
        signal_unix_pid(p, kill_tree, libc::SIGTERM);
        if exit_signal.wait_timeout(term_wait).is_some() {
            return;
        }
        signal_unix_pid(p, kill_tree, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = (pid, kill_tree, term_wait);

    if let Some(killer_arc) = killer {
        let _ = killer_arc.lock().kill();
    }
    let _ = exit_signal.wait_timeout(timeout.max(Duration::from_millis(200)));
}

#[cfg(unix)]
fn signal_unix_pid(pid: u32, kill_tree: bool, sig: libc::c_int) {
    let target = if kill_tree {
        -(pid as libc::pid_t)
    } else {
        pid as libc::pid_t
    };
    // SAFETY: `target` is a positive PID or negated process-group ID owned by this supervisor.
    unsafe {
        libc::kill(target, sig);
    }
}

fn write_pipe_stdin(stdin: &Arc<Mutex<Option<ChildStdin>>>, data: &[u8]) -> Result<(), CoreError> {
    let mut guard = stdin.lock();
    let pipe = guard.as_mut().ok_or_else(|| {
        CoreError::operation_failed(
            "stdin_closed",
            "Process stdin is closed",
            false,
            "inspect_status",
        )
    })?;
    pipe.write_all(data)
        .and_then(|()| pipe.flush())
        .map_err(|e| {
            CoreError::operation_failed("write_failed", e.to_string(), false, "inspect_status")
        })
}

fn write_pty_writer(
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    data: &[u8],
) -> Result<(), CoreError> {
    let mut guard = writer.lock();
    guard
        .write_all(data)
        .and_then(|()| guard.flush())
        .map_err(|e| {
            CoreError::operation_failed("pty_write_failed", e.to_string(), false, "inspect_status")
        })
}
