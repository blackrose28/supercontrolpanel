use std::{collections::HashMap, sync::Mutex};

use crate::models::ScpTransferEntry;

pub struct ScpTransferStore {
    pub entries: Mutex<Vec<ScpTransferEntry>>,
    pub session_to_transfer: Mutex<HashMap<String, String>>,
    pub session_output: Mutex<HashMap<String, String>>,
    pub next_id: Mutex<u64>,
}

impl Default for ScpTransferStore {
    fn default() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            session_to_transfer: Mutex::new(HashMap::new()),
            session_output: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        }
    }
}
