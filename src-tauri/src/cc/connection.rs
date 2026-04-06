use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Error as IoError, ErrorKind};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, sleep, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use tauri::AppHandle;

use super::protocol::{Envelope, MessageType};
use crate::server_mgmt::{load_servers, ConnectionMethod, ServerOs};

pub struct CcConnection {
    server_url: String,
    agent_token: String,
    device_id: String,
    app: AppHandle,
    seq: Arc<Mutex<u64>>,
    connected: Arc<Mutex<bool>>,
}

impl CcConnection {
    pub fn new(server_url: String, agent_token: String, device_id: String, app: AppHandle) -> Self {
        Self {
            server_url,
            agent_token,
            device_id,
            app,
            seq: Arc::new(Mutex::new(0)),
            connected: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn connect_with_retry(&self) -> Result<(), String> {
        let mut base_delay_ms = 500u64;
        let max_delay_ms = 30000u64;
        let mut attempt = 0;

        loop {
            attempt += 1;
            println!("CC connection attempt #{}", attempt);

            match self.connect().await {
                Ok(_) => {
                    eprintln!("CC connection ended; retrying");
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!("CC connection failed: {}", err_msg);
                }
            }

            // Jittered exponential backoff
            let jitter = rand::thread_rng().gen_range(0..=(base_delay_ms / 10));
            let delay = base_delay_ms + jitter;

            println!("Retrying in {}ms...", delay);
            sleep(Duration::from_millis(delay)).await;

            // Exponential backoff: double delay, cap at max
            base_delay_ms = (base_delay_ms * 2).min(max_delay_ms);
        }
    }

    async fn connect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = self.server_url.replace("http://", "ws://").replace("https://", "wss://");
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

        *self.connected.lock().await = true;
        println!("CC connection established");

        // Send initial status_snapshot
        Self::send_status_snapshot(&mut write, &self.device_id, &self.seq).await?;
        Self::send_inventory(&mut write, &self.device_id, &self.seq, &self.app).await?;

        // Start heartbeat task
        let device_id = self.device_id.clone();
        let seq_counter = self.seq.clone();
        let connected = self.connected.clone();
        let write_clone = Arc::new(Mutex::new(write));
        let heartbeat_write = write_clone.clone();
        
        tokio::spawn(async move {
            let mut heartbeat_interval = interval(Duration::from_secs(15));
            let start_time = std::time::Instant::now();

            loop {
                heartbeat_interval.tick().await;
                
                if !*connected.lock().await {
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

        // Read messages from server
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(envelope) = serde_json::from_str::<Envelope>(&text) {
                        println!("Received from CC: type={}", envelope.msg_type);
                        if let MessageType::RequestStatusSnapshot =
                            MessageType::from(envelope.msg_type.as_str())
                        {
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
                            let mut w = write_clone.lock().await;
                            if let Err(err) =
                                Self::send_inventory(&mut *w, &self.device_id, &self.seq, &self.app)
                                    .await
                            {
                                eprintln!("CC inventory send failed: {}", err);
                                break;
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    println!("CC connection closed by server");
                    *self.connected.lock().await = false;
                    return Err(IoError::new(
                        ErrorKind::ConnectionAborted,
                        "CC connection closed by server",
                    )
                    .into());
                }
                Err(e) => {
                    eprintln!("CC read error: {}", e);
                    *self.connected.lock().await = false;
                    return Err(IoError::new(
                        ErrorKind::ConnectionAborted,
                        format!("CC read error: {e}"),
                    )
                    .into());
                }
                _ => {}
            }
        }

        *self.connected.lock().await = false;
        Err(IoError::new(
            ErrorKind::UnexpectedEof,
            "CC connection stream ended",
        )
        .into())
    }

    async fn next_seq(&self) -> u64 {
        let mut seq = self.seq.lock().await;
        *seq += 1;
        *seq
    }

    async fn send_status_snapshot(
        write: &mut (
            impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin
        ),
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
        write: &mut (
            impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin
        ),
        device_id: &str,
        seq_counter: &Arc<Mutex<u64>>,
        app: &AppHandle,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

        let seq = {
            let mut s = seq_counter.lock().await;
            *s += 1;
            *s
        };
        let inventory = Envelope::inventory(device_id.to_string(), seq, payload);
        let msg = serde_json::to_string(&inventory)?;
        write.send(Message::Text(msg)).await?;
        Ok(())
    }
}
