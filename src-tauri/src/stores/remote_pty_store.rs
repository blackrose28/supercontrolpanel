use std::{collections::HashMap, sync::Mutex};

const MAX_PENDING_OUTPUT_BYTES_PER_SESSION: usize = 64 * 1024;
const MAX_OUTPUT_FLUSH_BYTES: usize = 16 * 1024;
/// Ring buffer of recent PTY bytes for admin UI replay (source of truth on device).
const MAX_SCROLLBACK_BYTES_PER_SESSION: usize = 32 * 1024;
const SLOW_CONSUMER_NOTICE: &str =
    "\r\n[cc] PTY output truncated because the remote consumer is slow.\r\n";

pub const REMOTE_PTY_DETACH_GRACE_MS: u64 = 30_000;
pub const REMOTE_PTY_IDLE_TIMEOUT_MS: u64 = 5 * 60 * 1000;

#[derive(Clone)]
pub struct RemotePtyDetachSnapshot {
    pub remote_session_id: String,
    pub local_session_id: String,
    pub detach_generation: u64,
}

pub enum RemotePtyGcDecision {
    Cancel,
    Wait,
    Close { local_session_id: String },
}

#[derive(Default)]
struct RemotePtySessionState {
    local_session_id: String,
    pending_output: String,
    output_flush_in_progress: bool,
    output_truncated: bool,
    scrollback: String,
    scrollback_truncated: bool,
    detached_at_ms: Option<u64>,
    detach_generation: u64,
}

#[derive(Default)]
pub struct RemotePtyStore {
    /// CC-plane session id (typically `ccpty:{deviceId}:{localSegment}`) → device `SessionStore` id.
    sessions: Mutex<HashMap<String, RemotePtySessionState>>,
}

impl RemotePtyStore {
    pub fn bind_session(
        &self,
        remote_session_id: &str,
        local_session_id: &str,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;

        let stale_keys: Vec<String> = sessions
            .iter()
            .filter(|(_, v)| v.local_session_id == local_session_id)
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale_keys {
            sessions.remove(&k);
        }

        sessions.insert(
            remote_session_id.to_string(),
            RemotePtySessionState {
                local_session_id: local_session_id.to_string(),
                detached_at_ms: None,
                ..RemotePtySessionState::default()
            },
        );
        Ok(())
    }

