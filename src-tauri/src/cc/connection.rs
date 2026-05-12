use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use std::collections::HashMap;
use std::io::{Error as IoError, ErrorKind};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tauri::{AppHandle, Listener, Manager};
use tokio::sync::Mutex;
use tokio::time::{interval, sleep, Duration, Instant};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

use super::commands::set_cc_status;
use super::dispatcher::{
    compute_inventory_hash, dispatch_inbound_server_message, parse_inbound_server_message,
    send_command_status, send_inventory, send_status_snapshot,
};
use super::protocol::{Envelope, MessageType};
use super::protocol_detector::{detect_protocol, ConnectionProtocol};
use crate::server_mgmt::ConnectionMethod;
use crate::services::{
    add_common_ssh_options, close_pty_internal, keyring_get_optional,
    keyring_server_ssh_password_account, load_server_by_id, open_remote_sftp_session,
    resolve_server_ssh_endpoint, ssh_target,
};
use crate::stores::{
    CcRuntimeState, RemotePtyGcDecision, RemotePtyStore, SessionStore, TransferRuntimeContext,
    TransferRuntimeDirection, REMOTE_PTY_DETACH_GRACE_MS, REMOTE_PTY_IDLE_TIMEOUT_MS,
};
use serde_json::json;

const INVENTORY_PUBLISH_INTERVAL_SECS: u64 = 5 * 60;
const INVENTORY_CHANGE_POLL_SECS: u64 = 30;
const REMOTE_PTY_GC_CHECK_INTERVAL_MS: u64 = 5_000;

fn socket_read_disconnect_status_message(
    _error: &tokio_tungstenite::tungstenite::Error,
) -> &'static str {
    "Connection lost. Reconnecting..."
}

pub struct CcConnection {
    server_url: String,
    agent_token: String,
    device_id: String,
    app: AppHandle,
    generation: u64,
    seq: Arc<Mutex<u64>>,
    recent_command_ids: Arc<Mutex<HashMap<String, u64>>>,
}

