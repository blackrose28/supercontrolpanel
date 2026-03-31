use std::{
    collections::HashMap,
    io::Write,
    sync::{mpsc::Sender, Arc, Mutex},
};

pub struct PtySession {
    pub master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    pub writer: Mutex<Box<dyn Write + Send>>,
    pub child: Mutex<Box<dyn portable_pty::Child + Send>>,
}

pub struct SessionStore {
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
