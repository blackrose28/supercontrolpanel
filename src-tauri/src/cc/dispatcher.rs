use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message;

use super::protocol::{Envelope, MessageType};
use super::telemetry::recent_log_events;
use crate::commands::{
    boundary::restart_boundary_auth_action,
    openvpn::restart_openvpn_action,
    pty::open_pty_internal,
    server::{restart_server_ssh_action, restart_server_tunnel_action},
};
use crate::server_mgmt::{load_servers, ConnectionMethod, ServerOs};
use crate::services::{close_pty_internal, lock_err, write_pty_internal};
use crate::stores::{RemotePtyStore, SessionStore};
use portable_pty::PtySize;

const COMMAND_EXECUTION_TIMEOUT_SECS: u64 = 60;

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
    RequestStatusSnapshot { command_id: Option<String> },
    RequestInventory { command_id: Option<String> },
    RequestRecentLogs { command_id: Option<String> },
    RequestRestartOpenvpn { command_id: Option<String> },
    RequestRestartBoundaryAuth { command_id: Option<String> },
    RequestRestartServerTunnel {
        command_id: Option<String>,
        server_id: String,
    },
    RequestRestartServerSsh {
        command_id: Option<String>,
        server_id: String,
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
}

impl InboundServerMessage {
    pub fn command_id(&self) -> Option<&str> {
        match self {
            Self::RequestStatusSnapshot { command_id }
            | Self::RequestInventory { command_id }
            | Self::RequestRecentLogs { command_id }
            | Self::RequestRestartOpenvpn { command_id }
            | Self::RequestRestartBoundaryAuth { command_id }
            | Self::RequestRestartServerTunnel { command_id, .. }
            | Self::RequestRestartServerSsh { command_id, .. }
            | Self::OpenPty { command_id, .. }
            | Self::WritePty { command_id, .. }
            | Self::ResizePty { command_id, .. }
            | Self::ClosePty { command_id, .. } => command_id.as_deref(),
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

pub fn parse_inbound_server_message(
    envelope: Envelope,
) -> Result<InboundServerMessage, InboundServerMessageError> {
    let command_id = envelope.command_id.clone();

    match MessageType::from(envelope.msg_type.as_str()) {
        MessageType::RequestStatusSnapshot => {
            Ok(InboundServerMessage::RequestStatusSnapshot { command_id })
        }
        MessageType::RequestInventory => Ok(InboundServerMessage::RequestInventory { command_id }),
        MessageType::RequestRecentLogs => Ok(InboundServerMessage::RequestRecentLogs { command_id }),
        MessageType::RequestRestartOpenvpn => {
            Ok(InboundServerMessage::RequestRestartOpenvpn { command_id })
        }
        MessageType::RequestRestartBoundaryAuth => {
            Ok(InboundServerMessage::RequestRestartBoundaryAuth { command_id })
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
                send_status_snapshot(&mut *w, &device_id, &seq_counter)
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
                send_status_snapshot(&mut *w, &device_id, &seq_counter)
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
                Some(json!({ "status": "ok", "sessionId": session_id, "acceptedBytes": data.len() })),
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
                Some(json!({ "status": "ok", "sessionId": session_id, "cols": cols, "rows": rows })),
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
    store
        .resolve_local_session(remote_session_id)
        .map_err(|_| pty_error("PTY_NOT_FOUND", format!("Remote PTY session not found: {remote_session_id}")))
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
        sessions
            .get(local_session_id)
            .cloned()
            .ok_or_else(|| pty_error("PTY_NOT_FOUND", format!("session not found: {local_session_id}")))?
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let seq = {
        let mut s = seq_counter.lock().await;
        *s += 1;
        *s
    };
    let snapshot = Envelope::status_snapshot(device_id.to_string(), seq);
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
    let status = Envelope::command_status(device_id.to_string(), seq, command_id.to_string(), payload);
    let msg = serde_json::to_string(&status)?;
    write.send(Message::Text(msg)).await?;
    Ok(())
}
