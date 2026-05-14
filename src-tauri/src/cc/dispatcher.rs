use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message;

use super::protocol::{Envelope, MessageType};
use super::telemetry::recent_log_events;
use crate::cc::pty_session_id::parse_cc_pty_session_id;
use crate::commands::{
    boundary::restart_boundary_auth_action,
    openvpn::restart_openvpn_action,
    pty::open_pty_internal,
    remote_browser::{
        close_remote_browser_action, create_remote_browser_directory_action,
        delete_remote_browser_path_action, list_remote_browser_children_action,
        stat_remote_browser_path_action,
    },
    server::{
        restart_server_ssh_action, restart_server_tunnel_action, start_server_ssh_action,
        start_server_tunnel_action, stop_server_tunnel_action,
    },
    update_functions_for_session_exit,
};
use crate::server_mgmt::{load_servers, ConnectionMethod, ServerOs};
use crate::services::{
    add_common_ssh_options, cc_scp_download_service, cc_scp_upload_service,
    cc_start_scp_download_service, cc_start_scp_upload_service, close_pty_internal, emit_pty_exit,
    keyring_get_optional, keyring_server_ssh_password_account, load_server_by_id, lock_err,
    open_pty_with_command_builder, open_remote_browser_with_id, open_remote_sftp_session,
    resolve_server_ssh_endpoint, ssh_target, write_pty_internal, StartScpResult,
};
use crate::stores::{
    next_transfer_resume_token, CcRuntimeState, FunctionStore, RemoteBrowserStore, RemotePtyStore,
    SessionStore, TransferRuntimeContext, TransferRuntimeDirection,
};
use portable_pty::PtySize;
use std::path::Path;
use std::process::Command;

const COMMAND_EXECUTION_TIMEOUT_SECS: u64 = 60;
const TRANSFER_HEARTBEAT_INTERVAL_SECS: u64 = 15;
const TRANSFER_TOTAL_TIMEOUT_SECS: u64 = 30 * 60;
const TRANSFER_STALL_TIMEOUT_SECS: u64 = 60;
const TRANSFER_PROGRESS_STALL_TIMEOUT_SECS: u64 = 90;
const TRANSFER_RETRY_BACKOFF_SECS: u64 = 2;
const TRANSFER_ATTEMPT: u64 = 1;
const TRANSFER_MAX_ATTEMPTS: u64 = 2;

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

#[derive(Debug, Clone)]
pub enum InboundServerMessage {
    RequestStatusSnapshot {
        command_id: Option<String>,
    },
    RequestInventory {
        command_id: Option<String>,
    },
    RequestRecentLogs {
        command_id: Option<String>,
    },
    RequestRestartOpenvpn {
        command_id: Option<String>,
    },
    RequestRestartBoundaryAuth {
        command_id: Option<String>,
    },
    RequestStartServerTunnel {
        command_id: Option<String>,
        server_id: String,
    },
    RequestStopServerTunnel {
        command_id: Option<String>,
        server_id: String,
    },
    RequestRestartServerTunnel {
        command_id: Option<String>,
        server_id: String,
    },
    RequestRestartServerSsh {
        command_id: Option<String>,
        server_id: String,
    },
    RequestStartServerSsh {
        command_id: Option<String>,
        req_seq: u64,
        server_id: String,
        session_id: String,
        cols: u16,
        rows: u16,
    },
    RequestAttachServerSsh {
        command_id: Option<String>,
        req_seq: u64,
        server_id: String,
        session_id: String,
        local_session_id: String,
        cols: u16,
        rows: u16,
    },
    RequestPtyScrollback {
        req_seq: u64,
        session_id: String,
    },
    OpenPty {
        command_id: Option<String>,
        req_seq: u64,
        session_id: String,
        cols: u16,
        rows: u16,
    },
    WritePty {
        command_id: Option<String>,
        req_seq: u64,
        session_id: String,
        data: String,
    },
    ResizePty {
        command_id: Option<String>,
        req_seq: u64,
        session_id: String,
        cols: u16,
        rows: u16,
    },
    ClosePty {
        command_id: Option<String>,
        req_seq: u64,
        session_id: String,
    },
    OpenRemoteBrowser {
        command_id: Option<String>,
        req_seq: u64,
        server_id: String,
        browser_session_id: String,
    },
    CloseRemoteBrowser {
        command_id: Option<String>,
        req_seq: u64,
        browser_session_id: String,
    },
    ListRemoteBrowserChildren {
        command_id: Option<String>,
        req_seq: u64,
        browser_session_id: String,
        path: String,
    },
    StatRemoteBrowserPath {
        command_id: Option<String>,
        req_seq: u64,
        browser_session_id: String,
        path: String,
    },
    CreateRemoteBrowserDirectory {
        command_id: Option<String>,
        req_seq: u64,
        browser_session_id: String,
        path: String,
    },
    DeleteRemoteBrowserPath {
        command_id: Option<String>,
        req_seq: u64,
        browser_session_id: String,
        path: String,
        recursive: bool,
    },
    ScpUpload {
        command_id: Option<String>,
        req_seq: u64,
        transfer_id: String,
        server_id: String,
        local_path: String,
        remote_path: String,
        recursive: bool,
        overwrite_policy: Option<String>,
        follow_symlinks: bool,
        verify_checksum: bool,
    },
    ScpDownload {
        command_id: Option<String>,
        req_seq: u64,
        transfer_id: String,
        server_id: String,
        remote_path: String,
        local_path: String,
        recursive: bool,
        overwrite_policy: Option<String>,
        follow_symlinks: bool,
        verify_checksum: bool,
    },
    StartScpUpload {
        command_id: Option<String>,
        req_seq: u64,
        transfer_id: String,
        server_id: String,
        local_path: String,
        remote_path: String,
        recursive: bool,
        overwrite_policy: Option<String>,
        follow_symlinks: bool,
        verify_checksum: bool,
        resume_token: Option<String>,
        resume_offset_bytes: Option<u64>,
        temp_path_hint: Option<String>,
    },
    StartScpDownload {
        command_id: Option<String>,
        req_seq: u64,
        transfer_id: String,
        server_id: String,
        remote_path: String,
        local_path: String,
        recursive: bool,
        overwrite_policy: Option<String>,
        follow_symlinks: bool,
        verify_checksum: bool,
        resume_token: Option<String>,
        resume_offset_bytes: Option<u64>,
        temp_path_hint: Option<String>,
    },
}

impl InboundServerMessage {
    pub fn command_id(&self) -> Option<&str> {
        match self {
            Self::RequestPtyScrollback { .. } => None,
            Self::RequestStatusSnapshot { command_id }
            | Self::RequestInventory { command_id }
            | Self::RequestRecentLogs { command_id }
            | Self::RequestRestartOpenvpn { command_id }
            | Self::RequestRestartBoundaryAuth { command_id }
            | Self::RequestStartServerTunnel { command_id, .. }
            | Self::RequestStopServerTunnel { command_id, .. }
            | Self::RequestRestartServerTunnel { command_id, .. }
            | Self::RequestRestartServerSsh { command_id, .. }
            | Self::RequestStartServerSsh { command_id, .. }
            | Self::RequestAttachServerSsh { command_id, .. }
            | Self::OpenPty { command_id, .. }
            | Self::WritePty { command_id, .. }
            | Self::ResizePty { command_id, .. }
            | Self::ClosePty { command_id, .. }
            | Self::OpenRemoteBrowser { command_id, .. }
            | Self::CloseRemoteBrowser { command_id, .. }
            | Self::ListRemoteBrowserChildren { command_id, .. }
            | Self::StatRemoteBrowserPath { command_id, .. }
            | Self::CreateRemoteBrowserDirectory { command_id, .. }
            | Self::DeleteRemoteBrowserPath { command_id, .. }
            | Self::ScpUpload { command_id, .. }
            | Self::ScpDownload { command_id, .. }
            | Self::StartScpUpload { command_id, .. }
            | Self::StartScpDownload { command_id, .. } => command_id.as_deref(),
        }
    }

