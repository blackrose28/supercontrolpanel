use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::protocol::Envelope;

pub struct CcDispatcher {
    device_id: String,
    seq: Arc<Mutex<u64>>,
}

impl CcDispatcher {
    pub fn new(device_id: String) -> Self {
        Self {
            device_id,
            seq: Arc::new(Mutex::new(0)),
        }
    }

    pub async fn forward_function_status(&self, status: Value) -> Envelope {
        let seq = self.next_seq().await;
        Envelope::function_status(self.device_id.clone(), seq, status)
    }

    async fn next_seq(&self) -> u64 {
        let mut seq = self.seq.lock().await;
        *seq += 1;
        *seq
    }
}
