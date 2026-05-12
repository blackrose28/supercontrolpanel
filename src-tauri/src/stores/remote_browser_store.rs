use std::{collections::HashMap, sync::Arc, sync::Mutex};

use ssh2::Session as SshSession;

pub struct RemoteBrowserSessionState {
    pub server_id: String,
    pub root_path: String,
    pub ssh: SshSession,
}

pub struct RemoteBrowserStore {
    pub sessions: Mutex<HashMap<String, Arc<RemoteBrowserSessionState>>>,
    pub next_id: Mutex<u64>,
}

impl Default for RemoteBrowserStore {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        }
    }
}