    pub fn resolve_local_session(&self, remote_session_id: &str) -> Result<String, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;
        sessions
            .get(remote_session_id)
            .map(|session| session.local_session_id.clone())
            .ok_or_else(|| {
                format!("PTY_NOT_FOUND: Remote PTY session not found: {remote_session_id}")
            })
    }

    pub fn remote_session_for_local(&self, local_session_id: &str) -> Option<String> {
        let Ok(sessions) = self.sessions.lock() else {
            return None;
        };
        sessions
            .iter()
            .find(|(_, v)| v.local_session_id == local_session_id)
            .map(|(remote_id, _)| remote_id.clone())
    }

    pub fn unbind_session(
        &self,
        remote_session_id: &str,
        local_session_id: &str,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;
        match sessions.get(remote_session_id) {
            None => {}
            Some(entry) if entry.local_session_id != local_session_id => {
                return Err(format!(
                    "remote PTY binding mismatch for {remote_session_id}: expected local {local_session_id}, found {}",
                    entry.local_session_id
                ));
            }
            Some(_) => {
                sessions.remove(remote_session_id);
            }
        }
        Ok(())
    }

    /// Removes the session row when `remote_session_id` maps to `local_session_id`.
    /// Returns `Some(remote_session_id)` when a row was removed, `None` if no matching binding existed.
    pub fn clear_session_binding(
        &self,
        remote_session_id: &str,
        local_session_id: &str,
    ) -> Result<Option<String>, String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;
        match sessions.get(remote_session_id) {
            None => Ok(None),
            Some(entry) if entry.local_session_id != local_session_id => Ok(None),
            Some(_) => {
                sessions.remove(remote_session_id);
                Ok(Some(remote_session_id.to_string()))
            }
        }
    }

    pub fn enqueue_output(&self, remote_session_id: &str, data: &str) -> Result<bool, String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;
        let Some(session) = sessions.get_mut(remote_session_id) else {
            return Ok(false);
        };

        append_scrollback_ring(&mut session.scrollback, &mut session.scrollback_truncated, data);

        let available =
            MAX_PENDING_OUTPUT_BYTES_PER_SESSION.saturating_sub(session.pending_output.len());
        if available > 0 {
            let appended = truncate_to_char_boundary(data, available);
            session.pending_output.push_str(appended);
            if appended.len() < data.len() {
                session.output_truncated = true;
            }
        } else {
            session.output_truncated = true;
        }

        if session.output_flush_in_progress {
            return Ok(false);
        }

        session.output_flush_in_progress = true;
        Ok(true)
    }

    pub fn take_output_chunk(&self, remote_session_id: &str) -> Result<Option<String>, String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;
        let Some(session) = sessions.get_mut(remote_session_id) else {
            return Ok(None);
        };

        if !session.pending_output.is_empty() {
            if session.pending_output.len() <= MAX_OUTPUT_FLUSH_BYTES {
                return Ok(Some(std::mem::take(&mut session.pending_output)));
            }

            let split_at =
                floor_char_boundary(&session.pending_output, MAX_OUTPUT_FLUSH_BYTES).max(1);
            let remainder = session.pending_output.split_off(split_at);
            let chunk = std::mem::replace(&mut session.pending_output, remainder);
            return Ok(Some(chunk));
        }

        if session.output_truncated {
            session.output_truncated = false;
            return Ok(Some(SLOW_CONSUMER_NOTICE.to_string()));
        }

        session.output_flush_in_progress = false;
        Ok(None)
    }

    /// Snapshot of scrollback for the CC remote session id (admin replay). `None` if unbound.
    pub fn scrollback_for_remote_session(
        &self,
        remote_session_id: &str,
    ) -> Result<Option<(String, bool)>, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;
        let Some(session) = sessions.get(remote_session_id) else {
            return Ok(None);
        };
        Ok(Some((
            session.scrollback.clone(),
            session.scrollback_truncated,
        )))
    }

    pub fn abort_output_flush(&self, remote_session_id: &str) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;
        if let Some(session) = sessions.get_mut(remote_session_id) {
            session.output_flush_in_progress = false;
        }
        Ok(())
    }

    pub fn mark_all_attached(&self) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;
        for session in sessions.values_mut() {
            session.detached_at_ms = None;
            session.detach_generation = session.detach_generation.saturating_add(1);
        }
        Ok(())
    }

    pub fn mark_all_detached(
        &self,
        detached_at_ms: u64,
    ) -> Result<Vec<RemotePtyDetachSnapshot>, String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;
        let mut snapshots = Vec::with_capacity(sessions.len());
        for (remote_session_id, session) in sessions.iter_mut() {
            session.detached_at_ms = Some(detached_at_ms);
            session.detach_generation = session.detach_generation.saturating_add(1);
            snapshots.push(RemotePtyDetachSnapshot {
                remote_session_id: remote_session_id.clone(),
                local_session_id: session.local_session_id.clone(),
                detach_generation: session.detach_generation,
            });
        }
        Ok(snapshots)
    }

    pub fn gc_decision(
        &self,
        remote_session_id: &str,
        detach_generation: u64,
        now_ms: u64,
    ) -> Result<RemotePtyGcDecision, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "remote pty sessions lock poisoned".to_string())?;
        let Some(session) = sessions.get(remote_session_id) else {
            return Ok(RemotePtyGcDecision::Cancel);
        };
        if session.detach_generation != detach_generation {
            return Ok(RemotePtyGcDecision::Cancel);
        }
        let Some(detached_at_ms) = session.detached_at_ms else {
            return Ok(RemotePtyGcDecision::Cancel);
        };
        if now_ms < detached_at_ms.saturating_add(REMOTE_PTY_IDLE_TIMEOUT_MS) {
            return Ok(RemotePtyGcDecision::Wait);
        }
        Ok(RemotePtyGcDecision::Close {
            local_session_id: session.local_session_id.clone(),
        })
    }
}