    fn action_name(&self) -> &'static str {
        match self {
            Self::RequestStatusSnapshot { .. } => "request_status_snapshot",
            Self::RequestInventory { .. } => "request_inventory",
            Self::RequestRecentLogs { .. } => "request_recent_logs",
            Self::RequestRestartOpenvpn { .. } => "request_restart_openvpn",
            Self::RequestRestartBoundaryAuth { .. } => "request_restart_boundary_auth",
            Self::RequestStartServerTunnel { .. } => "request_start_server_tunnel",
            Self::RequestStopServerTunnel { .. } => "request_stop_server_tunnel",
            Self::RequestRestartServerTunnel { .. } => "request_restart_server_tunnel",
            Self::RequestRestartServerSsh { .. } => "request_restart_server_ssh",
            Self::RequestStartServerSsh { .. } => "request_start_server_ssh",
            Self::RequestAttachServerSsh { .. } => "request_attach_server_ssh",
            Self::RequestPtyScrollback { .. } => "request_pty_scrollback",
            Self::OpenPty { .. } => "open_pty",
            Self::WritePty { .. } => "write_pty",
            Self::ResizePty { .. } => "resize_pty",
            Self::ClosePty { .. } => "close_pty",
            Self::OpenRemoteBrowser { .. } => "open_remote_browser",
            Self::CloseRemoteBrowser { .. } => "close_remote_browser",
            Self::ListRemoteBrowserChildren { .. } => "list_remote_browser_children",
            Self::StatRemoteBrowserPath { .. } => "stat_remote_browser_path",
            Self::CreateRemoteBrowserDirectory { .. } => "create_remote_browser_directory",
            Self::DeleteRemoteBrowserPath { .. } => "delete_remote_browser_path",
            Self::ScpUpload { .. } => "scp_upload",
            Self::ScpDownload { .. } => "scp_download",
            Self::StartScpUpload { .. } => "start_scp_upload",
            Self::StartScpDownload { .. } => "start_scp_download",
        }
    }

    fn target_id(&self) -> Option<&str> {
        match self {
            Self::RequestStartServerTunnel { server_id, .. }
            | Self::RequestStopServerTunnel { server_id, .. }
            | Self::RequestRestartServerTunnel { server_id, .. }
            | Self::RequestRestartServerSsh { server_id, .. }
            | Self::RequestStartServerSsh { server_id, .. }
            | Self::RequestAttachServerSsh { server_id, .. }
            | Self::OpenRemoteBrowser { server_id, .. }
            | Self::ScpUpload { server_id, .. }
            | Self::ScpDownload { server_id, .. }
            | Self::StartScpUpload { server_id, .. }
            | Self::StartScpDownload { server_id, .. } => Some(server_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InboundServerMessageError {
    pub command_id: Option<String>,
    pub reason_code: String,
    pub reason_detail: String,
}

#[derive(Debug, Deserialize)]
struct RestartServerPayload {
    #[serde(rename = "serverId")]
    server_id: String,
}

#[derive(Debug, Deserialize)]
struct StartServerSshPayload {
    #[serde(rename = "serverId")]
    server_id: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
struct AttachServerSshPayload {
    #[serde(rename = "serverId")]
    server_id: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "localSessionId")]
    local_session_id: String,
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
struct OpenPtyPayload {
    #[serde(rename = "sessionId")]
    session_id: String,
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
struct WritePtyPayload {
    #[serde(rename = "sessionId")]
    session_id: String,
    data: String,
}

#[derive(Debug, Deserialize)]
struct ResizePtyPayload {
    #[serde(rename = "sessionId")]
    session_id: String,
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
struct ClosePtyPayload {
    #[serde(rename = "sessionId")]
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct OpenRemoteBrowserPayload {
    #[serde(rename = "serverId")]
    server_id: String,
    #[serde(rename = "browserSessionId")]
    browser_session_id: String,
}

#[derive(Debug, Deserialize)]
struct BrowserSessionPayload {
    #[serde(rename = "browserSessionId")]
    browser_session_id: String,
}

#[derive(Debug, Deserialize)]
struct BrowserPathPayload {
    #[serde(rename = "browserSessionId")]
    browser_session_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct DeleteRemoteBrowserPathPayload {
    #[serde(rename = "browserSessionId")]
    browser_session_id: String,
    path: String,
    recursive: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ScpUploadPayload {
    #[serde(rename = "transferId")]
    transfer_id: String,
    #[serde(rename = "serverId")]
    server_id: String,
    #[serde(rename = "localPath")]
    local_path: String,
    #[serde(rename = "remotePath")]
    remote_path: String,
    recursive: Option<bool>,
    #[serde(rename = "overwritePolicy")]
    overwrite_policy: Option<String>,
    #[serde(rename = "followSymlinks")]
    follow_symlinks: Option<bool>,
    #[serde(rename = "verifyChecksum")]
    verify_checksum: Option<bool>,
    #[serde(rename = "resumeToken")]
    resume_token: Option<String>,
    #[serde(rename = "resumeOffsetBytes")]
    resume_offset_bytes: Option<u64>,
    #[serde(rename = "tempPathHint")]
    temp_path_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScpDownloadPayload {
    #[serde(rename = "transferId")]
    transfer_id: String,
    #[serde(rename = "serverId")]
    server_id: String,
    #[serde(rename = "remotePath")]
    remote_path: String,
    #[serde(rename = "localPath")]
    local_path: String,
    recursive: Option<bool>,
    #[serde(rename = "overwritePolicy")]
    overwrite_policy: Option<String>,
    #[serde(rename = "followSymlinks")]
    follow_symlinks: Option<bool>,
    #[serde(rename = "verifyChecksum")]
    verify_checksum: Option<bool>,
    #[serde(rename = "resumeToken")]
    resume_token: Option<String>,
    #[serde(rename = "resumeOffsetBytes")]
    resume_offset_bytes: Option<u64>,
    #[serde(rename = "tempPathHint")]
    temp_path_hint: Option<String>,
}

pub fn parse_inbound_server_message(
    envelope: Envelope,
) -> Result<InboundServerMessage, InboundServerMessageError> {
    let command_id = envelope.command_id.clone();

    match MessageType::from(envelope.msg_type.as_str()) {
        MessageType::RequestStatusSnapshot => {
            Ok(InboundServerMessage::RequestStatusSnapshot { command_id })
        }
        MessageType::RequestInventory => Ok(InboundServerMessage::RequestInventory { command_id }),
        MessageType::RequestRecentLogs => {
            Ok(InboundServerMessage::RequestRecentLogs { command_id })
        }
        MessageType::RequestRestartOpenvpn => {
            Ok(InboundServerMessage::RequestRestartOpenvpn { command_id })
        }
        MessageType::RequestRestartBoundaryAuth => {
            Ok(InboundServerMessage::RequestRestartBoundaryAuth { command_id })
        }
        MessageType::RequestStartServerTunnel => {
            let payload = parse_restart_server_payload(envelope)?;
            Ok(InboundServerMessage::RequestStartServerTunnel {
                command_id,
                server_id: payload.server_id,
            })
        }
        MessageType::RequestStopServerTunnel => {
            let payload = parse_restart_server_payload(envelope)?;
            Ok(InboundServerMessage::RequestStopServerTunnel {
                command_id,
                server_id: payload.server_id,
            })
        }
        MessageType::RequestRestartServerTunnel => {
            let payload = parse_restart_server_payload(envelope)?;
            Ok(InboundServerMessage::RequestRestartServerTunnel {
                command_id,
                server_id: payload.server_id,
            })
        }
        MessageType::RequestRestartServerSsh => {
            let payload = parse_restart_server_payload(envelope)?;
            Ok(InboundServerMessage::RequestRestartServerSsh {
                command_id,
                server_id: payload.server_id,
            })
        }
        MessageType::RequestStartServerSsh => {
            let req_seq = envelope.seq;
            let payload = parse_start_server_ssh_payload(envelope)?;
            Ok(InboundServerMessage::RequestStartServerSsh {
                command_id,
                req_seq,
                server_id: payload.server_id,
                session_id: payload.session_id,
                cols: payload.cols,
                rows: payload.rows,
            })
        }
        MessageType::RequestAttachServerSsh => {
            let req_seq = envelope.seq;
            let payload = parse_attach_server_ssh_payload(envelope)?;
            Ok(InboundServerMessage::RequestAttachServerSsh {
                command_id,
                req_seq,
                server_id: payload.server_id,
                session_id: payload.session_id,
                local_session_id: payload.local_session_id,
                cols: payload.cols,
                rows: payload.rows,
            })
        }
        MessageType::RequestPtyScrollback => {
            let req_seq = envelope.seq;
            let payload = parse_pty_scrollback_payload(envelope)?;
            Ok(InboundServerMessage::RequestPtyScrollback {
                req_seq,
                session_id: payload.session_id,
            })
        }
        MessageType::OpenPty => {
            let req_seq = envelope.seq;
            let payload = parse_open_pty_payload(envelope)?;
            Ok(InboundServerMessage::OpenPty {
                command_id,
                req_seq,
                session_id: payload.session_id,
                cols: payload.cols,
                rows: payload.rows,
            })
        }
        MessageType::WritePty => {
            let req_seq = envelope.seq;
            let payload = parse_write_pty_payload(envelope)?;
            Ok(InboundServerMessage::WritePty {
                command_id,
                req_seq,
                session_id: payload.session_id,
                data: payload.data,
            })
        }
        MessageType::ResizePty => {
            let req_seq = envelope.seq;
            let payload = parse_resize_pty_payload(envelope)?;
            Ok(InboundServerMessage::ResizePty {
                command_id,
                req_seq,
                session_id: payload.session_id,
                cols: payload.cols,
                rows: payload.rows,
            })
        }
        MessageType::ClosePty => {
            let req_seq = envelope.seq;
            let payload = parse_close_pty_payload(envelope)?;
            Ok(InboundServerMessage::ClosePty {
                command_id,
                req_seq,
                session_id: payload.session_id,
            })
        }
        MessageType::OpenRemoteBrowser => {
            let req_seq = envelope.seq;
            let payload = parse_open_remote_browser_payload(envelope)?;
            Ok(InboundServerMessage::OpenRemoteBrowser {
                command_id,
                req_seq,
                server_id: payload.server_id,
                browser_session_id: payload.browser_session_id,
            })
        }
        MessageType::CloseRemoteBrowser => {
            let req_seq = envelope.seq;
            let payload = parse_browser_session_payload(envelope, "close_remote_browser")?;
            Ok(InboundServerMessage::CloseRemoteBrowser {
                command_id,
                req_seq,
                browser_session_id: payload.browser_session_id,
            })
        }
        MessageType::ListRemoteBrowserChildren => {
            let req_seq = envelope.seq;
            let payload = parse_browser_path_payload(envelope, "list_remote_browser_children")?;
            Ok(InboundServerMessage::ListRemoteBrowserChildren {
                command_id,
                req_seq,
                browser_session_id: payload.browser_session_id,
                path: payload.path,
            })
        }
        MessageType::StatRemoteBrowserPath => {
            let req_seq = envelope.seq;
            let payload = parse_browser_path_payload(envelope, "stat_remote_browser_path")?;
            Ok(InboundServerMessage::StatRemoteBrowserPath {
                command_id,
                req_seq,
                browser_session_id: payload.browser_session_id,
                path: payload.path,
            })
        }
        MessageType::CreateRemoteBrowserDirectory => {
            let req_seq = envelope.seq;
            let payload = parse_browser_path_payload(envelope, "create_remote_browser_directory")?;
            Ok(InboundServerMessage::CreateRemoteBrowserDirectory {
                command_id,
                req_seq,
                browser_session_id: payload.browser_session_id,
                path: payload.path,
            })
        }
        MessageType::DeleteRemoteBrowserPath => {
            let req_seq = envelope.seq;
            let payload = parse_delete_remote_browser_path_payload(envelope)?;
            Ok(InboundServerMessage::DeleteRemoteBrowserPath {
                command_id,
                req_seq,
                browser_session_id: payload.browser_session_id,
                path: payload.path,
                recursive: payload.recursive.unwrap_or(false),
            })
        }
        MessageType::ScpUpload => {
            let req_seq = envelope.seq;
            let payload = parse_scp_upload_payload(envelope, "scp_upload")?;
            Ok(InboundServerMessage::ScpUpload {
                command_id,
                req_seq,
                transfer_id: payload.transfer_id,
                server_id: payload.server_id,
                local_path: payload.local_path,
                remote_path: payload.remote_path,
                recursive: payload.recursive.unwrap_or(false),
                overwrite_policy: payload.overwrite_policy,
                follow_symlinks: payload.follow_symlinks.unwrap_or(false),
                verify_checksum: payload.verify_checksum.unwrap_or(false),
            })
        }
        MessageType::ScpDownload => {
            let req_seq = envelope.seq;
            let payload = parse_scp_download_payload(envelope, "scp_download")?;
            Ok(InboundServerMessage::ScpDownload {
                command_id,
                req_seq,
                transfer_id: payload.transfer_id,
                server_id: payload.server_id,
                remote_path: payload.remote_path,
                local_path: payload.local_path,
                recursive: payload.recursive.unwrap_or(false),
                overwrite_policy: payload.overwrite_policy,
                follow_symlinks: payload.follow_symlinks.unwrap_or(true),
                verify_checksum: payload.verify_checksum.unwrap_or(false),
            })
        }
        MessageType::StartScpUpload => {
            let req_seq = envelope.seq;
            let payload = parse_scp_upload_payload(envelope, "start_scp_upload")?;
            Ok(InboundServerMessage::StartScpUpload {
                command_id,
                req_seq,
                transfer_id: payload.transfer_id,
                server_id: payload.server_id,
                local_path: payload.local_path,
                remote_path: payload.remote_path,
                recursive: payload.recursive.unwrap_or(false),
                overwrite_policy: payload.overwrite_policy,
                follow_symlinks: payload.follow_symlinks.unwrap_or(false),
                verify_checksum: payload.verify_checksum.unwrap_or(false),
                resume_token: payload.resume_token,
                resume_offset_bytes: payload.resume_offset_bytes,
                temp_path_hint: payload.temp_path_hint,
            })
        }
        MessageType::StartScpDownload => {
            let req_seq = envelope.seq;
            let payload = parse_scp_download_payload(envelope, "start_scp_download")?;
            Ok(InboundServerMessage::StartScpDownload {
                command_id,
                req_seq,
                transfer_id: payload.transfer_id,
                server_id: payload.server_id,
                remote_path: payload.remote_path,
                local_path: payload.local_path,
                recursive: payload.recursive.unwrap_or(false),
                overwrite_policy: payload.overwrite_policy,
                follow_symlinks: payload.follow_symlinks.unwrap_or(true),
                verify_checksum: payload.verify_checksum.unwrap_or(false),
                resume_token: payload.resume_token,
                resume_offset_bytes: payload.resume_offset_bytes,
                temp_path_hint: payload.temp_path_hint,
            })
        }
        other => Err(InboundServerMessageError {
            command_id,
            reason_code: "UNSUPPORTED_MESSAGE_TYPE".to_string(),
            reason_detail: format!("Unsupported inbound CC message type: {:?}", other),
        }),
    }
}

fn parse_open_pty_payload(envelope: Envelope) -> Result<OpenPtyPayload, InboundServerMessageError> {
    parse_payload(envelope, "open_pty")
}

fn parse_start_server_ssh_payload(
    envelope: Envelope,
) -> Result<StartServerSshPayload, InboundServerMessageError> {
    parse_payload(envelope, "request_start_server_ssh")
}

fn parse_attach_server_ssh_payload(
    envelope: Envelope,
) -> Result<AttachServerSshPayload, InboundServerMessageError> {
    parse_payload(envelope, "request_attach_server_ssh")
}

#[derive(Debug, Deserialize)]
struct PtyScrollbackPayload {
    #[serde(rename = "sessionId")]
    session_id: String,
}

fn parse_pty_scrollback_payload(
    envelope: Envelope,
) -> Result<PtyScrollbackPayload, InboundServerMessageError> {
    parse_payload(envelope, "request_pty_scrollback")
}

fn parse_write_pty_payload(
    envelope: Envelope,
) -> Result<WritePtyPayload, InboundServerMessageError> {
    parse_payload(envelope, "write_pty")
}

fn parse_resize_pty_payload(
    envelope: Envelope,
) -> Result<ResizePtyPayload, InboundServerMessageError> {
    parse_payload(envelope, "resize_pty")
}

fn parse_close_pty_payload(
    envelope: Envelope,
) -> Result<ClosePtyPayload, InboundServerMessageError> {
    parse_payload(envelope, "close_pty")
}

fn parse_open_remote_browser_payload(
    envelope: Envelope,
) -> Result<OpenRemoteBrowserPayload, InboundServerMessageError> {
    parse_payload(envelope, "open_remote_browser")
}

fn parse_browser_session_payload(
    envelope: Envelope,
    message_type: &str,
) -> Result<BrowserSessionPayload, InboundServerMessageError> {
    parse_payload(envelope, message_type)
}

fn parse_browser_path_payload(
    envelope: Envelope,
    message_type: &str,
) -> Result<BrowserPathPayload, InboundServerMessageError> {
    parse_payload(envelope, message_type)
}

fn parse_delete_remote_browser_path_payload(
    envelope: Envelope,
) -> Result<DeleteRemoteBrowserPathPayload, InboundServerMessageError> {
    parse_payload(envelope, "delete_remote_browser_path")
}

fn parse_scp_upload_payload(
    envelope: Envelope,
    message_type: &str,
) -> Result<ScpUploadPayload, InboundServerMessageError> {
    parse_payload(envelope, message_type)
}

fn parse_scp_download_payload(
    envelope: Envelope,
    message_type: &str,
) -> Result<ScpDownloadPayload, InboundServerMessageError> {
    parse_payload(envelope, message_type)
}

fn parse_payload<T: for<'de> Deserialize<'de>>(
    envelope: Envelope,
    message_type: &str,
) -> Result<T, InboundServerMessageError> {
    let command_id = envelope.command_id.clone();
    let payload = envelope.payload.ok_or_else(|| InboundServerMessageError {
        command_id: command_id.clone(),
        reason_code: "INVALID_PAYLOAD".to_string(),
        reason_detail: format!("Missing payload for {message_type}"),
    })?;

    serde_json::from_value(payload).map_err(|err| InboundServerMessageError {
        command_id,
        reason_code: "INVALID_PAYLOAD".to_string(),
        reason_detail: format!("Invalid payload for {message_type}: {err}"),
    })
}

fn parse_restart_server_payload(
    envelope: Envelope,
) -> Result<RestartServerPayload, InboundServerMessageError> {
    let command_id = envelope.command_id.clone();
    let payload = envelope.payload.ok_or_else(|| InboundServerMessageError {
        command_id: command_id.clone(),
        reason_code: "SERVER_ID_REQUIRED".to_string(),
        reason_detail: format!("Missing payload.serverId for {}", envelope.msg_type),
    })?;

    let parsed: RestartServerPayload =
        serde_json::from_value(payload).map_err(|_| InboundServerMessageError {
            command_id,
            reason_code: "SERVER_ID_REQUIRED".to_string(),
            reason_detail: format!("Missing payload.serverId for {}", envelope.msg_type),
        })?;

    let server_id = parsed.server_id.trim();
    if server_id.is_empty() {
        return Err(InboundServerMessageError {
            command_id: envelope.command_id,
            reason_code: "SERVER_ID_REQUIRED".to_string(),
            reason_detail: format!("Missing payload.serverId for {}", envelope.msg_type),
        });
    }

    Ok(RestartServerPayload {
        server_id: server_id.to_string(),
    })
}

pub async fn dispatch_inbound_server_message<W>(
    message: InboundServerMessage,
    write: Arc<Mutex<W>>,
    device_id: String,
    seq_counter: Arc<Mutex<u64>>,
    app: AppHandle,
) -> Result<(), String>
where
    W: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin
        + Send
        + 'static,
{
    let inbound_action = message.action_name();
    let inbound_command_id = message.command_id();
    let inbound_target = message.target_id();
    println!(
        "{}",
        command_lifecycle_log_line(
            "dispatch.received",
            inbound_command_id,
            "received",
            Some(inbound_action),
            None,
            inbound_target,
        )
    );

    match message {
        InboundServerMessage::RequestStatusSnapshot { command_id } => {
            if let Some(command_id) = command_id {
                let mut w = write.lock().await;
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "acknowledged",
                    None,
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
                send_status_snapshot(&mut *w, &device_id, &seq_counter, &app)
                    .await
                    .map_err(|err| err.to_string())?;
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "succeeded",
                    Some("status_snapshot".to_string()),
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
            } else {
                let mut w = write.lock().await;
                send_status_snapshot(&mut *w, &device_id, &seq_counter, &app)
                    .await
                    .map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        InboundServerMessage::RequestInventory { command_id } => {
            if let Some(command_id) = command_id {
                let mut w = write.lock().await;
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "acknowledged",
                    None,
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
                send_inventory(&mut *w, &device_id, &seq_counter, &app)
                    .await
                    .map_err(|err| err.to_string())?;
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "succeeded",
                    Some("inventory".to_string()),
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
            } else {
                let mut w = write.lock().await;
                send_inventory(&mut *w, &device_id, &seq_counter, &app)
                    .await
                    .map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        InboundServerMessage::RequestRecentLogs { command_id } => {
            if let Some(command_id) = command_id {
                let mut w = write.lock().await;
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "acknowledged",
                    None,
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
                send_recent_logs(&mut *w, &device_id, &seq_counter)
                    .await
                    .map_err(|err| err.to_string())?;
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "succeeded",
                    Some("recent_logs".to_string()),
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
            } else {
                let mut w = write.lock().await;
                send_recent_logs(&mut *w, &device_id, &seq_counter)
                    .await
                    .map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        InboundServerMessage::RequestRestartOpenvpn { command_id } => {
            spawn_restart_task(
                write,
                device_id,
                seq_counter,
                command_id,
                "restart_openvpn",
                None,
                move || restart_openvpn_action(app.clone()),
            );
            Ok(())
        }
        InboundServerMessage::RequestRestartBoundaryAuth { command_id } => {
            spawn_restart_task(
                write,
                device_id,
                seq_counter,
                command_id,
                "restart_boundary_auth",
                None,
                move || restart_boundary_auth_action(app.clone()),
            );
            Ok(())
        }
        InboundServerMessage::RequestStartServerTunnel {
            command_id,
            server_id,
        } => {
            let action_server_id = server_id.clone();
            spawn_restart_task(
                write,
                device_id,
                seq_counter,
                command_id,
                "start_server_tunnel",
                Some(server_id),
                move || {
                    let cfg = crate::commands::load_app_config_from_disk(&app)?;
                    let session_store = app.state::<SessionStore>();
                    let function_store = app.state::<FunctionStore>();
                    let browser_store = app.state::<RemoteBrowserStore>();
                    start_server_tunnel_action(
                        action_server_id.clone(),
                        cfg.app.default_terminal_cols,
                        cfg.app.default_terminal_rows,
                        app.clone(),
                        session_store,
                        function_store,
                        browser_store,
                    )
                    .map(|_| ())
                },
            );
            Ok(())
        }
        InboundServerMessage::RequestStopServerTunnel {
            command_id,
            server_id,
        } => {
            let action_server_id = server_id.clone();
            spawn_restart_task(
                write,
                device_id,
                seq_counter,
                command_id,
                "stop_server_tunnel",
                Some(server_id),
                move || {
                    let session_store = app.state::<SessionStore>();
                    let function_store = app.state::<FunctionStore>();
                    let browser_store = app.state::<RemoteBrowserStore>();
                    stop_server_tunnel_action(
                        action_server_id.clone(),
                        app.clone(),
                        session_store,
                        function_store,
                        browser_store,
                    )
                },
            );
            Ok(())
        }
        InboundServerMessage::RequestRestartServerTunnel {
            command_id,
            server_id,
        } => {
            let action_server_id = server_id.clone();
            spawn_restart_task(
                write,
                device_id,
                seq_counter,
                command_id,
                "restart_server_tunnel",
                Some(server_id),
                move || restart_server_tunnel_action(action_server_id.clone(), app.clone()),
            );
            Ok(())
        }
        InboundServerMessage::RequestRestartServerSsh {
            command_id,
            server_id,
        } => {
            let action_server_id = server_id.clone();
            spawn_restart_task(
                write,
                device_id,
                seq_counter,
                command_id,
                "restart_server_ssh",
                Some(server_id),
                move || restart_server_ssh_action(action_server_id.clone(), app.clone()),
            );
            Ok(())
        }
        InboundServerMessage::RequestStartServerSsh {
            command_id,
            req_seq,
            server_id,
            session_id,
            cols,
            rows,
        } => {
            let mut w = write.lock().await;
            let forced_local_session_id = match forced_local_session_for_admin_ssh_start(
                &device_id,
                &session_id,
            ) {
                Ok(v) => v,
                Err(detail) => {
                    send_nack(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        req_seq,
                        "PTY_INVALID_SESSION",
                        &detail,
                    )
                    .await
                    .map_err(|send_err| send_err.to_string())?;
                    return Ok(());
                }
            };
            let session_store = app.state::<SessionStore>();
            let function_store = app.state::<FunctionStore>();
            let result = match start_server_ssh_action(
                server_id.clone(),
                cols,
                rows,
                forced_local_session_id,
                app.clone(),
                session_store,
                function_store,
            ) {
                Ok(result) => result,
                Err(err) => {
                    send_nack(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        req_seq,
                        "PTY_SPAWN_ERROR",
                        &err,
                    )
                    .await
                    .map_err(|send_err| send_err.to_string())?;
                    return Ok(());
                }
            };
            bind_remote_pty_session(&app, &session_id, &result.session_id)?;
            send_ack(
                &mut *w,
                &device_id,
                &seq_counter,
                req_seq,
                Some(json!({
                    "status": "ok",
                    "serverId": server_id,
                    "sessionId": session_id,
                    "localSessionId": result.session_id,
                    "functionId": result.function_id,
                })),
            )
            .await
            .map_err(|err| err.to_string())?;
            if let Some(command_id) = command_id {
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "succeeded",
                    Some("request_start_server_ssh".to_string()),
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        InboundServerMessage::RequestAttachServerSsh {
            command_id,
            req_seq,
            server_id,
            session_id,
            local_session_id,
            cols,
            rows,
        } => {
            let mut w = write.lock().await;
            let session_store = app.state::<SessionStore>();
            if let Err(err) = resize_local_pty(&session_store, &local_session_id, cols, rows) {
                let code = pty_error_code(&err, "PTY_NOT_FOUND");
                send_nack(&mut *w, &device_id, &seq_counter, req_seq, &code, &err)
                    .await
                    .map_err(|send_err| send_err.to_string())?;
                return Ok(());
            }
            if let Err(err) = bind_remote_pty_session(&app, &session_id, &local_session_id) {
                send_nack(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    req_seq,
                    "PTY_BIND_ERROR",
                    &err,
                )
                .await
                .map_err(|send_err| send_err.to_string())?;
                return Ok(());
            }
            send_ack(
                &mut *w,
                &device_id,
                &seq_counter,
                req_seq,
                Some(json!({
                    "status": "ok",
                    "serverId": server_id,
                    "sessionId": session_id,
                    "localSessionId": local_session_id,
                })),
            )
            .await
            .map_err(|err| err.to_string())?;
            if let Some(command_id) = command_id {
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "succeeded",
                    Some("request_attach_server_ssh".to_string()),
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        InboundServerMessage::RequestPtyScrollback {
            req_seq,
            session_id,
        } => {
            let snapshot = app
                .state::<RemotePtyStore>()
                .scrollback_for_remote_session(&session_id)
                .map_err(|e| e.to_string())?;
            let (data, truncated) = snapshot.unwrap_or((String::new(), false));
            let mut w = write.lock().await;
            send_ws_event(
                &mut *w,
                "pty-scrollback",
                &device_id,
                &seq_counter,
                json!({
                    "sessionId": session_id,
                    "data": data,
                    "truncated": truncated,
                }),
            )
            .await
            .map_err(|e| e.to_string())?;
            send_ack(
                &mut *w,
                &device_id,
                &seq_counter,
                req_seq,
                Some(json!({ "status": "ok" })),
            )
            .await
            .map_err(|e| e.to_string())?;
            Ok(())
        }
        InboundServerMessage::OpenPty {
            command_id,
            req_seq,
            session_id,
            cols,
            rows,
        } => {
            let mut w = write.lock().await;
            let session_store = app.state::<SessionStore>();
            let local_session_id = match open_pty_internal(cols, rows, &app, &session_store) {
                Ok(local_session_id) => local_session_id,
                Err(err) => {
                    send_nack(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        req_seq,
                        "PTY_SPAWN_ERROR",
                        &err,
                    )
                    .await
                    .map_err(|send_err| send_err.to_string())?;
                    return Ok(());
                }
            };
            bind_remote_pty_session(&app, &session_id, &local_session_id)?;
            send_ack(
                &mut *w,
                &device_id,
                &seq_counter,
                req_seq,
                Some(json!({ "status": "ok", "sessionId": session_id })),
            )
            .await
            .map_err(|err| err.to_string())?;
            if let Some(command_id) = command_id {
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "succeeded",
                    Some("open_pty".to_string()),
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        InboundServerMessage::WritePty {
            command_id,
            req_seq,
            session_id,
            data,
        } => {
            let mut w = write.lock().await;
            let local_session_id = match resolve_remote_pty_session(&app, &session_id) {
                Ok(local_session_id) => local_session_id,
                Err(err) => {
                    send_nack(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        req_seq,
                        "PTY_NOT_FOUND",
                        &err,
                    )
                    .await
                    .map_err(|send_err| send_err.to_string())?;
                    return Ok(());
                }
            };
            let session_store = app.state::<SessionStore>();
            if let Err(err) = write_pty_internal(&local_session_id, &data, &session_store) {
                send_nack(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    req_seq,
                    "PTY_WRITE_REJECTED",
                    &err,
                )
                .await
                .map_err(|send_err| send_err.to_string())?;
                return Ok(());
            }
            send_ack(
                &mut *w,
                &device_id,
                &seq_counter,
                req_seq,
                Some(
                    json!({ "status": "ok", "sessionId": session_id, "acceptedBytes": data.len() }),
                ),
            )
            .await
            .map_err(|err| err.to_string())?;
            if let Some(command_id) = command_id {
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "succeeded",
                    Some("write_pty".to_string()),
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        InboundServerMessage::ResizePty {
            command_id,
            req_seq,
            session_id,
            cols,
            rows,
        } => {
            let mut w = write.lock().await;
            let local_session_id = match resolve_remote_pty_session(&app, &session_id) {
                Ok(local_session_id) => local_session_id,
                Err(err) => {
                    send_nack(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        req_seq,
                        "PTY_NOT_FOUND",
                        &err,
                    )
                    .await
                    .map_err(|send_err| send_err.to_string())?;
                    return Ok(());
                }
            };
            let session_store = app.state::<SessionStore>();
            if let Err(err) = resize_local_pty(&session_store, &local_session_id, cols, rows) {
                send_nack(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    req_seq,
                    "PTY_INVALID_SIZE",
                    &err,
                )
                .await
                .map_err(|send_err| send_err.to_string())?;
                return Ok(());
            }
            send_ack(
                &mut *w,
                &device_id,
                &seq_counter,
                req_seq,
                Some(
                    json!({ "status": "ok", "sessionId": session_id, "cols": cols, "rows": rows }),
                ),
            )
            .await
            .map_err(|err| err.to_string())?;
            if let Some(command_id) = command_id {
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "succeeded",
                    Some("resize_pty".to_string()),
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        InboundServerMessage::ClosePty {
            command_id,
            req_seq,
            session_id,
        } => {
            let mut w = write.lock().await;
            let local_session_id = match resolve_remote_pty_session(&app, &session_id) {
                Ok(local_session_id) => local_session_id,
                Err(err) => {
                    send_nack(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        req_seq,
                        "PTY_NOT_FOUND",
                        &err,
                    )
                    .await
                    .map_err(|send_err| send_err.to_string())?;
                    return Ok(());
                }
            };
            let session_store = app.state::<SessionStore>();
            if let Err(err) = close_pty_internal(&local_session_id, &session_store) {
                send_nack(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    req_seq,
                    "PTY_INTERNAL_ERROR",
                    &err,
                )
                .await
                .map_err(|send_err| send_err.to_string())?;
                return Ok(());
            }
            unbind_remote_pty_session(&app, &session_id, &local_session_id)?;
            send_ack(
                &mut *w,
                &device_id,
                &seq_counter,
                req_seq,
                Some(json!({ "status": "ok", "sessionId": session_id, "closing": true })),
            )
            .await
            .map_err(|err| err.to_string())?;
            if let Some(command_id) = command_id {
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    &command_id,
                    "succeeded",
                    Some("close_pty".to_string()),
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        InboundServerMessage::OpenRemoteBrowser {
            command_id,
            req_seq,
            server_id,
            browser_session_id,
        } => {
            let app_handle = app.clone();
            run_blocking_command_action(
                write,
                device_id,
                seq_counter,
                command_id,
                req_seq,
                "open_remote_browser",
                move || {
                    let browser_store = app_handle.state::<RemoteBrowserStore>();
                    let function_store = app_handle.state::<FunctionStore>();
                    let session = open_remote_browser_with_id(
                        &app_handle,
                        server_id,
                        browser_session_id,
                        &browser_store,
                        &function_store,
                        crate::server_tunnel_function_id,
                    )?;
                    Ok(json!({
                        "status": "ok",
                        "browserSessionId": session.browser_id,
                        "rootPath": session.root_path,
                    }))
                },
            )
            .await
        }
        InboundServerMessage::CloseRemoteBrowser {
            command_id,
            req_seq,
            browser_session_id,
        } => {
            let app_handle = app.clone();
            run_blocking_command_action(
                write,
                device_id,
                seq_counter,
                command_id,
                req_seq,
                "close_remote_browser",
                move || {
                    close_remote_browser_action(
                        browser_session_id.clone(),
                        app_handle.state::<RemoteBrowserStore>(),
                    )?;
                    Ok(json!({
                        "status": "ok",
                        "browserSessionId": browser_session_id,
                    }))
                },
            )
            .await
        }
        InboundServerMessage::ListRemoteBrowserChildren {
            command_id,
            req_seq,
            browser_session_id,
            path,
        } => {
            let app_handle = app.clone();
            run_blocking_command_action(
                write,
                device_id,
                seq_counter,
                command_id,
                req_seq,
                "list_remote_browser_children",
                move || {
                    let entries = list_remote_browser_children_action(
                        browser_session_id.clone(),
                        path.clone(),
                        app_handle.state::<RemoteBrowserStore>(),
                    )?;
                    Ok(json!({
                        "status": "ok",
                        "browserSessionId": browser_session_id,
                        "path": path,
                        "entries": entries,
                    }))
                },
            )
            .await
        }
        InboundServerMessage::StatRemoteBrowserPath {
            command_id,
            req_seq,
            browser_session_id,
            path,
        } => {
            let app_handle = app.clone();
            run_blocking_command_action(
                write,
                device_id,
                seq_counter,
                command_id,
                req_seq,
                "stat_remote_browser_path",
                move || {
                    let info = stat_remote_browser_path_action(
                        browser_session_id.clone(),
                        path.clone(),
                        app_handle.state::<RemoteBrowserStore>(),
                    )?;
                    Ok(json!({
                        "status": "ok",
                        "browserSessionId": browser_session_id,
                        "info": info,
                    }))
                },
            )
            .await
        }
        InboundServerMessage::CreateRemoteBrowserDirectory {
            command_id,
            req_seq,
            browser_session_id,
            path,
        } => {
            let app_handle = app.clone();
            run_blocking_command_action(
                write,
                device_id,
                seq_counter,
                command_id,
                req_seq,
                "create_remote_browser_directory",
                move || {
                    create_remote_browser_directory_action(
                        browser_session_id.clone(),
                        path.clone(),
                        app_handle.state::<RemoteBrowserStore>(),
                    )?;
                    Ok(json!({
                        "status": "ok",
                        "browserSessionId": browser_session_id,
                        "path": path,
                    }))
                },
            )
            .await
        }
        InboundServerMessage::DeleteRemoteBrowserPath {
            command_id,
            req_seq,
            browser_session_id,
            path,
            recursive,
        } => {
            let app_handle = app.clone();
            run_blocking_command_action(
                write,
                device_id,
                seq_counter,
                command_id,
                req_seq,
                "delete_remote_browser_path",
                move || {
                    delete_remote_browser_path_action(
                        browser_session_id.clone(),
                        path.clone(),
                        Some(recursive),
                        app_handle.state::<RemoteBrowserStore>(),
                    )?;
                    Ok(json!({
                        "status": "ok",
                        "browserSessionId": browser_session_id,
                        "path": path,
                        "recursive": recursive,
                    }))
                },
            )
            .await
        }
        InboundServerMessage::ScpUpload {
            command_id,
            req_seq,
            transfer_id,
            server_id,
            local_path,
            remote_path,
            recursive,
            overwrite_policy,
            follow_symlinks,
            verify_checksum,
        } => {
            let app_handle = app.clone();
            run_blocking_transfer_command_action(
                write,
                device_id,
                seq_counter,
                command_id,
                req_seq,
                "scp_upload",
                transfer_id.clone(),
                move || {
                    let message = cc_scp_upload_service(
                        &app_handle,
                        server_id.clone(),
                        local_path.clone(),
                        remote_path.clone(),
                        Some(recursive),
                        overwrite_policy.as_deref(),
                        follow_symlinks,
                        verify_checksum,
                    )?;
                    Ok(json!({
                        "status": "ok",
                        "transferId": transfer_id,
                        "message": message,
                    }))
                },
            )
            .await
        }
        InboundServerMessage::ScpDownload {
            command_id,
            req_seq,
            transfer_id,
            server_id,
            remote_path,
            local_path,
            recursive,
            overwrite_policy,
            follow_symlinks,
            verify_checksum,
        } => {
            let app_handle = app.clone();
            run_blocking_transfer_command_action(
                write,
                device_id,
                seq_counter,
                command_id,
                req_seq,
                "scp_download",
                transfer_id.clone(),
                move || {
                    let message = cc_scp_download_service(
                        &app_handle,
                        server_id.clone(),
                        remote_path.clone(),
                        local_path.clone(),
                        Some(recursive),
                        overwrite_policy.as_deref(),
                        follow_symlinks,
                        verify_checksum,
                    )?;
                    Ok(json!({
                        "status": "ok",
                        "transferId": transfer_id,
                        "message": message,
                    }))
                },
            )
            .await
        }
        InboundServerMessage::StartScpUpload {
            command_id,
            req_seq,
            transfer_id,
            server_id,
            local_path,
            remote_path,
            recursive,
            overwrite_policy,
            follow_symlinks,
            verify_checksum,
            resume_token,
            resume_offset_bytes,
            temp_path_hint,
        } => {
            let app_handle = app.clone();
            run_pty_backed_transfer_command_action(
                write,
                device_id,
                seq_counter,
                command_id,
                req_seq,
                "start_scp_upload",
                transfer_id,
                TransferRuntimeDirection::Upload,
                server_id.clone(),
                local_path.clone(),
                remote_path.clone(),
                resume_token.clone(),
                move || {
                    cc_start_scp_upload_service(
                        &app_handle,
                        &app_handle.state::<SessionStore>(),
                        server_id.clone(),
                        local_path.clone(),
                        remote_path.clone(),
                        Some(recursive),
                        overwrite_policy.as_deref(),
                        follow_symlinks,
                        verify_checksum,
                        resume_token.clone(),
                        resume_offset_bytes,
                        temp_path_hint.clone(),
                        open_pty_with_command_builder,
                        update_functions_for_session_exit,
                    )
                },
                &app,
            )
            .await
        }
        InboundServerMessage::StartScpDownload {
            command_id,
            req_seq,
            transfer_id,
            server_id,
            remote_path,
            local_path,
            recursive,
            overwrite_policy,
            follow_symlinks,
            verify_checksum,
            resume_token,
            resume_offset_bytes,
            temp_path_hint,
        } => {
            let app_handle = app.clone();
            run_pty_backed_transfer_command_action(
                write,
                device_id,
                seq_counter,
                command_id,
                req_seq,
                "start_scp_download",
                transfer_id,
                TransferRuntimeDirection::Download,
                server_id.clone(),
                local_path.clone(),
                remote_path.clone(),
                resume_token.clone(),
                move || {
                    cc_start_scp_download_service(
                        &app_handle,
                        &app_handle.state::<SessionStore>(),
                        server_id.clone(),
                        remote_path.clone(),
                        local_path.clone(),
                        Some(recursive),
                        overwrite_policy.as_deref(),
                        follow_symlinks,
                        verify_checksum,
                        resume_token.clone(),
                        resume_offset_bytes,
                        temp_path_hint.clone(),
                        open_pty_with_command_builder,
                        update_functions_for_session_exit,
                    )
                },
                &app,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        command_lifecycle_log_line, local_transfer_status_payload, parse_inbound_server_message,
        InboundServerMessage,
    };
    use crate::cc::protocol::Envelope;
    use crate::stores::TransferRuntimeDirection;
    use serde_json::json;

    #[test]
    fn parses_request_start_server_tunnel_message() {
        let envelope = Envelope {
            msg_type: "request_start_server_tunnel".to_string(),
            device_id: "device-1".to_string(),
            seq: 1,
            ts: "2026-04-23T00:00:00.000Z".to_string(),
            payload: Some(json!({ "serverId": "srv-1" })),
            command_id: Some("cmd-1".to_string()),
            req_seq: None,
        };

        let parsed = parse_inbound_server_message(envelope).expect("expected parser success");

        match parsed {
            InboundServerMessage::RequestStartServerTunnel {
                command_id,
                server_id,
            } => {
                assert_eq!(command_id.as_deref(), Some("cmd-1"));
                assert_eq!(server_id, "srv-1");
            }
            other => panic!("unexpected parsed message: {other:?}"),
        }
    }

    #[test]
    fn parses_request_stop_server_tunnel_message() {
        let envelope = Envelope {
            msg_type: "request_stop_server_tunnel".to_string(),
            device_id: "device-1".to_string(),
            seq: 2,
            ts: "2026-04-23T00:00:00.000Z".to_string(),
            payload: Some(json!({ "serverId": "srv-2" })),
            command_id: Some("cmd-2".to_string()),
            req_seq: None,
        };

        let parsed = parse_inbound_server_message(envelope).expect("expected parser success");

        match parsed {
            InboundServerMessage::RequestStopServerTunnel {
                command_id,
                server_id,
            } => {
                assert_eq!(command_id.as_deref(), Some("cmd-2"));
                assert_eq!(server_id, "srv-2");
            }
            other => panic!("unexpected parsed message: {other:?}"),
        }
    }

    #[test]
    fn parses_request_attach_server_ssh_message() {
        let envelope = Envelope {
            msg_type: "request_attach_server_ssh".to_string(),
            device_id: "device-1".to_string(),
            seq: 4,
            ts: "2026-04-23T00:00:00.000Z".to_string(),
            payload: Some(json!({
                "serverId": "srv-ssh",
                "sessionId": "server-ssh-srv-ssh-remote",
                "localSessionId": "local-ssh-session-1",
                "cols": 120,
                "rows": 32,
            })),
            command_id: Some("cmd-4".to_string()),
            req_seq: None,
        };

        let parsed = parse_inbound_server_message(envelope).expect("expected parser success");

        match parsed {
            InboundServerMessage::RequestAttachServerSsh {
                command_id,
                req_seq,
                server_id,
                session_id,
                local_session_id,
                cols,
                rows,
            } => {
                assert_eq!(command_id.as_deref(), Some("cmd-4"));
                assert_eq!(req_seq, 4);
                assert_eq!(server_id, "srv-ssh");
                assert_eq!(session_id, "server-ssh-srv-ssh-remote");
                assert_eq!(local_session_id, "local-ssh-session-1");
                assert_eq!(cols, 120);
                assert_eq!(rows, 32);
            }
            other => panic!("unexpected parsed message: {other:?}"),
        }
    }

    #[test]
    fn parses_request_pty_scrollback_message() {
        let envelope = Envelope {
            msg_type: "request_pty_scrollback".to_string(),
            device_id: "device-1".to_string(),
            seq: 9,
            ts: "2026-04-23T00:00:00.000Z".to_string(),
            payload: Some(json!({ "sessionId": "remote-pty-1" })),
            command_id: Some("cmd-scroll".to_string()),
            req_seq: None,
        };

        let parsed = parse_inbound_server_message(envelope).expect("expected parser success");

        match parsed {
            InboundServerMessage::RequestPtyScrollback {
                req_seq,
                session_id,
            } => {
                assert_eq!(req_seq, 9);
                assert_eq!(session_id, "remote-pty-1");
            }
            other => panic!("unexpected parsed message: {other:?}"),
        }
    }

    #[test]
    fn parses_request_start_server_ssh_message() {
        let envelope = Envelope {
            msg_type: "request_start_server_ssh".to_string(),
            device_id: "device-1".to_string(),
            seq: 3,
            ts: "2026-04-23T00:00:00.000Z".to_string(),
            payload: Some(json!({
                "serverId": "srv-ssh",
                "sessionId": "ssh-session-1",
                "cols": 120,
                "rows": 32,
            })),
            command_id: Some("cmd-3".to_string()),
            req_seq: None,
        };

        let parsed = parse_inbound_server_message(envelope).expect("expected parser success");

        match parsed {
            InboundServerMessage::RequestStartServerSsh {
                command_id,
                req_seq,
                server_id,
                session_id,
                cols,
                rows,
            } => {
                assert_eq!(command_id.as_deref(), Some("cmd-3"));
                assert_eq!(req_seq, 3);
                assert_eq!(server_id, "srv-ssh");
                assert_eq!(session_id, "ssh-session-1");
                assert_eq!(cols, 120);
                assert_eq!(rows, 32);
            }
            other => panic!("unexpected parsed message: {other:?}"),
        }
    }

    #[test]
    fn forced_local_for_admin_ssh_start_ccpty_matching_device() {
        let sid = crate::cc::pty_session_id::cc_pty_session_id("d1", "loc1").unwrap();
        assert_eq!(
            super::forced_local_session_for_admin_ssh_start("d1", &sid).unwrap(),
            Some("loc1".to_string())
        );
    }

    #[test]
    fn forced_local_for_admin_ssh_start_rejects_device_mismatch() {
        let sid = crate::cc::pty_session_id::cc_pty_session_id("other", "loc1").unwrap();
        let err = super::forced_local_session_for_admin_ssh_start("d1", &sid).unwrap_err();
        assert!(err.contains("expected d1"), "{err}");
        assert!(err.contains("found other"), "{err}");
    }

    #[test]
    fn forced_local_for_admin_ssh_start_non_ccpty_is_none() {
        assert_eq!(
            super::forced_local_session_for_admin_ssh_start("d1", "ssh-legacy")
                .unwrap(),
            None
        );
    }
    #[test]
    fn local_transfer_status_payload_includes_visible_trace_fields() {
        let payload = local_transfer_status_payload(
            "transfer-1",
            Some("session-1"),
            "running",
            "transfer",
            None,
            None,
            Some("start_scp_upload".to_string()),
            TransferRuntimeDirection::Upload,
            "srv-1",
            "/tmp/source.txt",
            "/var/source.txt",
            1,
            false,
        );

        assert_eq!(payload["transferId"], json!("transfer-1"));
        assert_eq!(payload["sessionId"], json!("session-1"));
        assert_eq!(payload["state"], json!("running"));
        assert_eq!(payload["direction"], json!("upload"));
        assert_eq!(payload["serverId"], json!("srv-1"));
        assert_eq!(payload["localPath"], json!("/tmp/source.txt"));
        assert_eq!(payload["remotePath"], json!("/var/source.txt"));
        assert_eq!(payload["message"], json!("start_scp_upload"));
    }

    #[test]
    fn command_lifecycle_log_line_includes_state_outcome_and_reason() {
        let line = command_lifecycle_log_line(
            "status",
            Some("cmd-1"),
            "executing",
            Some("start_server_tunnel:srv-1"),
            Some("ACTION_FAILED"),
            Some("tunnel launch error"),
        );

        assert!(line.contains("stage=status"));
        assert!(line.contains("commandId=cmd-1"));
        assert!(line.contains("state=executing"));
        assert!(line.contains("outcome=start_server_tunnel:srv-1"));
        assert!(line.contains("reasonCode=ACTION_FAILED"));
        assert!(line.contains("reasonDetail=tunnel launch error"));
    }
}

async fn run_blocking_command_action<W, F>(
    write: Arc<Mutex<W>>,
    device_id: String,
    seq_counter: Arc<Mutex<u64>>,
    command_id: Option<String>,
    req_seq: u64,
    action_name: &'static str,
    action: F,
) -> Result<(), String>
where
    W: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin
        + Send
        + 'static,
    F: FnOnce() -> Result<Value, String> + Send + 'static,
{
    if let Some(command_id) = command_id.as_deref() {
        let mut w = write.lock().await;
        send_command_status(
            &mut *w,
            &device_id,
            &seq_counter,
            command_id,
            "executing",
            Some(action_name.to_string()),
            None,
            None,
        )
        .await
        .map_err(|err| err.to_string())?;
    }

    let result = tokio::task::spawn_blocking(action)
        .await
        .map_err(|err| err.to_string())?;

    let mut w = write.lock().await;
    match result {
        Ok(payload) => {
            send_ack(&mut *w, &device_id, &seq_counter, req_seq, Some(payload))
                .await
                .map_err(|err| err.to_string())?;
            if let Some(command_id) = command_id.as_deref() {
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    command_id,
                    "succeeded",
                    Some(action_name.to_string()),
                    None,
                    None,
                )
                .await
                .map_err(|err| err.to_string())?;
            }
        }
        Err(err) => {
            let (reason_code, reason_detail) = split_file_op_reason(&err);
            send_nack(
                &mut *w,
                &device_id,
                &seq_counter,
                req_seq,
                &reason_code,
                &reason_detail,
            )
            .await
            .map_err(|send_err| send_err.to_string())?;
            if let Some(command_id) = command_id.as_deref() {
                send_command_status(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    command_id,
                    "failed",
                    None,
                    Some(reason_code),
                    Some(reason_detail),
                )
                .await
                .map_err(|send_err| send_err.to_string())?;
            }
        }
    }

    Ok(())
}

async fn run_blocking_transfer_command_action<W, F>(
    write: Arc<Mutex<W>>,
    device_id: String,
    seq_counter: Arc<Mutex<u64>>,
    command_id: Option<String>,
    req_seq: u64,
    action_name: &'static str,
    transfer_id: String,
    action: F,
) -> Result<(), String>
where
    W: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin
        + Send
        + 'static,
    F: Fn() -> Result<Value, String> + Send + Clone + 'static,
{
    if let Some(command_id) = command_id.as_deref() {
        let mut w = write.lock().await;
        send_command_status(
            &mut *w,
            &device_id,
            &seq_counter,
            command_id,
            "executing",
            Some(action_name.to_string()),
            None,
            None,
        )
        .await
        .map_err(|err| err.to_string())?;
    }

    for attempt in TRANSFER_ATTEMPT..=TRANSFER_MAX_ATTEMPTS {
        emit_transfer_status(
            &write,
            &device_id,
            &seq_counter,
            &transfer_id,
            "starting",
            "precheck",
            None,
            None,
            Some(action_name.to_string()),
            attempt,
            false,
        )
        .await;
        let result = tokio::task::spawn_blocking(action.clone())
            .await
            .map_err(|err| err.to_string())?;
        match result {
            Ok(payload) => {
                let mut w = write.lock().await;
                send_ack(&mut *w, &device_id, &seq_counter, req_seq, Some(payload))
                    .await
                    .map_err(|err| err.to_string())?;
                if let Some(command_id) = command_id.as_deref() {
                    send_command_status(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        command_id,
                        "succeeded",
                        Some(action_name.to_string()),
                        None,
                        None,
                    )
                    .await
                    .map_err(|err| err.to_string())?;
                }
                emit_transfer_status(
                    &write,
                    &device_id,
                    &seq_counter,
                    &transfer_id,
                    "succeeded",
                    "finalize",
                    None,
                    None,
                    Some(action_name.to_string()),
                    attempt,
                    false,
                )
                .await;
                return Ok(());
            }
            Err(err) => {
                let (reason_code, reason_detail) = split_file_op_reason(&err);
                let will_retry =
                    attempt < TRANSFER_MAX_ATTEMPTS && is_retryable_transfer_reason(&reason_code);
                if will_retry {
                    emit_transfer_status(
                        &write,
                        &device_id,
                        &seq_counter,
                        &transfer_id,
                        "starting",
                        "precheck",
                        Some(reason_code.clone()),
                        Some(reason_detail.clone()),
                        Some(format!(
                            "{action_name} attempt {attempt} failed; retrying in {}s",
                            TRANSFER_RETRY_BACKOFF_SECS
                        )),
                        attempt,
                        true,
                    )
                    .await;
                    tokio::time::sleep(Duration::from_secs(TRANSFER_RETRY_BACKOFF_SECS)).await;
                    continue;
                }

                let mut w = write.lock().await;
                send_nack(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    req_seq,
                    &reason_code,
                    &reason_detail,
                )
                .await
                .map_err(|send_err| send_err.to_string())?;
                if let Some(command_id) = command_id.as_deref() {
                    send_command_status(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        command_id,
                        "failed",
                        None,
                        Some(reason_code.clone()),
                        Some(reason_detail.clone()),
                    )
                    .await
                    .map_err(|send_err| send_err.to_string())?;
                }
                drop(w);
                emit_transfer_status(
                    &write,
                    &device_id,
                    &seq_counter,
                    &transfer_id,
                    "failed",
                    "finalize",
                    Some(reason_code),
                    Some(reason_detail),
                    Some(action_name.to_string()),
                    attempt,
                    false,
                )
                .await;
                return Err(err);
            }
        }
    }

    Err("agent_error: transfer retry loop exhausted unexpectedly".to_string())
}

async fn run_pty_backed_transfer_command_action<W, F>(
    write: Arc<Mutex<W>>,
    device_id: String,
    seq_counter: Arc<Mutex<u64>>,
    command_id: Option<String>,
    req_seq: u64,
    action_name: &'static str,
    transfer_id: String,
    transfer_direction: TransferRuntimeDirection,
    server_id: String,
    local_path: String,
    remote_path: String,
    resume_token: Option<String>,
    action: F,
    app: &AppHandle,
) -> Result<(), String>
where
    W: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin
        + Send
        + 'static,
    F: Fn() -> Result<StartScpResult, String> + Send + Clone + 'static,
{
    if let Some(command_id) = command_id.as_deref() {
        let mut w = write.lock().await;
        send_command_status(
            &mut *w,
            &device_id,
            &seq_counter,
            command_id,
            "executing",
            Some(action_name.to_string()),
            None,
            None,
        )
        .await
        .map_err(|err| err.to_string())?;
    }

    for attempt in TRANSFER_ATTEMPT..=TRANSFER_MAX_ATTEMPTS {
        emit_transfer_status(
            &write,
            &device_id,
            &seq_counter,
            &transfer_id,
            "starting",
            "precheck",
            None,
            None,
            Some(action_name.to_string()),
            attempt,
            false,
        )
        .await;
        emit_local_transfer_status(
            app,
            local_transfer_status_payload(
                &transfer_id,
                None,
                "starting",
                "precheck",
                None,
                None,
                Some(action_name.to_string()),
                transfer_direction,
                &server_id,
                &local_path,
                &remote_path,
                attempt,
                false,
            ),
        );

        let result = tokio::task::spawn_blocking(action.clone())
            .await
            .map_err(|err| err.to_string())?;

        let mut w = write.lock().await;
        match result {
            Ok(start_result) => match start_result {
                StartScpResult::Started {
                    session_id,
                    temp_path_hint,
                } => {
                    let _ = app.state::<CcRuntimeState>().bind_transfer_session(
                        &session_id,
                        TransferRuntimeContext {
                            transfer_id: transfer_id.clone(),
                            direction: transfer_direction,
                            server_id: server_id.clone(),
                            local_path: local_path.clone(),
                            remote_path: remote_path.clone(),
                            temp_path_hint,
                            resume_token: resume_token
                                .clone()
                                .unwrap_or_else(|| next_transfer_resume_token(&transfer_id)),
                        },
                    );
                    emit_local_transfer_status(
                        app,
                        local_transfer_status_payload(
                            &transfer_id,
                            Some(&session_id),
                            "running",
                            "transfer",
                            None,
                            None,
                            Some(action_name.to_string()),
                            transfer_direction,
                            &server_id,
                            &local_path,
                            &remote_path,
                            attempt,
                            false,
                        ),
                    );
                    spawn_transfer_watchdog(
                        write.clone(),
                        device_id.clone(),
                        seq_counter.clone(),
                        transfer_id.clone(),
                        session_id.clone(),
                        app.clone(),
                    );
                    send_ack(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        req_seq,
                        Some(json!({
                            "status": "ok",
                            "transferId": transfer_id,
                            "sessionId": session_id,
                            "attempt": attempt,
                            "maxAttempts": TRANSFER_MAX_ATTEMPTS,
                            "resumeSupported": false,
                            "resumeToken": serde_json::Value::Null,
                            "resumeOffsetBytes": serde_json::Value::Null,
                        })),
                    )
                    .await
                    .map_err(|err| err.to_string())?;
                    let _ = send_ws_event(
                        &mut *w,
                        "transfer_status",
                        &device_id,
                        &seq_counter,
                        json!({
                            "transferId": transfer_id,
                            "state": "running",
                            "direction": serde_json::Value::Null,
                            "serverId": serde_json::Value::Null,
                            "browserSessionId": serde_json::Value::Null,
                            "phase": "transfer",
                            "reasonCode": serde_json::Value::Null,
                            "reasonDetail": serde_json::Value::Null,
                            "message": action_name,
                            "attempt": attempt,
                            "maxAttempts": TRANSFER_MAX_ATTEMPTS,
                            "resumeSupported": false,
                            "retryScheduled": false,
                        }),
                    )
                    .await;
                    if let Some(command_id) = command_id.as_deref() {
                        send_command_status(
                            &mut *w,
                            &device_id,
                            &seq_counter,
                            command_id,
                            "succeeded",
                            Some(action_name.to_string()),
                            None,
                            None,
                        )
                        .await
                        .map_err(|err| err.to_string())?;
                    }
                    return Ok(());
                }
                StartScpResult::Skipped(message) => {
                    send_ack(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        req_seq,
                        Some(json!({
                            "status": "ok",
                            "transferId": transfer_id,
                            "skipped": true,
                            "message": message,
                            "attempt": attempt,
                            "maxAttempts": TRANSFER_MAX_ATTEMPTS,
                            "resumeSupported": false,
                        })),
                    )
                    .await
                    .map_err(|err| err.to_string())?;
                    let _ = send_ws_event(
                        &mut *w,
                        "transfer_status",
                        &device_id,
                        &seq_counter,
                        json!({
                            "transferId": transfer_id,
                            "state": "succeeded",
                            "direction": serde_json::Value::Null,
                            "serverId": serde_json::Value::Null,
                            "browserSessionId": serde_json::Value::Null,
                            "phase": "finalize",
                            "reasonCode": serde_json::Value::Null,
                            "reasonDetail": serde_json::Value::Null,
                            "message": message,
                            "attempt": attempt,
                            "maxAttempts": TRANSFER_MAX_ATTEMPTS,
                            "resumeSupported": false,
                            "retryScheduled": false,
                        }),
                    )
                    .await;
                    emit_local_transfer_status(
                        app,
                        local_transfer_status_payload(
                            &transfer_id,
                            None,
                            "succeeded",
                            "finalize",
                            None,
                            None,
                            Some(message.clone()),
                            transfer_direction,
                            &server_id,
                            &local_path,
                            &remote_path,
                            attempt,
                            false,
                        ),
                    );
                    if let Some(command_id) = command_id.as_deref() {
                        send_command_status(
                            &mut *w,
                            &device_id,
                            &seq_counter,
                            command_id,
                            "succeeded",
                            Some("skipped".to_string()),
                            None,
                            None,
                        )
                        .await
                        .map_err(|err| err.to_string())?;
                    }
                    return Ok(());
                }
            },
            Err(err) => {
                let (reason_code, reason_detail) = split_file_op_reason(&err);
                let will_retry =
                    attempt < TRANSFER_MAX_ATTEMPTS && is_retryable_transfer_reason(&reason_code);
                if will_retry {
                    let _ = send_ws_event(
                        &mut *w,
                        "transfer_status",
                        &device_id,
                        &seq_counter,
                        json!({
                            "transferId": transfer_id,
                            "state": "starting",
                            "direction": serde_json::Value::Null,
                            "serverId": serde_json::Value::Null,
                            "browserSessionId": serde_json::Value::Null,
                            "phase": "precheck",
                            "reasonCode": reason_code,
                            "reasonDetail": reason_detail,
                            "message": format!(
                                "{action_name} attempt {attempt} failed; retrying in {}s",
                                TRANSFER_RETRY_BACKOFF_SECS
                            ),
                            "attempt": attempt,
                            "maxAttempts": TRANSFER_MAX_ATTEMPTS,
                            "resumeSupported": false,
                            "retryScheduled": true,
                        }),
                    )
                    .await;
                    emit_local_transfer_status(
                        app,
                        local_transfer_status_payload(
                            &transfer_id,
                            None,
                            "starting",
                            "precheck",
                            Some(reason_code.clone()),
                            Some(reason_detail.clone()),
                            Some(format!(
                                "{action_name} attempt {attempt} failed; retrying in {}s",
                                TRANSFER_RETRY_BACKOFF_SECS
                            )),
                            transfer_direction,
                            &server_id,
                            &local_path,
                            &remote_path,
                            attempt,
                            true,
                        ),
                    );
                    drop(w);
                    tokio::time::sleep(Duration::from_secs(TRANSFER_RETRY_BACKOFF_SECS)).await;
                    continue;
                }

                send_nack(
                    &mut *w,
                    &device_id,
                    &seq_counter,
                    req_seq,
                    &reason_code,
                    &reason_detail,
                )
                .await
                .map_err(|send_err| send_err.to_string())?;
                let _ = send_ws_event(
                    &mut *w,
                    "transfer_status",
                    &device_id,
                    &seq_counter,
                    json!({
                        "transferId": transfer_id,
                        "state": "failed",
                        "direction": serde_json::Value::Null,
                        "serverId": serde_json::Value::Null,
                        "browserSessionId": serde_json::Value::Null,
                        "phase": "finalize",
                        "reasonCode": reason_code,
                        "reasonDetail": reason_detail,
                        "message": action_name,
                        "attempt": attempt,
                        "maxAttempts": TRANSFER_MAX_ATTEMPTS,
                        "resumeSupported": false,
                        "retryScheduled": false,
                    }),
                )
                .await;
                emit_local_transfer_status(
                    app,
                    local_transfer_status_payload(
                        &transfer_id,
                        None,
                        "failed",
                        "finalize",
                        Some(reason_code.clone()),
                        Some(reason_detail.clone()),
                        Some(action_name.to_string()),
                        transfer_direction,
                        &server_id,
                        &local_path,
                        &remote_path,
                        attempt,
                        false,
                    ),
                );
                if let Some(command_id) = command_id.as_deref() {
                    send_command_status(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        command_id,
                        "failed",
                        None,
                        Some(split_file_op_reason(&err).0),
                        Some(split_file_op_reason(&err).1),
                    )
                    .await
                    .map_err(|send_err| send_err.to_string())?;
                }
                return Err(err);
            }
        }
    }

    Err("agent_error: transfer retry loop exhausted unexpectedly".to_string())
}

fn transfer_direction_label(direction: TransferRuntimeDirection) -> &'static str {
    match direction {
        TransferRuntimeDirection::Upload => "upload",
        TransferRuntimeDirection::Download => "download",
    }
}

fn local_transfer_status_payload(
    transfer_id: &str,
    session_id: Option<&str>,
    state: &str,
    phase: &str,
    reason_code: Option<String>,
    reason_detail: Option<String>,
    message: Option<String>,
    direction: TransferRuntimeDirection,
    server_id: &str,
    local_path: &str,
    remote_path: &str,
    attempt: u64,
    retry_scheduled: bool,
) -> Value {
    json!({
        "transferId": transfer_id,
        "sessionId": session_id,
        "state": state,
        "direction": transfer_direction_label(direction),
        "serverId": server_id,
        "localPath": local_path,
        "remotePath": remote_path,
        "phase": phase,
        "reasonCode": reason_code,
        "reasonDetail": reason_detail,
        "message": message,
        "attempt": attempt,
        "maxAttempts": TRANSFER_MAX_ATTEMPTS,
        "retryScheduled": retry_scheduled,
    })
}

fn emit_local_transfer_status(app: &AppHandle, payload: Value) {
    let _ = app.emit("scp-transfer-status", payload);
}

async fn emit_transfer_status<W>(
    write: &Arc<Mutex<W>>,
    device_id: &str,
    seq_counter: &Arc<Mutex<u64>>,
    transfer_id: &str,
    state: &str,
    phase: &str,
    reason_code: Option<String>,
    reason_detail: Option<String>,
    message: Option<String>,
    attempt: u64,
    retry_scheduled: bool,
) where
    W: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin
        + Send
        + 'static,
{
    let mut w = write.lock().await;
    let _ = send_ws_event(
        &mut *w,
        "transfer_status",
        device_id,
        seq_counter,
        json!({
            "transferId": transfer_id,
            "state": state,
            "direction": serde_json::Value::Null,
            "serverId": serde_json::Value::Null,
            "browserSessionId": serde_json::Value::Null,
            "phase": phase,
            "reasonCode": reason_code,
            "reasonDetail": reason_detail,
            "message": message,
            "attempt": attempt,
            "maxAttempts": TRANSFER_MAX_ATTEMPTS,
            "resumeSupported": false,
            "retryScheduled": retry_scheduled,
        }),
    )
    .await;
}

fn is_retryable_transfer_reason(reason_code: &str) -> bool {
    matches!(reason_code, "agent_error" | "rate_limited")
}

fn split_file_op_reason(detail: &str) -> (String, String) {
    let Some((prefix, rest)) = detail.split_once(':') else {
        return ("ACTION_FAILED".to_string(), detail.to_string());
    };
    let reason_code = prefix.trim();
    if reason_code.is_empty()
        || !reason_code
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch == '_')
    {
        return ("ACTION_FAILED".to_string(), detail.to_string());
    }
    (reason_code.to_string(), rest.trim().to_string())
}

fn spawn_transfer_watchdog<W>(
    write: Arc<Mutex<W>>,
    device_id: String,
    seq_counter: Arc<Mutex<u64>>,
    transfer_id: String,
    local_session_id: String,
    app: AppHandle,
) where
    W: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin
        + Send
        + 'static,
{
    tauri::async_runtime::spawn(async move {
        let started_at = std::time::Instant::now();

        loop {
            tokio::time::sleep(Duration::from_secs(TRANSFER_HEARTBEAT_INTERVAL_SECS)).await;

            let current_transfer_id = app
                .state::<CcRuntimeState>()
                .transfer_id_for_session(&local_session_id)
                .ok()
                .flatten();
            if current_transfer_id.as_deref() != Some(transfer_id.as_str()) {
                return;
            }

            let stalled = app
                .state::<CcRuntimeState>()
                .transfer_last_activity_elapsed(&local_session_id)
                .ok()
                .flatten()
                .map(|elapsed| elapsed >= Duration::from_secs(TRANSFER_STALL_TIMEOUT_SECS))
                .unwrap_or(false);
            if stalled {
                let session_still_open = lock_err(&app.state::<SessionStore>().sessions)
                    .map(|sessions| sessions.contains_key(&local_session_id))
                    .unwrap_or(false);
                if !session_still_open {
                    return;
                }

                let current_transfer_id = app
                    .state::<CcRuntimeState>()
                    .transfer_id_for_session(&local_session_id)
                    .ok()
                    .flatten();
                if current_transfer_id.as_deref() != Some(transfer_id.as_str()) {
                    return;
                }

                if let Ok(Some(mut payload)) =
                    close_pty_internal(&local_session_id, &app.state::<SessionStore>())
                {
                    payload.reason = format!(
                        "transfer_stalled: no PTY output observed for {}s",
                        TRANSFER_STALL_TIMEOUT_SECS
                    );
                    emit_pty_exit(&app, payload);
                }

                return;
            }

            let progress_stalled = app
                .state::<CcRuntimeState>()
                .transfer_last_progress_elapsed(&local_session_id)
                .ok()
                .flatten()
                .map(|elapsed| elapsed >= Duration::from_secs(TRANSFER_PROGRESS_STALL_TIMEOUT_SECS))
                .unwrap_or(false);
            if progress_stalled {
                let session_still_open = lock_err(&app.state::<SessionStore>().sessions)
                    .map(|sessions| sessions.contains_key(&local_session_id))
                    .unwrap_or(false);
                if !session_still_open {
                    return;
                }

                let current_transfer_id = app
                    .state::<CcRuntimeState>()
                    .transfer_id_for_session(&local_session_id)
                    .ok()
                    .flatten();
                if current_transfer_id.as_deref() != Some(transfer_id.as_str()) {
                    return;
                }

                if let Ok(Some(mut payload)) =
                    close_pty_internal(&local_session_id, &app.state::<SessionStore>())
                {
                    payload.reason = format!(
                        "transfer_stalled: no transfer progress observed for {}s",
                        TRANSFER_PROGRESS_STALL_TIMEOUT_SECS
                    );
                    emit_pty_exit(&app, payload);
                }

                return;
            }

            if started_at.elapsed() >= Duration::from_secs(TRANSFER_TOTAL_TIMEOUT_SECS) {
                let session_still_open = lock_err(&app.state::<SessionStore>().sessions)
                    .map(|sessions| sessions.contains_key(&local_session_id))
                    .unwrap_or(false);
                if !session_still_open {
                    return;
                }

                let current_transfer_id = app
                    .state::<CcRuntimeState>()
                    .transfer_id_for_session(&local_session_id)
                    .ok()
                    .flatten();
                if current_transfer_id.as_deref() != Some(transfer_id.as_str()) {
                    return;
                }

                if let Ok(Some(mut payload)) =
                    close_pty_internal(&local_session_id, &app.state::<SessionStore>())
                {
                    payload.reason = format!(
                        "transfer_timeout: exceeded {}s total transfer runtime",
                        TRANSFER_TOTAL_TIMEOUT_SECS
                    );
                    emit_pty_exit(&app, payload);
                }

                return;
            }

            let sampled_progress = app
                .state::<CcRuntimeState>()
                .transfer_context_for_session(&local_session_id)
                .ok()
                .flatten()
                .and_then(|context| {
                    tokio::task::block_in_place(|| sample_transfer_progress(&app, &context))
                });
            if let Some(progress) = sampled_progress.as_ref() {
                let _ = app.state::<CcRuntimeState>().note_transfer_progress(
                    &local_session_id,
                    progress.percent.map(|value| value.floor() as u16),
                    Some(progress.bytes_transferred),
                );
            }

            let mut w = write.lock().await;
            let _ = send_ws_event(
                &mut *w,
                "transfer_progress",
                &device_id,
                &seq_counter,
                json!({
                    "transferId": transfer_id,
                    "bytesTransferred": sampled_progress.as_ref().map(|progress| progress.bytes_transferred).unwrap_or(0),
                    "bytesTotal": sampled_progress.as_ref().and_then(|progress| progress.bytes_total.map(|value| json!(value))).unwrap_or(serde_json::Value::Null),
                    "percent": sampled_progress.as_ref().and_then(|progress| progress.percent.map(|value| json!(value))).unwrap_or(serde_json::Value::Null),
                    "phase": "transfer",
                    "heartbeat": true,
                    "message": sampled_progress.as_ref().and_then(|progress| progress.message.clone()).unwrap_or_else(|| "Transfer heartbeat".to_string()),
                    "attempt": TRANSFER_ATTEMPT,
                    "maxAttempts": TRANSFER_MAX_ATTEMPTS,
                    "resumeSupported": false,
                    "retryScheduled": false,
                }),
            )
            .await;
        }
    });
}

struct SampledTransferProgress {
    bytes_transferred: u64,
    bytes_total: Option<u64>,
    percent: Option<f64>,
    message: Option<String>,
}

fn sample_transfer_progress(
    app: &AppHandle,
    context: &TransferRuntimeContext,
) -> Option<SampledTransferProgress> {
    match context.direction {
        TransferRuntimeDirection::Upload => sample_upload_progress(app, context),
        TransferRuntimeDirection::Download => sample_download_progress(app, context),
    }
}

fn sample_upload_progress(
    app: &AppHandle,
    context: &TransferRuntimeContext,
) -> Option<SampledTransferProgress> {
    let temp_path = context.temp_path_hint.as_deref()?;
    let bytes_total = std::fs::metadata(&context.local_path).ok()?.len();
    let bytes_transferred = sample_remote_file_size(app, &context.server_id, temp_path)?;
    Some(SampledTransferProgress {
        bytes_transferred,
        bytes_total: Some(bytes_total),
        percent: percent_from_bytes(bytes_transferred, bytes_total),
        message: Some("Resuming upload…".to_string()),
    })
}

fn sample_download_progress(
    app: &AppHandle,
    context: &TransferRuntimeContext,
) -> Option<SampledTransferProgress> {
    let local_path = context.temp_path_hint.as_deref()?;
    let bytes_transferred = std::fs::metadata(local_path).ok()?.len();
    let bytes_total = sample_remote_file_size(app, &context.server_id, &context.remote_path)?;
    Some(SampledTransferProgress {
        bytes_transferred,
        bytes_total: Some(bytes_total),
        percent: percent_from_bytes(bytes_transferred, bytes_total),
        message: Some("Resuming download…".to_string()),
    })
}

fn percent_from_bytes(bytes_transferred: u64, bytes_total: u64) -> Option<f64> {
    if bytes_total == 0 {
        return None;
    }
    Some(((bytes_transferred as f64 / bytes_total as f64) * 100.0).clamp(0.0, 100.0))
}

fn sample_remote_file_size(app: &AppHandle, server_id: &str, remote_path: &str) -> Option<u64> {
    let server = load_server_by_id(app, server_id).ok()?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP").ok()?;
    let pw = keyring_get_optional(&keyring_server_ssh_password_account(server_id))
        .ok()
        .flatten();
    if let Some(size) = open_remote_sftp_session(
        &endpoint,
        server.connection_method == ConnectionMethod::Boundary,
        pw.as_deref(),
    )
    .ok()
    .and_then(|session| session.sftp().ok())
    .and_then(|sftp| sftp.lstat(Path::new(remote_path)).ok())
    .and_then(|stat| stat.size)
    {
        return Some(size);
    }
    let mut ssh = Command::new("ssh");
    add_common_ssh_options(&mut ssh, "-p", endpoint.port);
    let output = ssh
        .arg(ssh_target(&endpoint))
        .arg(format!(
            "sh -lc {}",
            shell_quote(&format!("wc -c < {}", shell_quote(remote_path)))
        ))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

fn shell_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\"'\"'"))
}

fn forced_local_session_for_admin_ssh_start(
    device_id: &str,
    session_id: &str,
) -> Result<Option<String>, String> {
    match parse_cc_pty_session_id(session_id) {
        Some((parsed_device, local)) => {
            if parsed_device != device_id {
                return Err(format!(
                    "canonical session id must be for this device (expected {device_id}, found {parsed_device})",
                ));
            }
            Ok(Some(local))
        }
        None => Ok(None),
    }
}

fn bind_remote_pty_session(
    app: &AppHandle,
    remote_session_id: &str,
    local_session_id: &str,
) -> Result<(), String> {
    let store = app.state::<RemotePtyStore>();
    store.bind_session(remote_session_id, local_session_id)
}

fn resolve_remote_pty_session(app: &AppHandle, remote_session_id: &str) -> Result<String, String> {
    let store = app.state::<RemotePtyStore>();
    store.resolve_local_session(remote_session_id).map_err(|_| {
        pty_error(
            "PTY_NOT_FOUND",
            format!("Remote PTY session not found: {remote_session_id}"),
        )
    })
}

fn unbind_remote_pty_session(
    app: &AppHandle,
    remote_session_id: &str,
    local_session_id: &str,
) -> Result<(), String> {
    let store = app.state::<RemotePtyStore>();
    store.unbind_session(remote_session_id, local_session_id)
}

fn resize_local_pty(
    store: &SessionStore,
    local_session_id: &str,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let session = {
        let sessions = lock_err(&store.sessions)?;
        sessions.get(local_session_id).cloned().ok_or_else(|| {
            pty_error(
                "PTY_NOT_FOUND",
                format!("session not found: {local_session_id}"),
            )
        })?
    };

    let master = lock_err(&session.master)?;
    master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| pty_error("PTY_INVALID_SIZE", format!("failed to resize pty: {e}")))?;
    Ok(())
}

fn pty_error_code(detail: &str, fallback: &str) -> String {
    let Some((prefix, _)) = detail.split_once(':') else {
        return fallback.to_string();
    };
    let code = prefix.trim();
    if code.is_empty() || !code.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_') {
        return fallback.to_string();
    }
    code.to_string()
}

fn pty_error(code: &str, detail: impl Into<String>) -> String {
    format!("{code}: {}", detail.into())
}

pub async fn send_ack(
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    device_id: &str,
    seq_counter: &Arc<Mutex<u64>>,
    req_seq: u64,
    result: Option<Value>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let seq = {
        let mut s = seq_counter.lock().await;
        *s += 1;
        *s
    };
    let ack = Envelope::ack(device_id.to_string(), seq, req_seq, result);
    let msg = serde_json::to_string(&ack)?;
    write.send(Message::Text(msg)).await?;
    Ok(())
}

pub async fn send_nack(
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    device_id: &str,
    seq_counter: &Arc<Mutex<u64>>,
    req_seq: u64,
    code: &str,
    message: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let seq = {
        let mut s = seq_counter.lock().await;
        *s += 1;
        *s
    };
    let nack = Envelope::nack(device_id.to_string(), seq, req_seq, code, message);
    let msg = serde_json::to_string(&nack)?;
    write.send(Message::Text(msg)).await?;
    Ok(())
}

pub async fn send_ws_event(
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    msg_type: &str,
    device_id: &str,
    seq_counter: &Arc<Mutex<u64>>,
    payload: serde_json::Value,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let seq = {
        let mut s = seq_counter.lock().await;
        *s += 1;
        *s
    };
    let envelope = Envelope::new(msg_type, device_id.to_string(), seq, Some(payload));
    let msg = serde_json::to_string(&envelope)?;
    write.send(Message::Text(msg)).await?;
    Ok(())
}

fn spawn_restart_task<W, F>(
    write: Arc<Mutex<W>>,
    device_id: String,
    seq_counter: Arc<Mutex<u64>>,
    command_id: Option<String>,
    action_name: &'static str,
    target_id: Option<String>,
    action: F,
) where
    W: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin
        + Send
        + 'static,
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    println!(
        "{}",
        command_lifecycle_log_line(
            "dispatch.spawn",
            command_id.as_deref(),
            "queued",
            Some(action_name),
            None,
            target_id.as_deref(),
        )
    );
    tokio::spawn(async move {
        if let Some(command_id) = command_id.clone() {
            let mut w = write.lock().await;
            let outcome = target_id
                .as_ref()
                .map(|target| format!("{action_name}:{target}"))
                .unwrap_or_else(|| action_name.to_string());
            let _ = send_command_status(
                &mut *w,
                &device_id,
                &seq_counter,
                &command_id,
                "executing",
                Some(outcome),
                None,
                None,
            )
            .await;
        }

        let result = timeout(
            Duration::from_secs(COMMAND_EXECUTION_TIMEOUT_SECS),
            tokio::task::spawn_blocking(action),
        )
        .await;

        if let Some(command_id) = command_id {
            let mut w = write.lock().await;
            match result {
                Ok(Ok(Ok(()))) => {
                    let _ = send_command_status(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        &command_id,
                        "succeeded",
                        Some(action_name.to_string()),
                        None,
                        None,
                    )
                    .await;
                }
                Ok(Ok(Err(err))) => {
                    eprintln!("CC {} failed: {}", action_name, err);
                    let _ = send_command_status(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        &command_id,
                        "failed",
                        None,
                        Some("ACTION_FAILED".to_string()),
                        Some(err),
                    )
                    .await;
                }
                Ok(Err(err)) => {
                    let _ = send_command_status(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        &command_id,
                        "failed",
                        None,
                        Some("JOIN_ERROR".to_string()),
                        Some(err.to_string()),
                    )
                    .await;
                }
                Err(_) => {
                    let detail = target_id
                        .as_ref()
                        .map(|target| {
                            format!(
                                "{}:{} exceeded {}s without a terminal result",
                                action_name, target, COMMAND_EXECUTION_TIMEOUT_SECS
                            )
                        })
                        .unwrap_or_else(|| {
                            format!(
                                "{} exceeded {}s without a terminal result",
                                action_name, COMMAND_EXECUTION_TIMEOUT_SECS
                            )
                        });
                    eprintln!("CC {}", detail);
                    let _ = send_command_status(
                        &mut *w,
                        &device_id,
                        &seq_counter,
                        &command_id,
                        "failed",
                        None,
                        Some("EXECUTION_TIMEOUT".to_string()),
                        Some(detail),
                    )
                    .await;
                }
            }
        }
    });
}

pub async fn send_status_snapshot(
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    device_id: &str,
    seq_counter: &Arc<Mutex<u64>>,
    app: &AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let function_store = app.state::<FunctionStore>();
    let function_states: Vec<_> = {
        let statuses = function_store
            .statuses
            .lock()
            .map_err(|_| "mutex poisoned")?;
        statuses.values().cloned().collect()
    };

    let seq = {
        let mut s = seq_counter.lock().await;
        *s += 1;
        *s
    };
    let agent_run_id = app.state::<CcRuntimeState>().agent_run_id().to_string();
    let snapshot =
        Envelope::status_snapshot(device_id.to_string(), seq, agent_run_id, function_states);
    let msg = serde_json::to_string(&snapshot)?;
    write.send(Message::Text(msg)).await?;
    Ok(())
}

pub async fn send_inventory(
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    device_id: &str,
    seq_counter: &Arc<Mutex<u64>>,
    app: &AppHandle,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let (payload, hash) = build_inventory_payload(app)?;
    let seq = {
        let mut s = seq_counter.lock().await;
        *s += 1;
        *s
    };
    let inventory = Envelope::inventory(device_id.to_string(), seq, payload);
    let msg = serde_json::to_string(&inventory)?;
    write.send(Message::Text(msg)).await?;
    Ok(hash)
}

pub fn build_inventory_payload(
    app: &AppHandle,
) -> Result<(serde_json::Value, String), Box<dyn std::error::Error + Send + Sync>> {
    let servers = load_servers(app).map_err(|err| format!("load servers: {err}"))?;
    let servers_payload: Vec<_> = servers
        .iter()
        .map(|server| {
            json!({
                "id": server.id,
                "name": server.name,
                "host": server.host,
                "os": match server.os {
                    ServerOs::Linux => "linux",
                    ServerOs::Windows => "windows",
                },
                "connectionMethod": match server.connection_method {
                    ConnectionMethod::Ssh => "ssh",
                    ConnectionMethod::Boundary => "boundary",
                },
                "sshPort": server.ssh_port,
                "sshUser": server.ssh_user,
                "servicesCount": server.services.len(),
            })
        })
        .collect();

    let serialized = serde_json::to_string(&servers_payload)?;
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    let hash = format!("{:016x}", hasher.finish());

    let payload = json!({
        "serversCount": servers_payload.len(),
        "serversListHash": hash,
        "servers": servers_payload,
    });

    Ok((payload, hash))
}

pub fn compute_inventory_hash(
    app: &AppHandle,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    build_inventory_payload(app).map(|(_, hash)| hash)
}

pub async fn send_recent_logs(
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    device_id: &str,
    seq_counter: &Arc<Mutex<u64>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let logs = recent_log_events(50);
    let payload = json!({
        "count": logs.len(),
        "events": logs,
    });
    let seq = {
        let mut s = seq_counter.lock().await;
        *s += 1;
        *s
    };
    let recent_logs = Envelope::recent_logs(device_id.to_string(), seq, payload);
    let msg = serde_json::to_string(&recent_logs)?;
    write.send(Message::Text(msg)).await?;
    Ok(())
}

pub async fn send_command_status(
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    device_id: &str,
    seq_counter: &Arc<Mutex<u64>>,
    command_id: &str,
    state: &str,
    outcome: Option<String>,
    reason_code: Option<String>,
    reason_detail: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "{}",
        command_lifecycle_log_line(
            "status",
            Some(command_id),
            state,
            outcome.as_deref(),
            reason_code.as_deref(),
            reason_detail.as_deref(),
        )
    );
    let payload = json!({
        "state": state,
        "outcome": outcome,
        "reasonCode": reason_code,
        "reasonDetail": reason_detail,
    });
    let seq = {
        let mut s = seq_counter.lock().await;
        *s += 1;
        *s
    };
    let status =
        Envelope::command_status(device_id.to_string(), seq, command_id.to_string(), payload);
    let msg = serde_json::to_string(&status)?;
    write.send(Message::Text(msg)).await?;
    Ok(())
}

fn command_lifecycle_log_line(
    stage: &str,
    command_id: Option<&str>,
    state: &str,
    outcome: Option<&str>,
    reason_code: Option<&str>,
    reason_detail: Option<&str>,
) -> String {
    let command_id = command_id.unwrap_or("<missing>");
    let outcome = outcome.unwrap_or("<none>");
    let reason_code = reason_code.unwrap_or("<none>");
    let reason_detail = reason_detail.unwrap_or("<none>");
    format!(
        "CommandLifecycle: stage={stage} commandId={command_id} state={state} outcome={outcome} reasonCode={reason_code} reasonDetail={reason_detail}"
    )
}
