//! PTY handles and local session ownership, without event transport or worker threads.
//!
//! Hosts supply the PTY backend, session IDs, output readers, monitoring, and status/event
//! adapters. Cleanup covers the portable-pty child; descendant trees, elevation helpers,
//! bounded shutdown, and termination signals still require host/platform supervision.

use crate::runtime::{lock_err, PtyExitPayload};
use portable_pty::{CommandBuilder, PtySize, PtySystem};
use std::{
    collections::{hash_map::Entry, HashMap},
    io::{Read, Write},
    sync::{mpsc::Sender, Arc, Mutex},
};

pub struct PtySession {
    pub master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    pub writer: Mutex<Box<dyn Write + Send>>,
    pub child: Mutex<Box<dyn portable_pty::Child + Send>>,
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // No other owner remains. A poisoned lock must not abandon the child.
        let child = self.child.get_mut().unwrap_or_else(|err| err.into_inner());
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub struct SessionStore {
    // Public for compatibility with desktop adapters. New hosts should use the operations
    // below and never wait on children or deliver events while holding the registry lock.
    pub sessions: Mutex<HashMap<String, Arc<PtySession>>>,
    pub watchers: Mutex<HashMap<String, Vec<Sender<String>>>>,
    pub next_id: Mutex<u64>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            watchers: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        }
    }
}

impl SessionStore {
    pub fn remove(&self, id: &str) -> Result<Option<Arc<PtySession>>, String> {
        lock_err(&self.watchers)?.remove(id);
        Ok(lock_err(&self.sessions)?.remove(id))
    }

    pub fn write(&self, id: &str, data: &str) -> Result<(), String> {
        let session = lock_err(&self.sessions)?
            .get(id)
            .cloned()
            .ok_or_else(|| format!("session not found: {id}"))?;
        let mut writer = lock_err(&session.writer)?;
        writer
            .write_all(data.as_bytes())
            .map_err(|e| format!("failed to write pty input: {e}"))?;
        writer
            .flush()
            .map_err(|e| format!("failed to flush pty input: {e}"))?;
        Ok(())
    }

    pub fn close(&self, id: &str) -> Result<Option<PtyExitPayload>, String> {
        let Some(session) = self.remove(id)? else {
            return Ok(None);
        };
        let mut child = lock_err(&session.child)?;
        let _ = child.kill();
        let status = child
            .wait()
            .map_err(|e| format!("failed to wait child: {e}"))?;
        Ok(Some(PtyExitPayload {
            id: id.to_string(),
            exit_code: i32::try_from(status.exit_code()).ok(),
            reason: "closed_by_command".to_string(),
        }))
    }
}

/// Prepare and register one PTY. Platform work runs outside the session mutex.
/// Duplicate IDs never replace an existing owner, including concurrent launches.
/// The returned reader belongs to the host; dropping it does not close the session.
pub fn spawn_pty(
    system: &dyn PtySystem,
    store: &SessionStore,
    id: &str,
    cols: u16,
    rows: u16,
    cmd: CommandBuilder,
) -> Result<Box<dyn Read + Send>, String> {
    if lock_err(&store.sessions)?.contains_key(id) {
        return Err(format!("session id already in use: {id}"));
    }
    let pair = system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("failed to open pty: {e}"))?;
    // Complete fallible handle setup before a child can exist.
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("failed to clone pty reader: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("failed to get pty writer: {e}"))?;
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("failed to spawn process in pty: {e}"))?;
    drop(pair.slave);
    let session = Arc::new(PtySession {
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        child: Mutex::new(child),
    });
    // If locking or publication fails, the unregistered session kills/reaps its child.
    // The registry guard drops before the unregistered session on every error path.
    match lock_err(&store.sessions)?.entry(id.to_string()) {
        Entry::Vacant(entry) => {
            entry.insert(session);
        }
        Entry::Occupied(_) => return Err(format!("session id already in use: {id}")),
    }
    Ok(reader)
}
