//! CLI-local runtime ownership. Desktop hosts retain their existing adapters.
//!
//! This context owns children and offers foreground output drainage and cooperative cancellation.
//! Native I/O and cleanup can block; signals, deadlines, and process-tree supervision remain host work.

use crate::{
    profile::ProfileOwner,
    runtime::PtyExitPayload,
    sessions::{spawn_pty, SessionMonitor, SessionPoll, SessionStore},
};
use portable_pty::{CommandBuilder, PtySystem};
use std::{
    io::{self, Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use uuid::Uuid;

pub struct ProfileRuntime {
    // Fields drop in declaration order: all private session ownership must be released
    // before the profile lock. Never expose strong session handles from this context.
    sessions: SessionStore,
    nonce: Uuid,
    next_id: u64,
    owner: ProfileOwner,
}

/// Reader and weak exit observer for one launch; neither retains runtime ownership.
pub struct RuntimePty {
    pub reader: Box<dyn Read + Send>,
    id: String,
    monitor: SessionMonitor,
}

/// Cancellation may discard unread output; it never promises a complete transcript.
pub enum ForegroundOutcome {
    Completed(PtyExitPayload),
    Cancelled,
}

impl RuntimePty {
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl ProfileRuntime {
    pub fn acquire(config_dir: &Path) -> Result<Self, String> {
        let owner = ProfileOwner::acquire(config_dir)?;
        Ok(Self {
            sessions: SessionStore::default(),
            nonce: Uuid::new_v4(),
            next_id: 1,
            owner,
        })
    }

    pub fn credential_namespace(&self) -> &str {
        self.owner.credential_namespace()
    }

    /// Launches are serialized by mutable access. IDs are never reused in this runtime;
    /// a random per-runtime nonce separates subsequent owners of the same profile.
    /// Reserve the ID before platform work, including failed launches.
    pub fn spawn(
        &mut self,
        system: &dyn PtySystem,
        cols: u16,
        rows: u16,
        command: CommandBuilder,
    ) -> Result<RuntimePty, String> {
        let sequence = self.next_id;
        self.next_id = sequence
            .checked_add(1)
            .ok_or("runtime session IDs exhausted")?;
        let id = format!("cli-{}-{sequence}", self.nonce);
        let launched = spawn_pty(system, &self.sessions, &id, cols, rows, command)?;
        Ok(RuntimePty {
            reader: launched.reader,
            id,
            monitor: launched.monitor,
        })
    }

    /// Run one child synchronously, draining and flushing output before returning its exit.
    /// No status/event publication or input forwarding is performed. Reader, sink, and native
    /// child operations may block indefinitely (including descendants retaining the PTY).
    /// Errors and unwinding attempt direct-child cleanup while the profile remains owned.
    pub fn run_foreground(
        &mut self,
        system: &dyn PtySystem,
        cols: u16,
        rows: u16,
        command: CommandBuilder,
        output: &mut dyn Write,
    ) -> Result<PtyExitPayload, String> {
        let mut launched = self.spawn(system, cols, rows, command)?;
        let cleanup = ForegroundCleanup {
            runtime: self,
            id: launched.id().to_owned(),
        };
        let result = drain_foreground(self, &mut launched, output);
        cleanup.finish(result)
    }

    /// Drain output on a scoped worker while observing a host-owned cancellation flag.
    /// Cancellation closes the direct child before joining the worker. No worker is detached:
    /// blocked reads, writes, flushes and native calls can delay return indefinitely while
    /// retaining profile ownership. Cancellation discards remaining output and skips flush
    /// when observed by the worker. A write/flush already in progress may still finish.
    /// Completed output is fully drained and flushed before exit polling/retirement.
    /// Set the flag monotonically for this call; do not reset it until the call returns.
    /// Cancellation cannot interrupt launch. I/O/cleanup errors take precedence over cancellation.
    pub fn run_foreground_cancellable(
        &mut self,
        system: &dyn PtySystem,
        size: (u16, u16),
        command: CommandBuilder,
        output: &mut (dyn Write + Send),
        cancelled: &AtomicBool,
    ) -> Result<ForegroundOutcome, String> {
        if cancelled.load(Ordering::Acquire) {
            return Ok(ForegroundOutcome::Cancelled);
        }
        let launched = self.spawn(system, size.0, size.1, command)?;
        supervise_foreground(self, launched, output, cancelled)
    }

    pub fn write(&self, id: &str, data: &str) -> Result<(), String> {
        self.sessions.write(id, data)
    }

    pub fn close(&self, id: &str) -> Result<Option<PtyExitPayload>, String> {
        self.sessions.close(id)
    }

    /// Returns owned results after core locks are released. The caller decides how to
    /// publish them; this does not fence desktop callbacks or queued ID-only events.
    pub fn poll(&self, launched: &RuntimePty) -> Result<SessionPoll, String> {
        launched.monitor.poll(&self.sessions)
    }
}

// No strong session handles escape. This also runs if a host output writer panics.
struct ForegroundCleanup<'a> {
    runtime: &'a ProfileRuntime,
    id: String,
}

impl ForegroundCleanup<'_> {
    fn finish<T>(self, result: Result<T, String>) -> Result<T, String> {
        if let Err(primary) = result {
            self.runtime
                .close(&self.id)
                .map_err(|error| format!("{primary}; cleanup failed: {error}"))?;
            return Err(primary);
        }
        result
    }
}

impl Drop for ForegroundCleanup<'_> {
    fn drop(&mut self) {
        let _ = self.runtime.close(&self.id);
    }
}

