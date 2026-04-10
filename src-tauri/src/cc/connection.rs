use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use std::collections::HashMap;
use std::io::{Error as IoError, ErrorKind};
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
use crate::services::close_pty_internal;
use crate::stores::{
    CcRuntimeState, RemotePtyGcDecision, RemotePtyStore, SessionStore, REMOTE_PTY_DETACH_GRACE_MS,
    REMOTE_PTY_IDLE_TIMEOUT_MS,
};
use serde_json::json;

const INVENTORY_PUBLISH_INTERVAL_SECS: u64 = 5 * 60;
const INVENTORY_CHANGE_POLL_SECS: u64 = 30;
const REMOTE_PTY_GC_CHECK_INTERVAL_MS: u64 = 5_000;

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
        let url = self
            .server_url
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        let ws_url = format!("{}/agent", url);

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
        send_status_snapshot(&mut write, &self.device_id, &self.seq).await?;
        let initial_inventory_hash =
            send_inventory(&mut write, &self.device_id, &self.seq, &self.app).await?;

        // Start heartbeat task
        let device_id = self.device_id.clone();
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

                let heartbeat = Envelope::heartbeat(device_id.clone(), seq, uptime_sec);
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
                match send_inventory(&mut *w, &inventory_device_id, &inventory_seq, &inventory_app)
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
            let Some(remote_session_id) =
                remote_pty_session_for_local(&pty_output_app, local_session_id)
            else {
                return;
            };
            let Some(output_data) = payload_value
                .get("data")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
            else {
                return;
            };

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
            let Some(remote_session_id) =
                remote_pty_session_for_local(&pty_exit_app, &local_session_id)
            else {
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
            tauri::async_runtime::spawn(async move {
                if !app.state::<CcRuntimeState>().is_connected()
                    || !app
                        .state::<CcRuntimeState>()
                        .is_generation_current(pty_exit_generation)
                {
                    return;
                }
                let mut w = write.lock().await;
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
                        &format!("Socket read error: {e}"),
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
