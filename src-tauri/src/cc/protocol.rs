use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub seq: u64,
    pub ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(rename = "reqSeq", skip_serializing_if = "Option::is_none")]
    pub req_seq: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum MessageType {
    Welcome,
    Ack,
    Nack,
    Heartbeat,
    StatusSnapshot,
    RequestStatusSnapshot,
    FunctionStatus,
    Unknown(String),
}

impl From<&str> for MessageType {
    fn from(s: &str) -> Self {
        match s {
            "welcome" => MessageType::Welcome,
            "ack" => MessageType::Ack,
            "nack" => MessageType::Nack,
            "heartbeat" => MessageType::Heartbeat,
            "status_snapshot" => MessageType::StatusSnapshot,
            "request_status_snapshot" => MessageType::RequestStatusSnapshot,
            "function-status" => MessageType::FunctionStatus,
            other => MessageType::Unknown(other.to_string()),
        }
    }
}

impl Envelope {
    pub fn new(msg_type: &str, device_id: String, seq: u64, payload: Option<Value>) -> Self {
        Self {
            msg_type: msg_type.to_string(),
            device_id,
            seq,
            ts: chrono::Utc::now().to_rfc3339(),
            payload,
            req_seq: None,
        }
    }

    pub fn heartbeat(device_id: String, seq: u64, uptime_sec: u64) -> Self {
        let payload = serde_json::json!({
            "uptimeSec": uptime_sec,
            "versions": {
                "app": env!("CARGO_PKG_VERSION"),
                "rust": "1.76.0",
                "tauri": "2.0"
            }
        });
        Self::new("heartbeat", device_id, seq, Some(payload))
    }

    pub fn status_snapshot(device_id: String, seq: u64) -> Self {
        let payload = serde_json::json!({
            "activeSessions": [],
            "lastSeqByStream": {}
        });
        Self::new("status_snapshot", device_id, seq, Some(payload))
    }

    pub fn function_status(device_id: String, seq: u64, status: Value) -> Self {
        Self::new("function-status", device_id, seq, Some(status))
    }
}
