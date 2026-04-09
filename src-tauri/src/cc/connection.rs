use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{Error as IoError, ErrorKind};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;
use tokio::time::{interval, sleep, Duration, Instant};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

use super::commands::set_cc_status;
use super::protocol::{Envelope, MessageType};
use super::telemetry::recent_log_events;
use crate::commands::{
    boundary::restart_boundary_auth_action,
    openvpn::restart_openvpn_action,
    server::{restart_server_ssh_action, restart_server_tunnel_action},
};
use crate::server_mgmt::{load_servers, ConnectionMethod, ServerOs};
use crate::stores::CcRuntimeState;

const INVENTORY_PUBLISH_INTERVAL_SECS: u64 = 5 * 60;
const INVENTORY_CHANGE_POLL_SECS: u64 = 30;

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
        let _ = set_cc_status(
            &self.app,
            "connected",
            &format!("Connected as {} to {}.", self.device_id, self.server_url),
        );
        println!("CC connection established");

        // Send initial status_snapshot
        Self::send_status_snapshot(&mut write, &self.device_id, &self.seq).await?;
        let initial_inventory_hash =
            Self::send_inventory(&mut write, &self.device_id, &self.seq, &self.app).await?;

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

                let Ok(current_hash) = Self::compute_inventory_hash(&inventory_app) else {
                    continue;
                };
                if current_hash == last_inventory_hash
                    && last_inventory_sent_at.elapsed()
                        < Duration::from_secs(INVENTORY_PUBLISH_INTERVAL_SECS)
                {
                    continue;
                }

                let mut w = inventory_write.lock().await;
                match Self::send_inventory(&mut *w, &inventory_device_id, &inventory_seq, &inventory_app)
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

        // Read messages from server
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(envelope) = serde_json::from_str::<Envelope>(&text) {
                        println!("Received from CC: type={}", envelope.msg_type);
                        if let Some(command_id) = envelope.command_id.clone() {
                            if self.is_duplicate_command(&command_id).await {
                                let mut w = write_clone.lock().await;
                                let _ = Self::send_command_status(
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
                        if let MessageType::RequestStatusSnapshot =
                            MessageType::from(envelope.msg_type.as_str())
                        {
                            if let Some(command_id) = envelope.command_id.clone() {
                                let mut w = write_clone.lock().await;
                                let _ = Self::send_command_status(
                                    &mut *w,
                                    &self.device_id,
                                    &self.seq,
                                    &command_id,
                                    "acknowledged",
                                    None,
                                    None,
                                    None,
                                )
                                .await;
                            }
                            let mut w = write_clone.lock().await;
                            if let Err(err) =
                                Self::send_status_snapshot(&mut *w, &self.device_id, &self.seq)
                                    .await
                            {
                                eprintln!("CC status_snapshot send failed: {}", err);
                                break;
                            }
                        } else if let MessageType::RequestInventory =
                            MessageType::from(envelope.msg_type.as_str())
                        {
                            if let Some(command_id) = envelope.command_id.clone() {
                                let mut w = write_clone.lock().await;
                                let _ = Self::send_command_status(
                                    &mut *w,
                                    &self.device_id,
                                    &self.seq,
                                    &command_id,
                                    "acknowledged",
                                    None,
                                    None,
                                    None,
                                )
                                .await;
                            }
                            let mut w = write_clone.lock().await;
                            if let Err(err) =
                                Self::send_inventory(&mut *w, &self.device_id, &self.seq, &self.app)
                                    .await
                            {
                                eprintln!("CC inventory send failed: {}", err);
                                break;
                            }
                        } else if let MessageType::RequestRecentLogs =
                            MessageType::from(envelope.msg_type.as_str())
                        {
                            if let Some(command_id) = envelope.command_id.clone() {
                                let mut w = write_clone.lock().await;
                                let _ = Self::send_command_status(
                                    &mut *w,
                                    &self.device_id,
                                    &self.seq,
                                    &command_id,
                                    "acknowledged",
                                    None,
                                    None,
                                    None,
                                )
                                .await;
                            }
                            let mut w = write_clone.lock().await;
                            if let Err(err) =
                                Self::send_recent_logs(&mut *w, &self.device_id, &self.seq).await
                            {
                                eprintln!("CC recent_logs send failed: {}", err);
                                break;
                            }
                        } else if let MessageType::RequestRestartOpenvpn =
                            MessageType::from(envelope.msg_type.as_str())
                        {
                            let command_id = envelope.command_id.clone();
                            let app = self.app.clone();
                            let device_id = self.device_id.clone();
                            let seq = self.seq.clone();
                            let write_for_status = write_clone.clone();
                            tokio::spawn(async move {
                                if let Some(command_id) = command_id.clone() {
                                    let mut w = write_for_status.lock().await;
                                    let _ = Self::send_command_status(
                                        &mut *w,
                                        &device_id,
                                        &seq,
                                        &command_id,
                                        "executing",
                                        Some("restart_openvpn".to_string()),
                                        None,
                                        None,
                                    )
                                    .await;
                                }
                                let result =
                                    tokio::task::spawn_blocking(move || restart_openvpn_action(app))
                                        .await;
                                if let Some(command_id) = command_id {
                                    let mut w = write_for_status.lock().await;
                                    match result {
                                        Ok(Ok(())) => {
                                            let _ = Self::send_command_status(
                                                &mut *w,
                                                &device_id,
                                                &seq,
                                                &command_id,
                                                "succeeded",
                                                Some("restart_openvpn".to_string()),
                                                None,
                                                None,
                                            )
                                            .await;
                                        }
                                        Ok(Err(err)) => {
                                            eprintln!("CC restart_openvpn failed: {}", err);
                                            let _ = Self::send_command_status(
                                                &mut *w,
                                                &device_id,
                                                &seq,
                                                &command_id,
                                                "failed",
                                                None,
                                                Some("ACTION_FAILED".to_string()),
                                                Some(err),
                                            )
                                            .await;
                                        }
                                        Err(err) => {
                                            let _ = Self::send_command_status(
                                                &mut *w,
                                                &device_id,
                                                &seq,
                                                &command_id,
                                                "failed",
                                                None,
                                                Some("JOIN_ERROR".to_string()),
                                                Some(err.to_string()),
                                            )
                                            .await;
                                        }
                                    }
                                }
                            });
                        } else if let MessageType::RequestRestartBoundaryAuth =
                            MessageType::from(envelope.msg_type.as_str())
                        {
                            let command_id = envelope.command_id.clone();
                            let app = self.app.clone();
                            let device_id = self.device_id.clone();
                            let seq = self.seq.clone();
                            let write_for_status = write_clone.clone();
                            tokio::spawn(async move {
                                if let Some(command_id) = command_id.clone() {
                                    let mut w = write_for_status.lock().await;
                                    let _ = Self::send_command_status(
                                        &mut *w,
                                        &device_id,
                                        &seq,
                                        &command_id,
                                        "executing",
                                        Some("restart_boundary_auth".to_string()),
                                        None,
                                        None,
                                    )
                                    .await;
                                }
                                let result = tokio::task::spawn_blocking(move || {
                                    restart_boundary_auth_action(app)
                                })
                                .await;
                                if let Some(command_id) = command_id {
                                    let mut w = write_for_status.lock().await;
                                    match result {
                                        Ok(Ok(())) => {
                                            let _ = Self::send_command_status(
                                                &mut *w,
                                                &device_id,
                                                &seq,
                                                &command_id,
                                                "succeeded",
                                                Some("restart_boundary_auth".to_string()),
                                                None,
                                                None,
                                            )
                                            .await;
                                        }
                                        Ok(Err(err)) => {
                                            eprintln!("CC restart_boundary_auth failed: {}", err);
                                            let _ = Self::send_command_status(
                                                &mut *w,
                                                &device_id,
                                                &seq,
                                                &command_id,
                                                "failed",
                                                None,
                                                Some("ACTION_FAILED".to_string()),
                                                Some(err),
                                            )
                                            .await;
                                        }
                                        Err(err) => {
                                            let _ = Self::send_command_status(
                                                &mut *w,
                                                &device_id,
                                                &seq,
                                                &command_id,
                                                "failed",
                                                None,
                                                Some("JOIN_ERROR".to_string()),
                                                Some(err.to_string()),
                                            )
                                            .await;
                                        }
                                    }
                                }
                            });
                        } else if let MessageType::RequestRestartServerTunnel =
                            MessageType::from(envelope.msg_type.as_str())
                        {
                            let server_id = envelope
                                .payload
                                .as_ref()
                                .and_then(|payload| payload.get("serverId"))
                                .and_then(|value| value.as_str())
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(ToOwned::to_owned);
                            let command_id = envelope.command_id.clone();
                            match server_id {
                                Some(server_id) => {
                                    let app = self.app.clone();
                                    let device_id = self.device_id.clone();
                                    let seq = self.seq.clone();
                                    let write_for_status = write_clone.clone();
                                    tokio::spawn(async move {
                                        if let Some(command_id) = command_id.clone() {
                                            let mut w = write_for_status.lock().await;
                                            let _ = Self::send_command_status(
                                                &mut *w,
                                                &device_id,
                                                &seq,
                                                &command_id,
                                                "executing",
                                                Some(format!("restart_server_tunnel:{server_id}")),
                                                None,
                                                None,
                                            )
                                            .await;
                                        }
                                        let result = tokio::task::spawn_blocking(move || {
                                            restart_server_tunnel_action(server_id.clone(), app)
                                        })
                                        .await;
                                        if let Some(command_id) = command_id {
                                            let mut w = write_for_status.lock().await;
                                            match result {
                                                Ok(Ok(())) => {
                                                    let _ = Self::send_command_status(
                                                        &mut *w,
                                                        &device_id,
                                                        &seq,
                                                        &command_id,
                                                        "succeeded",
                                                        Some("restart_server_tunnel".to_string()),
                                                        None,
                                                        None,
                                                    )
                                                    .await;
                                                }
                                                Ok(Err(err)) => {
                                                    eprintln!(
                                                        "CC restart_server_tunnel failed: {}",
                                                        err
                                                    );
                                                    let _ = Self::send_command_status(
                                                        &mut *w,
                                                        &device_id,
                                                        &seq,
                                                        &command_id,
                                                        "failed",
                                                        None,
                                                        Some("ACTION_FAILED".to_string()),
                                                        Some(err),
                                                    )
                                                    .await;
                                                }
                                                Err(err) => {
                                                    let _ = Self::send_command_status(
                                                        &mut *w,
                                                        &device_id,
                                                        &seq,
                                                        &command_id,
                                                        "failed",
                                                        None,
                                                        Some("JOIN_ERROR".to_string()),
                                                        Some(err.to_string()),
                                                    )
                                                    .await;
                                                }
                                            }
                                        }
                                    });
                                }
                                None => {
                                    eprintln!(
                                        "CC restart_server_tunnel ignored: missing payload.serverId"
                                    );
                                    if let Some(command_id) = command_id {
                                        let mut w = write_clone.lock().await;
                                        let _ = Self::send_command_status(
                                            &mut *w,
                                            &self.device_id,
                                            &self.seq,
                                            &command_id,
                                            "failed",
                                            None,
                                            Some("SERVER_ID_REQUIRED".to_string()),
                                            Some(
                                                "Missing payload.serverId for restart_server_tunnel"
                                                    .to_string(),
                                            ),
                                        )
                                        .await;
                                    }
                                }
                            }
                        } else if let MessageType::RequestRestartServerSsh =
                            MessageType::from(envelope.msg_type.as_str())
                        {
                            let server_id = envelope
                                .payload
                                .as_ref()
                                .and_then(|payload| payload.get("serverId"))
                                .and_then(|value| value.as_str())
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(ToOwned::to_owned);
                            let command_id = envelope.command_id.clone();
                            match server_id {
                                Some(server_id) => {
                                    let app = self.app.clone();
                                    let device_id = self.device_id.clone();
                                    let seq = self.seq.clone();
                                    let write_for_status = write_clone.clone();
                                    tokio::spawn(async move {
                                        if let Some(command_id) = command_id.clone() {
                                            let mut w = write_for_status.lock().await;
                                            let _ = Self::send_command_status(
                                                &mut *w,
                                                &device_id,
                                                &seq,
                                                &command_id,
                                                "executing",
                                                Some(format!("restart_server_ssh:{server_id}")),
                                                None,
                                                None,
                                            )
                                            .await;
                                        }
                                        let result = tokio::task::spawn_blocking(move || {
                                            restart_server_ssh_action(server_id.clone(), app)
                                        })
                                        .await;
                                        if let Some(command_id) = command_id {
                                            let mut w = write_for_status.lock().await;
                                            match result {
                                                Ok(Ok(())) => {
                                                    let _ = Self::send_command_status(
                                                        &mut *w,
                                                        &device_id,
                                                        &seq,
                                                        &command_id,
                                                        "succeeded",
                                                        Some("restart_server_ssh".to_string()),
                                                        None,
                                                        None,
                                                    )
                                                    .await;
                                                }
                                                Ok(Err(err)) => {
                                                    eprintln!("CC restart_server_ssh failed: {}", err);
                                                    let _ = Self::send_command_status(
                                                        &mut *w,
                                                        &device_id,
                                                        &seq,
                                                        &command_id,
                                                        "failed",
                                                        None,
                                                        Some("ACTION_FAILED".to_string()),
                                                        Some(err),
                                                    )
                                                    .await;
                                                }
                                                Err(err) => {
                                                    let _ = Self::send_command_status(
                                                        &mut *w,
                                                        &device_id,
                                                        &seq,
                                                        &command_id,
                                                        "failed",
                                                        None,
                                                        Some("JOIN_ERROR".to_string()),
                                                        Some(err.to_string()),
                                                    )
                                                    .await;
                                                }
                                            }
                                        }
                                    });
                                }
                                None => {
                                    eprintln!(
                                        "CC restart_server_ssh ignored: missing payload.serverId"
                                    );
                                    if let Some(command_id) = command_id {
                                        let mut w = write_clone.lock().await;
                                        let _ = Self::send_command_status(
                                            &mut *w,
                                            &self.device_id,
                                            &self.seq,
                                            &command_id,
                                            "failed",
                                            None,
                                            Some("SERVER_ID_REQUIRED".to_string()),
                                            Some(
                                                "Missing payload.serverId for restart_server_ssh"
                                                    .to_string(),
                                            ),
                                        )
                                        .await;
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) => {
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

    async fn send_status_snapshot(
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

    async fn send_inventory(
        write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
        device_id: &str,
        seq_counter: &Arc<Mutex<u64>>,
        app: &AppHandle,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let (payload, hash) = Self::build_inventory_payload(app)?;
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

    fn build_inventory_payload(
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

    fn compute_inventory_hash(
        app: &AppHandle,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Self::build_inventory_payload(app).map(|(_, hash)| hash)
    }

    async fn send_recent_logs(
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

    async fn send_command_status(
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
        let status =
            Envelope::command_status(device_id.to_string(), seq, command_id.to_string(), payload);
        let msg = serde_json::to_string(&status)?;
        write.send(Message::Text(msg)).await?;
        Ok(())
    }
}
