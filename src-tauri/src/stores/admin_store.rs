use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

pub struct OpenVpnAdminSession {
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub id: String,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub stop_path: PathBuf,
}

pub struct BoundaryAutomationSession {
    pub session_id: String,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
pub struct OpenVpnReconnectState {
    pub consecutive_failures: u32,
    pub next_retry_at_ms: Option<u64>,
    pub retry_generation: u64,
    pub user_requested_stop: bool,
}

#[derive(Default)]
pub struct OpenVpnAdminStore {
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub session: Mutex<Option<OpenVpnAdminSession>>,
}

#[derive(Default)]
pub struct BoundaryAutomationStore {
    pub session: Mutex<Option<BoundaryAutomationSession>>,
}

#[derive(Default)]
pub struct OpenVpnReconnectStore {
    pub state: Mutex<OpenVpnReconnectState>,
}
