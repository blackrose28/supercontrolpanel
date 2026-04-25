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
    RequestStartServerSsh,
    RequestAttachServerSsh,
    RequestStartServerTunnel,
    RequestStopServerTunnel,
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
            "request_start_server_ssh" => MessageType::RequestStartServerSsh,
            "request_attach_server_ssh" => MessageType::RequestAttachServerSsh,
            "request_start_server_tunnel" => MessageType::RequestStartServerTunnel,
            "request_stop_server_tunnel" => MessageType::RequestStopServerTunnel,
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
            "os": std::env::consts::OS,
            "versions": {
                "app": env!("CARGO_PKG_VERSION"),
                "rust": "1.76.0",
                "tauri": "2.0"
            }
        });
        Self::new("heartbeat", device_id, seq, Some(payload))
    }

    pub fn status_snapshot(
        device_id: String,
        seq: u64,
        function_states: Vec<crate::models::FunctionStatus>,
    ) -> Self {
        let states_json: Vec<serde_json::Value> = function_states
            .iter()
            .map(|s| {
                serde_json::json!({
                    "functionId": s.function_id,
                    "state": s.state,
                    "message": s.message,
                    "sessionId": s.session_id,
                    "boundaryTokenSource": s.boundary_token_source,
                    "updatedAtMs": s.updated_at_ms,
                })
            })
            .collect();
        let payload = serde_json::json!({
            "activeSessions": [],
            "lastSeqByStream": {},
            "functionStates": states_json,
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

    pub fn command_status(device_id: String, seq: u64, command_id: String, status: Value) -> Self {
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

#[cfg(test)]
mod tests {
    use super::{Envelope, MessageType};

    #[test]
    fn status_snapshot_includes_function_states() {
        let states = vec![
            crate::models::FunctionStatus {
                function_id: "server-tunnel-srv1".to_string(),
                state: "running".to_string(),
                message: "Tunnel active".to_string(),
                boundary_token_source: None,
                session_id: Some("pty-abc123".to_string()),
                updated_at_ms: 1714000000000,
            },
            crate::models::FunctionStatus {
                function_id: "openvpn-connect".to_string(),
                state: "idle".to_string(),
                message: "Ready".to_string(),
                boundary_token_source: None,
                session_id: None,
                updated_at_ms: 1714000000001,
            },
        ];

        let snapshot = Envelope::status_snapshot("device-1".to_string(), 42, states);
        assert_eq!(snapshot.msg_type, "status_snapshot");
        assert_eq!(snapshot.device_id, "device-1");
        assert_eq!(snapshot.seq, 42);

        let payload = snapshot.payload.unwrap();
        let func_states = payload.get("functionStates").unwrap().as_array().unwrap();
        assert_eq!(func_states.len(), 2);

        let tunnel = &func_states[0];
        assert_eq!(tunnel.get("functionId").unwrap(), "server-tunnel-srv1");
        assert_eq!(tunnel.get("state").unwrap(), "running");
        assert_eq!(tunnel.get("sessionId").unwrap(), "pty-abc123");
        assert_eq!(tunnel.get("message").unwrap(), "Tunnel active");

        let openvpn = &func_states[1];
        assert_eq!(openvpn.get("functionId").unwrap(), "openvpn-connect");
        assert_eq!(openvpn.get("state").unwrap(), "idle");
        assert!(openvpn.get("sessionId").unwrap().is_null());
    }

    #[test]
    fn status_snapshot_includes_active_sessions_and_last_seq() {
        let states: Vec<crate::models::FunctionStatus> = vec![];
        let snapshot = Envelope::status_snapshot("device-1".to_string(), 1, states);
        let payload = snapshot.payload.unwrap();

        // Existing fields are still present
        assert!(payload.get("activeSessions").is_some());
        assert!(payload.get("lastSeqByStream").is_some());
        // New field is present even when empty
        assert!(payload.get("functionStates").is_some());
        assert_eq!(
            payload
                .get("functionStates")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
}