fn append_scrollback_ring(target: &mut String, truncated: &mut bool, data: &str) {
    if data.is_empty() {
        return;
    }
    target.push_str(data);
    if target.len() <= MAX_SCROLLBACK_BYTES_PER_SESSION {
        return;
    }
    let excess = target.len() - MAX_SCROLLBACK_BYTES_PER_SESSION;
    let slice_from = floor_char_boundary(target, excess).max(1);
    *target = target[slice_from..].to_string();
    *truncated = true;
}

fn truncate_to_char_boundary(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    &input[..floor_char_boundary(input, max_bytes)]
}

fn floor_char_boundary(input: &str, max_bytes: usize) -> usize {
    let mut boundary = max_bytes.min(input.len());
    while boundary > 0 && !input.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_session_for_local_resolves_after_bind() {
        let store = RemotePtyStore::default();
        let remote = "ccpty:dev-1:pty-7";
        store.bind_session(remote, "pty-7").expect("bind");
        assert_eq!(store.remote_session_for_local("pty-7").as_deref(), Some(remote));
        assert_eq!(store.resolve_local_session(remote).unwrap(), "pty-7");
    }

    #[test]
    fn bind_session_evicts_prior_remote_for_same_local() {
        let store = RemotePtyStore::default();
        store.bind_session("ccpty:dev:a", "pty-1").expect("bind a");
        store.bind_session("ccpty:dev:b", "pty-1").expect("bind b");
        assert_eq!(
            store.remote_session_for_local("pty-1").as_deref(),
            Some("ccpty:dev:b")
        );
        assert!(store.resolve_local_session("ccpty:dev:a").is_err());
    }

    #[test]
    fn clear_session_binding_requires_matching_local() {
        let store = RemotePtyStore::default();
        store.bind_session("ccpty:d:1", "local-1").expect("bind");
        assert_eq!(
            store
                .clear_session_binding("ccpty:d:1", "wrong-local")
                .expect("clear"),
            None
        );
        assert!(store.resolve_local_session("ccpty:d:1").is_ok());
        assert_eq!(
            store
                .clear_session_binding("ccpty:d:1", "local-1")
                .expect("clear"),
            Some("ccpty:d:1".to_string())
        );
        assert!(store.resolve_local_session("ccpty:d:1").is_err());
    }

    #[test]
    fn unbind_session_errors_on_local_mismatch() {
        let store = RemotePtyStore::default();
        store.bind_session("ccpty:d:1", "local-1").expect("bind");
        assert!(store.unbind_session("ccpty:d:1", "other-local").is_err());
        assert!(store.resolve_local_session("ccpty:d:1").is_ok());
    }

    #[test]
    fn scrollback_ring_keeps_tail_and_sets_truncated() {
        let store = RemotePtyStore::default();
        store
            .bind_session("remote-1", "local-1")
            .expect("bind");
        let chunk = "a".repeat(20_000);
        store
            .enqueue_output("remote-1", &chunk)
            .expect("enqueue");
        store
            .enqueue_output("remote-1", &chunk)
            .expect("enqueue");
        let snap = store
            .scrollback_for_remote_session("remote-1")
            .expect("snap")
            .expect("some");
        assert!(snap.1);
        assert!(snap.0.len() <= MAX_SCROLLBACK_BYTES_PER_SESSION);
        assert!(snap.0.ends_with('a'));
    }
}