impl CcConnection {
    pub fn new(
        server_url: String,
        agent_token: String,
        device_id: String,
        app: AppHandle,
        generation: u64,
    ) -> Self {
        Self {
            server_url,
            agent_token,
            device_id,
            app,
            generation,
            seq: Arc::new(Mutex::new(0)),
            recent_command_ids: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn connect_with_retry(&self) -> Result<(), String> {
        let mut base_delay_ms = 500u64;
        let max_delay_ms = 30000u64;
        let mut attempt = 0;

        loop {
            if !self.is_current_generation() {
                return Ok(());
            }
            attempt += 1;
            println!("CC connection attempt #{}", attempt);
            let _ = set_cc_status(
                &self.app,
                "connecting",
                &format!(
                    "Connecting to {} as {} (attempt #{attempt}).",
                    self.server_url, self.device_id
                ),
            );

            match self.connect().await {
                Ok(_) => {
                    eprintln!("CC connection ended; retrying");
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!("CC connection failed: {}", err_msg);
                }
            }

            if !self.is_current_generation() {
                return Ok(());
            }

            // Jittered exponential backoff
            let jitter = rand::thread_rng().gen_range(0..=(base_delay_ms / 10));
            let delay = base_delay_ms + jitter;
            let _ = set_cc_status(
                &self.app,
                "connecting",
                &format!(
                    "Disconnected from {}. Retrying in {}ms (attempt #{attempt}).",
                    self.server_url, delay
                ),
            );

            println!("Retrying in {}ms...", delay);
            sleep(Duration::from_millis(delay)).await;

            // Exponential backoff: double delay, cap at max
            base_delay_ms = (base_delay_ms * 2).min(max_delay_ms);
        }
    }

    async fn connect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Detect server capability: WebSocket vs SSE
        let protocol = match detect_protocol(&self.server_url, &self.agent_token).await {
            Ok(protocol) => protocol,
            Err(e) => {
                return Err(Box::new(IoError::new(
                    ErrorKind::ConnectionRefused,
                    format!("Protocol detection failed: {}", e),
                )));
            }
        };

        println!("CC using protocol: {:?}", protocol);

        match protocol {
            ConnectionProtocol::WebSocket => {
                return self.connect_websocket().await;
            }
            ConnectionProtocol::Sse => {
                // Delegate to SSE connection
                return self.connect_sse().await;
            }
        }
    }

    async fn connect_websocket(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = self
            .server_url
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        // Handle both cases where URL ends with /agent or not
        let ws_url = if url.trim_end_matches('/').ends_with("/agent") {
            url.trim_end_matches('/').to_string()
        } else {
            format!("{}/agent", url)
        };

        let mut request = ws_url.into_client_request()?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", self.agent_token).parse()?,
        );
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", "scp.cc.v1".parse()?);

        let (ws_stream, _) = connect_async(request).await?;
        let (mut write, mut read) = ws_stream.split();

        if !self.is_current_generation() {
            return Ok(());
        }

        self.app.state::<CcRuntimeState>().set_connected(true);
        let _ = self.app.state::<RemotePtyStore>().mark_all_attached();
        let _ = set_cc_status(
            &self.app,
            "connected",
            &format!("Connected as {} to {}.", self.device_id, self.server_url),
        );
        println!("CC connection established");

        // Send initial status_snapshot
        send_status_snapshot(&mut write, &self.device_id, &self.seq, &self.app).await?;
        let initial_inventory_hash =
            send_inventory(&mut write, &self.device_id, &self.seq, &self.app).await?;

        // Start heartbeat task
        let device_id = self.device_id.clone();
        let agent_run_id = self
            .app
            .state::<CcRuntimeState>()
            .agent_run_id()
            .to_string();
        let seq_counter = self.seq.clone();
        let status_app = self.app.clone();
        let generation = self.generation;
        let write_clone = Arc::new(Mutex::new(write));
        let heartbeat_write = write_clone.clone();

        tokio::spawn(async move {
            let mut heartbeat_interval = interval(Duration::from_secs(15));
            let start_time = std::time::Instant::now();

            loop {
                heartbeat_interval.tick().await;

                if !status_app.state::<CcRuntimeState>().is_connected()
                    || !status_app
                        .state::<CcRuntimeState>()
                        .is_generation_current(generation)
                {
                    break;
                }

                let uptime_sec = start_time.elapsed().as_secs();
                let seq = {
                    let mut s = seq_counter.lock().await;
                    *s += 1;
                    *s
                };

                let heartbeat =
                    Envelope::heartbeat(device_id.clone(), seq, agent_run_id.clone(), uptime_sec);
                if let Ok(msg) = serde_json::to_string(&heartbeat) {
                    let mut w = heartbeat_write.lock().await;
                    if w.send(Message::Text(msg)).await.is_err() {
                        break;
                    }
                }
            }
        });

        let inventory_device_id = self.device_id.clone();
        let inventory_seq = self.seq.clone();
        let inventory_app = self.app.clone();
        let inventory_generation = self.generation;
        let inventory_write = write_clone.clone();

        tokio::spawn(async move {
            let mut inventory_interval = interval(Duration::from_secs(INVENTORY_CHANGE_POLL_SECS));
            let mut last_inventory_hash = initial_inventory_hash;
            let mut last_inventory_sent_at = Instant::now();

            loop {
                inventory_interval.tick().await;

                if !inventory_app.state::<CcRuntimeState>().is_connected()
                    || !inventory_app
                        .state::<CcRuntimeState>()
                        .is_generation_current(inventory_generation)
                {
                    break;
                }

                let Ok(current_hash) = compute_inventory_hash(&inventory_app) else {
                    continue;
                };
                if current_hash == last_inventory_hash
                    && last_inventory_sent_at.elapsed()
                        < Duration::from_secs(INVENTORY_PUBLISH_INTERVAL_SECS)
                {
                    continue;
                }

                let mut w = inventory_write.lock().await;
                match send_inventory(
                    &mut *w,
                    &inventory_device_id,
                    &inventory_seq,
                    &inventory_app,
                )
                .await
                {
                    Ok(sent_hash) => {
                        last_inventory_hash = sent_hash;
                        last_inventory_sent_at = Instant::now();
                    }
                    Err(err) => {
                        eprintln!("CC periodic inventory send failed: {}", err);
                        break;
                    }
                }
            }
        });

        let pty_output_write = write_clone.clone();
        let pty_output_device_id = self.device_id.clone();
        let pty_output_seq = self.seq.clone();
        let pty_output_app = self.app.clone();
        let pty_output_generation = self.generation;
        let pty_output_listener = self.app.listen("pty-output", move |event| {
            let payload = event.payload();
            let Ok(payload_value) = serde_json::from_str::<serde_json::Value>(payload) else {
                return;
            };
            let Some(local_session_id) = payload_value.get("id").and_then(serde_json::Value::as_str)
            else {
                return;
            };
            let transfer_context = pty_output_app
                .state::<CcRuntimeState>()
                .transfer_context_for_session(local_session_id)
                .ok()
                .flatten();
            let transfer_id = transfer_context
                .as_ref()
                .map(|context| context.transfer_id.clone());
            let Some(remote_session_id) =
                remote_pty_session_for_local(&pty_output_app, local_session_id)
            else {
                if let Some(transfer_id) = transfer_id {
                    let Some(output_data) = payload_value
                        .get("data")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string)
                    else {
                        return;
                    };
                    let write = pty_output_write.clone();
                    let device_id = pty_output_device_id.clone();
                    let seq_counter = pty_output_seq.clone();
                    let app = pty_output_app.clone();
                    let local_session_id = local_session_id.to_string();
                    tauri::async_runtime::spawn(async move {
                        if !app.state::<CcRuntimeState>().is_connected()
                            || !app
                                .state::<CcRuntimeState>()
                                .is_generation_current(pty_output_generation)
                        {
                            return;
                        }
                        let progress_percent = extract_scp_progress_percent(&output_data);
                        let _ = app
                            .state::<CcRuntimeState>()
                            .note_transfer_activity(&local_session_id);
                        let _ = app
                            .state::<CcRuntimeState>()
                            .note_transfer_progress(&local_session_id, progress_percent, None);
                        let mut w = write.lock().await;
                        let _ = send_ws_event(
                            &mut *w,
                            "transfer_progress",
                            &device_id,
                            &seq_counter,
                            json!({
                                "transferId": transfer_id,
                                "bytesTransferred": 0,
                                "bytesTotal": serde_json::Value::Null,
                                "percent": progress_percent.map(|value| json!(value)).unwrap_or(serde_json::Value::Null),
                                "throughputBps": serde_json::Value::Null,
                                "etaSec": serde_json::Value::Null,
                                "phase": "transfer",
                                "resumeSupported": false,
                                "resumeToken": serde_json::Value::Null,
                                "resumeOffsetBytes": serde_json::Value::Null,
                                "tempPathHint": serde_json::Value::Null,
                            }),
                        )
                        .await;
                    });
                }
                return;
            };
            let Some(output_data) = payload_value
                .get("data")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
            else {
                return;
            };
            if let Some(transfer_id) = transfer_id {
                let write = pty_output_write.clone();
                let device_id = pty_output_device_id.clone();
                let seq_counter = pty_output_seq.clone();
                let app = pty_output_app.clone();
                let local_session_id = local_session_id.to_string();
                let transfer_output = output_data.clone();
                let transfer_context = transfer_context.clone();
                tauri::async_runtime::spawn(async move {
                    if !app.state::<CcRuntimeState>().is_connected()
                        || !app
                            .state::<CcRuntimeState>()
                            .is_generation_current(pty_output_generation)
                    {
                        return;
                    }
                    let progress_percent = extract_scp_progress_percent(&transfer_output);
                    let _ = app
                        .state::<CcRuntimeState>()
                        .note_transfer_activity(&local_session_id);
                    let _ = app
                        .state::<CcRuntimeState>()
                        .note_transfer_progress(&local_session_id, progress_percent, None);
                    let resume_fields = transfer_context
                        .as_ref()
                        .map(progress_resume_fields)
                        .unwrap_or_default();
                    let mut w = write.lock().await;
                    let _ = send_ws_event(
                        &mut *w,
                        "transfer_progress",
                        &device_id,
                        &seq_counter,
                        json!({
                            "transferId": transfer_id,
                            "bytesTransferred": 0,
                            "bytesTotal": serde_json::Value::Null,
                            "percent": progress_percent.map(|value| json!(value)).unwrap_or(serde_json::Value::Null),
                            "throughputBps": serde_json::Value::Null,
                            "etaSec": serde_json::Value::Null,
                            "phase": "transfer",
                            "resumeSupported": resume_fields.resume_supported,
                            "resumeToken": resume_fields.resume_token,
                            "resumeOffsetBytes": resume_fields.resume_offset_bytes,
                            "tempPathHint": resume_fields.temp_path_hint,
                        }),
                    )
                    .await;
                });
            }

            let should_spawn_flush = match pty_output_app
                .state::<RemotePtyStore>()
                .enqueue_output(&remote_session_id, &output_data)
            {
                Ok(should_spawn_flush) => should_spawn_flush,
                Err(err) => {
                    eprintln!("CC PTY output enqueue failed: {err}");
                    return;
                }
            };
            if !should_spawn_flush {
                return;
            }

            let write = pty_output_write.clone();
            let device_id = pty_output_device_id.clone();
            let seq_counter = pty_output_seq.clone();
            let app = pty_output_app.clone();
            let remote_session_id_for_flush = remote_session_id.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    if !app.state::<CcRuntimeState>().is_connected()
                        || !app
                            .state::<CcRuntimeState>()
                            .is_generation_current(pty_output_generation)
                    {
                        let _ = app
                            .state::<RemotePtyStore>()
                            .abort_output_flush(&remote_session_id_for_flush);
                        return;
                    }

                    let next_chunk = match app
                        .state::<RemotePtyStore>()
                        .take_output_chunk(&remote_session_id_for_flush)
                    {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) => return,
                        Err(err) => {
                            eprintln!("CC PTY output drain failed: {err}");
                            let _ = app
                                .state::<RemotePtyStore>()
                                .abort_output_flush(&remote_session_id_for_flush);
                            return;
                        }
                    };

                    let mut w = write.lock().await;
                    if let Err(err) = send_ws_event(
                        &mut *w,
                        "pty-output",
                        &device_id,
                        &seq_counter,
                        json!({
                            "sessionId": remote_session_id_for_flush,
                            "data": next_chunk,
                        }),
                    )
                    .await
                    {
                        eprintln!("CC PTY output send failed: {err}");
                        let _ = app
                            .state::<RemotePtyStore>()
                            .abort_output_flush(&remote_session_id_for_flush);
                        return;
                    }
                }
            });
        });

        let pty_exit_write = write_clone.clone();
        let pty_exit_device_id = self.device_id.clone();
        let pty_exit_seq = self.seq.clone();
        let pty_exit_app = self.app.clone();
        let pty_exit_generation = self.generation;
        let pty_exit_listener = self.app.listen("pty-exit", move |event| {
            let payload = event.payload();
            let Ok(mut payload_value) = serde_json::from_str::<serde_json::Value>(payload) else {
                return;
            };
            let Some(local_session_id) = payload_value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
            else {
                return;
            };
            let transfer_context = pty_exit_app
                .state::<CcRuntimeState>()
                .unbind_transfer_session(&local_session_id)
                .ok()
                .flatten();
            let transfer_id = transfer_context
                .as_ref()
                .map(|context| context.transfer_id.clone());
            let Some(remote_session_id) =
                remote_pty_session_for_local(&pty_exit_app, &local_session_id)
            else {
                if let Some(transfer_id) = transfer_id {
                    let write = pty_exit_write.clone();
                    let device_id = pty_exit_device_id.clone();
                    let seq_counter = pty_exit_seq.clone();
                    let app = pty_exit_app.clone();
                    let transfer_context = transfer_context.clone();
                    tauri::async_runtime::spawn(async move {
                        if !app.state::<CcRuntimeState>().is_connected()
                            || !app
                                .state::<CcRuntimeState>()
                                .is_generation_current(pty_exit_generation)
                        {
                            return;
                        }
                        let mut w = write.lock().await;
                        let succeeded = payload_value
                            .get("exitCode")
                            .and_then(serde_json::Value::as_i64)
                            == Some(0);
                        let resume_fields = if succeeded {
                            ResumeTelemetryFields::default()
                        } else {
                            transfer_context
                                .as_ref()
                                .map(|context| derive_resume_fields(&app, context))
                                .unwrap_or_default()
                        };
                        let reason_detail = payload_value
                            .get("reason")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        let reason_code = if succeeded {
                            serde_json::Value::Null
                        } else {
                            json!(transfer_reason_code(reason_detail))
                        };
                        let _ = send_ws_event(
                            &mut *w,
                            "transfer_status",
                            &device_id,
                            &seq_counter,
                            json!({
                                "transferId": transfer_id,
                                "state": if succeeded { "succeeded" } else { "failed" },
                                "direction": serde_json::Value::Null,
                                "serverId": serde_json::Value::Null,
                                "browserSessionId": serde_json::Value::Null,
                                "phase": "finalize",
                                "reasonCode": reason_code,
                                "reasonDetail": payload_value.get("reason").cloned().unwrap_or(serde_json::Value::Null),
                                "message": payload_value.get("reason").cloned().unwrap_or(serde_json::Value::Null),
                                "resumeSupported": resume_fields.resume_supported,
                                "resumeToken": resume_fields.resume_token,
                                "resumeOffsetBytes": resume_fields.resume_offset_bytes,
                                "tempPathHint": resume_fields.temp_path_hint,
                            }),
                        )
                        .await;
                    });
                }
                return;
            };
            if let Some(object) = payload_value.as_object_mut() {
                object.remove("id");
                object.insert(
                    "sessionId".to_string(),
                    serde_json::Value::String(remote_session_id.clone()),
                );
            }
            let _ = clear_remote_pty_session_mapping(
                &pty_exit_app,
                &local_session_id,
                &remote_session_id,
            );

            let write = pty_exit_write.clone();
            let device_id = pty_exit_device_id.clone();
            let seq_counter = pty_exit_seq.clone();
            let app = pty_exit_app.clone();
            let transfer_id_for_event = transfer_id.clone();
            let transfer_context_for_event = transfer_context.clone();
            tauri::async_runtime::spawn(async move {
                if !app.state::<CcRuntimeState>().is_connected()
                    || !app
                        .state::<CcRuntimeState>()
                        .is_generation_current(pty_exit_generation)
                {
                    return;
                }
                let mut w = write.lock().await;
                if let Some(transfer_id) = transfer_id_for_event {
                    let succeeded = payload_value
                        .get("exitCode")
                        .and_then(serde_json::Value::as_i64)
                        == Some(0);
                    let resume_fields = if succeeded {
                        ResumeTelemetryFields::default()
                    } else {
                        transfer_context_for_event
                            .as_ref()
                            .map(|context| derive_resume_fields(&app, context))
                            .unwrap_or_default()
                    };
                    let reason_detail = payload_value
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let reason_code = if succeeded {
                        serde_json::Value::Null
                    } else {
                        json!(transfer_reason_code(reason_detail))
                    };
                    let _ = send_ws_event(
                        &mut *w,
                        "transfer_status",
                        &device_id,
                        &seq_counter,
                        json!({
                            "transferId": transfer_id,
                            "state": if succeeded { "succeeded" } else { "failed" },
                            "direction": serde_json::Value::Null,
                            "serverId": serde_json::Value::Null,
                            "browserSessionId": serde_json::Value::Null,
                            "phase": "finalize",
                            "reasonCode": reason_code,
                            "reasonDetail": payload_value.get("reason").cloned().unwrap_or(serde_json::Value::Null),
                            "message": payload_value.get("reason").cloned().unwrap_or(serde_json::Value::Null),
                            "resumeSupported": resume_fields.resume_supported,
                            "resumeToken": resume_fields.resume_token,
                            "resumeOffsetBytes": resume_fields.resume_offset_bytes,
                            "tempPathHint": resume_fields.temp_path_hint,
                        }),
                    )
                    .await;
                }
                let _ = send_ws_event(
                    &mut *w,
                    "pty-exit",
                    &device_id,
                    &seq_counter,
                    payload_value,
                )
                .await;
            });
        });

        // Read messages from server
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(envelope) = serde_json::from_str::<Envelope>(&text) {
                        println!("Received from CC: type={}", envelope.msg_type);
                        match MessageType::from(envelope.msg_type.as_str()) {
                            MessageType::Welcome
                            | MessageType::Ack
                            | MessageType::Nack
                            | MessageType::Heartbeat
                            | MessageType::StatusSnapshot
                            | MessageType::Inventory
                            | MessageType::RecentLogs
                            | MessageType::CommandStatus
                            | MessageType::FunctionStatus => {
                                continue;
                            }
                            _ => {}
                        }
                        let parsed_message = match parse_inbound_server_message(envelope) {
                            Ok(message) => message,
                            Err(err) => {
                                eprintln!("CC inbound protocol error: {}", err.reason_detail);
                                if let Some(command_id) = err.command_id {
                                    let mut w = write_clone.lock().await;
                                    let _ = send_command_status(
                                        &mut *w,
                                        &self.device_id,
                                        &self.seq,
                                        &command_id,
                                        "failed",
                                        None,
                                        Some(err.reason_code),
                                        Some(err.reason_detail),
                                    )
                                    .await;
                                }
                                continue;
                            }
                        };
                        if let Some(command_id) = parsed_message.command_id().map(str::to_string) {
                            if self.is_duplicate_command(&command_id).await {
                                let mut w = write_clone.lock().await;
                                let _ = send_command_status(
                                    &mut *w,
                                    &self.device_id,
                                    &self.seq,
                                    &command_id,
                                    "acknowledged",
                                    Some("duplicate_suppressed".to_string()),
                                    Some("DUPLICATE_COMMAND".to_string()),
                                    Some(
                                        "Duplicate commandId received recently; side effects were suppressed."
                                            .to_string(),
                                    ),
                                )
                                .await;
                                continue;
                            }
                        }
                        if let Err(err) = dispatch_inbound_server_message(
                            parsed_message,
                            write_clone.clone(),
                            self.device_id.clone(),
                            self.seq.clone(),
                            self.app.clone(),
                        )
                        .await
                        {
                            eprintln!("CC inbound dispatch failed: {}", err);
                            break;
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    self.app.unlisten(pty_output_listener);
                    self.app.unlisten(pty_exit_listener);
                    schedule_remote_pty_gc(&self.app);
                    println!("CC connection closed by server");
                    if !self.is_current_generation() {
                        return Ok(());
                    }
                    self.app.state::<CcRuntimeState>().set_connected(false);
                    let _ =
                        set_cc_status(&self.app, "disconnected", "Connection closed by server.");
                    return Err(IoError::new(
                        ErrorKind::ConnectionAborted,
                        "CC connection closed by server",
                    )
                    .into());
                }
                Err(e) => {
                    self.app.unlisten(pty_output_listener);
                    self.app.unlisten(pty_exit_listener);
                    schedule_remote_pty_gc(&self.app);
                    eprintln!("CC read error: {}", e);
                    if !self.is_current_generation() {
                        return Ok(());
                    }
                    self.app.state::<CcRuntimeState>().set_connected(false);
                    let _ = set_cc_status(
                        &self.app,
                        "disconnected",
                        socket_read_disconnect_status_message(&e),
                    );
                    return Err(IoError::new(
                        ErrorKind::ConnectionAborted,
                        format!("CC read error: {e}"),
                    )
                    .into());
                }
                _ => {}
            }
        }

        self.app.unlisten(pty_output_listener);
        self.app.unlisten(pty_exit_listener);
        schedule_remote_pty_gc(&self.app);
        if !self.is_current_generation() {
            return Ok(());
        }
        self.app.state::<CcRuntimeState>().set_connected(false);
        let _ = set_cc_status(&self.app, "disconnected", "CC connection stream ended.");
        Err(IoError::new(ErrorKind::UnexpectedEof, "CC connection stream ended").into())
    }

    async fn next_seq(&self) -> u64 {
        let mut seq = self.seq.lock().await;
        *seq += 1;
        *seq
    }

    async fn is_duplicate_command(&self, command_id: &str) -> bool {
        const DUPLICATE_TTL_MS: u64 = 10 * 60 * 1000;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut seen = self.recent_command_ids.lock().await;
        seen.retain(|_, ts| now_ms.saturating_sub(*ts) <= DUPLICATE_TTL_MS);
        if seen.contains_key(command_id) {
            return true;
        }
        seen.insert(command_id.to_string(), now_ms);
        false
    }

    fn is_current_generation(&self) -> bool {
        self.app
            .state::<CcRuntimeState>()
            .is_generation_current(self.generation)
    }

    async fn connect_sse(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Create SSE connection and run it
        let sse = super::sse_connection::SseConnection::new(
            self.server_url.clone(),
            self.agent_token.clone(),
            self.device_id.clone(),
            self.app.clone(),
            self.generation,
        );

        match sse.connect_with_retry().await {
            Ok(_) => {
                // SSE connection ended normally (generation not current)
                Ok(())
            }
            Err(e) => {
                // SSE connection failed
                Err(Box::new(IoError::new(ErrorKind::ConnectionAborted, e))
                    as Box<dyn std::error::Error + Send + Sync>)
            }
        }
    }
}

