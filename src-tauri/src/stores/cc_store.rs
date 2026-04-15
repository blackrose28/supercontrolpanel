use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct TransferRuntimeContext {
    pub transfer_id: String,
    pub direction: TransferRuntimeDirection,
    pub server_id: String,
    pub local_path: String,
    pub remote_path: String,
    pub temp_path_hint: Option<String>,
    pub resume_token: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TransferRuntimeDirection {
    Upload,
    Download,
}

pub struct CcRuntimeState {
    connected: AtomicBool,
    generation: AtomicU64,
    scp_session_context: Mutex<HashMap<String, TransferRuntimeContext>>,
    scp_session_last_activity: Mutex<HashMap<String, Instant>>,
    scp_session_progress_state: Mutex<HashMap<String, TransferProgressState>>,
}

struct TransferProgressState {
    last_progress_at: Instant,
    last_percent: Option<u16>,
    last_bytes_transferred: Option<u64>,
}

impl CcRuntimeState {
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Relaxed);
    }

    pub fn begin_generation(&self) -> u64 {
        self.connected.store(false, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn is_generation_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Relaxed) == generation
    }

    pub fn bind_transfer_session(
        &self,
        session_id: &str,
        context: TransferRuntimeContext,
    ) -> Result<(), String> {
        let mut guard = self
            .scp_session_context
            .lock()
            .map_err(|_| "cc runtime transfer map lock poisoned".to_string())?;
        guard.insert(session_id.to_string(), context);
        let mut activity_guard = self
            .scp_session_last_activity
            .lock()
            .map_err(|_| "cc runtime transfer activity map lock poisoned".to_string())?;
        activity_guard.insert(session_id.to_string(), Instant::now());
        let mut progress_guard = self
            .scp_session_progress_state
            .lock()
            .map_err(|_| "cc runtime transfer progress map lock poisoned".to_string())?;
        progress_guard.insert(
            session_id.to_string(),
            TransferProgressState {
                last_progress_at: Instant::now(),
                last_percent: None,
                last_bytes_transferred: None,
            },
        );
        Ok(())
    }

    pub fn transfer_id_for_session(&self, session_id: &str) -> Result<Option<String>, String> {
        let guard = self
            .scp_session_context
            .lock()
            .map_err(|_| "cc runtime transfer map lock poisoned".to_string())?;
        Ok(guard.get(session_id).map(|context| context.transfer_id.clone()))
    }

    pub fn transfer_context_for_session(
        &self,
        session_id: &str,
    ) -> Result<Option<TransferRuntimeContext>, String> {
        let guard = self
            .scp_session_context
            .lock()
            .map_err(|_| "cc runtime transfer map lock poisoned".to_string())?;
        Ok(guard.get(session_id).cloned())
    }

    pub fn unbind_transfer_session(
        &self,
        session_id: &str,
    ) -> Result<Option<TransferRuntimeContext>, String> {
        let mut guard = self
            .scp_session_context
            .lock()
            .map_err(|_| "cc runtime transfer map lock poisoned".to_string())?;
        let removed = guard.remove(session_id);
        let mut activity_guard = self
            .scp_session_last_activity
            .lock()
            .map_err(|_| "cc runtime transfer activity map lock poisoned".to_string())?;
        activity_guard.remove(session_id);
        let mut progress_guard = self
            .scp_session_progress_state
            .lock()
            .map_err(|_| "cc runtime transfer progress map lock poisoned".to_string())?;
        progress_guard.remove(session_id);
        Ok(removed)
    }

    pub fn note_transfer_activity(&self, session_id: &str) -> Result<(), String> {
        let mut guard = self
            .scp_session_last_activity
            .lock()
            .map_err(|_| "cc runtime transfer activity map lock poisoned".to_string())?;
        if let Some(entry) = guard.get_mut(session_id) {
            *entry = Instant::now();
        }
        Ok(())
    }

    pub fn note_transfer_progress(
        &self,
        session_id: &str,
        percent: Option<u16>,
        bytes_transferred: Option<u64>,
    ) -> Result<(), String> {
        let mut guard = self
            .scp_session_progress_state
            .lock()
            .map_err(|_| "cc runtime transfer progress map lock poisoned".to_string())?;
        if let Some(entry) = guard.get_mut(session_id) {
            let percent_advanced = match (entry.last_percent, percent) {
                (None, Some(_)) => true,
                (Some(previous), Some(current)) if current > previous => true,
                _ => false,
            };
            let bytes_advanced = match (entry.last_bytes_transferred, bytes_transferred) {
                (None, Some(_)) => true,
                (Some(previous), Some(current)) if current > previous => true,
                _ => false,
            };
            if percent_advanced || bytes_advanced {
                entry.last_percent = percent;
                entry.last_bytes_transferred = bytes_transferred;
                entry.last_progress_at = Instant::now();
            }
        }
        Ok(())
    }

    pub fn transfer_last_activity_elapsed(&self, session_id: &str) -> Result<Option<std::time::Duration>, String> {
        let guard = self
            .scp_session_last_activity
            .lock()
            .map_err(|_| "cc runtime transfer activity map lock poisoned".to_string())?;
        Ok(guard.get(session_id).map(Instant::elapsed))
    }

    pub fn transfer_last_progress_elapsed(
        &self,
        session_id: &str,
    ) -> Result<Option<std::time::Duration>, String> {
        let guard = self
            .scp_session_progress_state
            .lock()
            .map_err(|_| "cc runtime transfer progress map lock poisoned".to_string())?;
        Ok(guard.get(session_id).map(|entry| entry.last_progress_at.elapsed()))
    }
}

impl Default for CcRuntimeState {
    fn default() -> Self {
        Self {
            connected: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            scp_session_context: Mutex::new(HashMap::new()),
            scp_session_last_activity: Mutex::new(HashMap::new()),
            scp_session_progress_state: Mutex::new(HashMap::new()),
        }
    }
}

pub fn next_transfer_resume_token(transfer_id: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("resume-{transfer_id}-{now}")
}
