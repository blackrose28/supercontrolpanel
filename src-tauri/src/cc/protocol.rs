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
    #[serde(rename = "commandId", skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
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
    Inventory,
    RecentLogs,
    RequestStatusSnapshot,
    RequestInventory,
    RequestRecentLogs,
    RequestRestartOpenvpn,
    RequestRestartBoundaryAuth,
    RequestRestartServerTunnel,
    RequestRestartServerSsh,
    OpenPty,
    WritePty,
    ResizePty,
    ClosePty,
    OpenRemoteBrowser,
    CloseRemoteBrowser,
    ListRemoteBrowserChildren,
    StatRemoteBrowserPath,
    CreateRemoteBrowserDirectory,
    DeleteRemoteBrowserPath,
    ScpUpload,
    ScpDownload,
    StartScpUpload,
    StartScpDownload,
    CommandStatus,
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
            "inventory" => MessageType::Inventory,
            "recent_logs" => MessageType::RecentLogs,
            "request_status_snapshot" => MessageType::RequestStatusSnapshot,
            "request_inventory" => MessageType::RequestInventory,
            "request_recent_logs" => MessageType::RequestRecentLogs,
            "request_restart_openvpn" => MessageType::RequestRestartOpenvpn,
            "request_restart_boundary_auth" => MessageType::RequestRestartBoundaryAuth,
            "request_restart_server_tunnel" => MessageType::RequestRestartServerTunnel,
            "request_restart_server_ssh" => MessageType::RequestRestartServerSsh,
            "open_pty" => MessageType::OpenPty,
            "write_pty" => MessageType::WritePty,
            "resize_pty" => MessageType::ResizePty,
            "close_pty" => MessageType::ClosePty,
            "open_remote_browser" => MessageType::OpenRemoteBrowser,
            "close_remote_browser" => MessageType::CloseRemoteBrowser,
            "list_remote_browser_children" => MessageType::ListRemoteBrowserChildren,
            "stat_remote_browser_path" => MessageType::StatRemoteBrowserPath,
            "create_remote_browser_directory" => MessageType::CreateRemoteBrowserDirectory,
            "delete_remote_browser_path" => MessageType::DeleteRemoteBrowserPath,
            "scp_upload" => MessageType::ScpUpload,
            "scp_download" => MessageType::ScpDownload,
            "start_scp_upload" => MessageType::StartScpUpload,
            "start_scp_download" => MessageType::StartScpDownload,
            "command_status" => MessageType::CommandStatus,
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
            command_id: None,
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

    pub fn inventory(device_id: String, seq: u64, inventory: Value) -> Self {
        Self::new("inventory", device_id, seq, Some(inventory))
    }

    pub fn recent_logs(device_id: String, seq: u64, logs: Value) -> Self {
        Self::new("recent_logs", device_id, seq, Some(logs))
    }

    pub fn command_status(
        device_id: String,
        seq: u64,
        command_id: String,
        status: Value,
    ) -> Self {
        let mut envelope = Self::new("command_status", device_id, seq, Some(status));
        envelope.command_id = Some(command_id);
        envelope
    }

    pub fn ack(device_id: String, seq: u64, req_seq: u64, result: Option<Value>) -> Self {
        let payload = result.unwrap_or_else(|| serde_json::json!({ "status": "ok" }));
        let mut envelope = Self::new("ack", device_id, seq, Some(payload));
        envelope.req_seq = Some(req_seq);
        envelope
    }

    pub fn nack(device_id: String, seq: u64, req_seq: u64, code: &str, message: &str) -> Self {
        let payload = serde_json::json!({
            "status": "error",
            "error": code,
            "message": message,
        });
        let mut envelope = Self::new("nack", device_id, seq, Some(payload));
        envelope.req_seq = Some(req_seq);
        envelope
    }
}