fn remote_pty_session_for_local(app: &AppHandle, local_session_id: &str) -> Option<String> {
    let store = app.state::<RemotePtyStore>();
    store.remote_session_for_local(local_session_id)
}

fn clear_remote_pty_session_mapping(
    app: &AppHandle,
    local_session_id: &str,
    remote_session_id: &str,
) -> Result<(), String> {
    let store = app.state::<RemotePtyStore>();
    if let Some(found_remote_session_id) = store.clear_session_for_local(local_session_id)? {
        if found_remote_session_id != remote_session_id {
            return Err(format!(
                "remote pty session mismatch while clearing local session {local_session_id}: expected {remote_session_id}, found {found_remote_session_id}"
            ));
        }
    }
    Ok(())
}

async fn send_ws_event(
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

fn schedule_remote_pty_gc(app: &AppHandle) {
    let detached_at_ms = now_ms();
    let snapshots = match app
        .state::<RemotePtyStore>()
        .mark_all_detached(detached_at_ms)
    {
        Ok(snapshots) => snapshots,
        Err(err) => {
            eprintln!("CC remote PTY detach bookkeeping failed: {err}");
            return;
        }
    };

    for snapshot in snapshots {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if snapshot.local_session_id.is_empty() {
                return;
            }

            sleep(Duration::from_millis(REMOTE_PTY_DETACH_GRACE_MS)).await;
            loop {
                match app.state::<RemotePtyStore>().gc_decision(
                    &snapshot.remote_session_id,
                    snapshot.detach_generation,
                    now_ms(),
                ) {
                    Ok(RemotePtyGcDecision::Cancel) => return,
                    Ok(RemotePtyGcDecision::Wait) => {
                        sleep(Duration::from_millis(REMOTE_PTY_GC_CHECK_INTERVAL_MS)).await;
                    }
                    Ok(RemotePtyGcDecision::Close { local_session_id }) => {
                        let session_store = app.state::<SessionStore>();
                        match close_pty_internal(&local_session_id, &session_store) {
                            Ok(_) => {
                                let _ = clear_remote_pty_session_mapping(
                                    &app,
                                    &local_session_id,
                                    &snapshot.remote_session_id,
                                );
                                eprintln!(
                                    "CC remote PTY {} closed after {}ms detached idle timeout.",
                                    snapshot.remote_session_id, REMOTE_PTY_IDLE_TIMEOUT_MS
                                );
                            }
                            Err(err) => {
                                let _ = clear_remote_pty_session_mapping(
                                    &app,
                                    &local_session_id,
                                    &snapshot.remote_session_id,
                                );
                                eprintln!(
                                    "CC remote PTY {} cleanup close failed after detach timeout: {}",
                                    snapshot.remote_session_id, err
                                );
                            }
                        }
                        return;
                    }
                    Err(err) => {
                        eprintln!("CC remote PTY GC decision failed: {err}");
                        return;
                    }
                }
            }
        });
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Default)]
struct ResumeTelemetryFields {
    resume_supported: bool,
    resume_token: serde_json::Value,
    resume_offset_bytes: serde_json::Value,
    temp_path_hint: serde_json::Value,
}

