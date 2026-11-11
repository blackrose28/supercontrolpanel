//! CLI-local runtime ownership. Desktop hosts retain their existing adapters.
//!
//! This context owns children and offers synchronous foreground output drainage. Native I/O
//! and cleanup can block; signals, bounded cancellation, and process-tree supervision remain host work.

use crate::{
    profile::ProfileOwner,
    runtime::PtyExitPayload,
    sessions::{spawn_pty, SessionMonitor, SessionPoll, SessionStore},
};
use portable_pty::{CommandBuilder, PtySystem};
use std::{
    io::{self, Read, Write},
    path::Path,
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
        if let Err(primary) = result {
            cleanup
                .runtime
                .close(&cleanup.id)
                .map_err(|error| format!("{primary}; cleanup failed: {error}"))?;
            return Err(primary);
        }
        result
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