fn drain_foreground(
    runtime: &ProfileRuntime,
    launched: &mut RuntimePty,
    output: &mut dyn Write,
) -> Result<PtyExitPayload, String> {
    io::copy(&mut launched.reader, output)
        .map_err(|error| format!("failed to stream pty output: {error}"))?;
    output
        .flush()
        .map_err(|error| format!("failed to flush pty output: {error}"))?;
    loop {
        match runtime.poll(launched)? {
            SessionPoll::Running => std::thread::sleep(Duration::from_millis(20)),
            SessionPoll::Detached => return Err("foreground session detached".to_string()),
            SessionPoll::Exited(exit) => return Ok(exit),
        }
    }
}

fn copy_cancellable(
    mut reader: Box<dyn Read + Send>,
    output: &mut (dyn Write + Send),
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let mut buffer = [0; 8192];
    while !cancelled.load(Ordering::Acquire) {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("failed to stream pty output: {error}")),
        };
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| format!("failed to stream pty output: {error}"))?;
    }
    if !cancelled.load(Ordering::Acquire) {
        output
            .flush()
            .map_err(|error| format!("failed to flush pty output: {error}"))?;
    }
    Ok(())
}

fn supervise_foreground(
    runtime: &ProfileRuntime,
    launched: RuntimePty,
    output: &mut (dyn Write + Send),
    cancelled: &AtomicBool,
) -> Result<ForegroundOutcome, String> {
    let RuntimePty {
        reader,
        id,
        monitor,
    } = launched;
    std::thread::scope(|scope| {
        // Close before scope's automatic join if the supervisor unwinds.
        let cleanup = ForegroundCleanup { runtime, id };
        let result = (|| {
            let worker = std::thread::Builder::new()
                .name("scp-foreground-output".into())
                .spawn_scoped(scope, move || copy_cancellable(reader, output, cancelled))
                .map_err(|error| format!("failed to start pty output worker: {error}"))?;
            join_foreground_output(runtime, &cleanup.id, worker, cancelled)?;
            loop {
                if cancelled.load(Ordering::Acquire) {
                    runtime.close(&cleanup.id)?;
                    return Ok(ForegroundOutcome::Cancelled);
                }
                match monitor.poll(&runtime.sessions)? {
                    SessionPoll::Running => std::thread::sleep(Duration::from_millis(20)),
                    SessionPoll::Detached => return Err("foreground session detached".into()),
                    SessionPoll::Exited(exit) => return Ok(ForegroundOutcome::Completed(exit)),
                }
            }
        })();
        cleanup.finish(result)
    })
}

fn join_foreground_output(
    runtime: &ProfileRuntime,
    id: &str,
    worker: std::thread::ScopedJoinHandle<'_, Result<(), String>>,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    while !cancelled.load(Ordering::Acquire) && !worker.is_finished() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let closed = if cancelled.load(Ordering::Acquire) {
        runtime.close(id).map(|_| ())
    } else {
        Ok(())
    };
    // Join even when close fails. Never release profile ownership around a live worker.
    let copied = match worker.join() {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    if let Err(primary) = copied {
        closed.map_err(|error| format!("{primary}; cleanup failed: {error}"))?;
        return Err(primary);
    }
    closed
}