fn progress_resume_fields(context: &TransferRuntimeContext) -> ResumeTelemetryFields {
    ResumeTelemetryFields {
        resume_supported: false,
        resume_token: serde_json::Value::Null,
        resume_offset_bytes: serde_json::Value::Null,
        temp_path_hint: context
            .temp_path_hint
            .as_ref()
            .map(|value| json!(value))
            .unwrap_or(serde_json::Value::Null),
    }
}

fn derive_resume_fields(
    app: &AppHandle,
    context: &TransferRuntimeContext,
) -> ResumeTelemetryFields {
    let offset = match context.direction {
        TransferRuntimeDirection::Upload => derive_remote_resume_offset(app, context),
        TransferRuntimeDirection::Download => derive_local_resume_offset(context),
    };

    match offset {
        Some(offset) if offset > 0 => ResumeTelemetryFields {
            resume_supported: true,
            resume_token: json!(context.resume_token),
            resume_offset_bytes: json!(offset),
            temp_path_hint: context
                .temp_path_hint
                .as_ref()
                .map(|value| json!(value))
                .unwrap_or(serde_json::Value::Null),
        },
        _ => ResumeTelemetryFields {
            resume_supported: false,
            resume_token: serde_json::Value::Null,
            resume_offset_bytes: serde_json::Value::Null,
            temp_path_hint: context
                .temp_path_hint
                .as_ref()
                .map(|value| json!(value))
                .unwrap_or(serde_json::Value::Null),
        },
    }
}

fn derive_remote_resume_offset(app: &AppHandle, context: &TransferRuntimeContext) -> Option<u64> {
    let temp_path = context.temp_path_hint.as_deref()?;
    let server = load_server_by_id(app, &context.server_id).ok()?;
    let endpoint = resolve_server_ssh_endpoint(&server, "SCP").ok()?;
    let pw = keyring_get_optional(&keyring_server_ssh_password_account(&context.server_id))
        .ok()
        .flatten();
    if let Some(size) = open_remote_sftp_session(
        &endpoint,
        server.connection_method == ConnectionMethod::Boundary,
        pw.as_deref(),
    )
    .ok()
    .and_then(|session| session.sftp().ok())
    .and_then(|sftp| sftp.lstat(Path::new(temp_path)).ok())
    .and_then(|stat| stat.size)
    {
        return Some(size);
    }
    derive_remote_resume_offset_via_ssh(&endpoint, temp_path)
}

fn derive_remote_resume_offset_via_ssh(
    endpoint: &crate::models::SshEndpoint,
    temp_path: &str,
) -> Option<u64> {
    let mut ssh = Command::new("ssh");
    add_common_ssh_options(&mut ssh, "-p", endpoint.port);
    let output = ssh
        .arg(ssh_target(endpoint))
        .arg(format!("sh -lc 'wc -c < {}'", shell_quote(temp_path)))
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

fn derive_local_resume_offset(context: &TransferRuntimeContext) -> Option<u64> {
    let temp_path = context.temp_path_hint.as_deref()?;
    std::fs::metadata(temp_path)
        .ok()
        .map(|metadata| metadata.len())
}

fn transfer_reason_code(reason: &str) -> String {
    let Some((prefix, _)) = reason.split_once(':') else {
        return "ACTION_FAILED".to_string();
    };
    let code = prefix.trim();
    if code.is_empty() || !code.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_') {
        "ACTION_FAILED".to_string()
    } else {
        code.to_string()
    }
}

fn extract_scp_progress_percent(output: &str) -> Option<u16> {
    let mut last = None;
    for token in output.split_whitespace() {
        if let Some(number) = token.strip_suffix('%') {
            if let Ok(value) = number.parse::<u16>() {
                last = Some(value.min(100));
            }
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::socket_read_disconnect_status_message;
    use std::io::{Error as IoError, ErrorKind};
    use tokio_tungstenite::tungstenite::Error as WsError;

    #[test]
    fn socket_read_disconnect_status_hides_raw_network_loss_error() {
        let error = WsError::Io(IoError::new(
            ErrorKind::ConnectionAborted,
            "Network connection lost.",
        ));

        assert_eq!(
            socket_read_disconnect_status_message(&error),
            "Connection lost. Reconnecting..."
        );
    }
}
