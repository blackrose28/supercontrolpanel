use rand::Rng;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, sleep, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

use super::protocol::Envelope;

pub struct CcConnection {
    server_url: String,
    agent_token: String,
    device_id: String,
    seq: Arc<Mutex<u64>>,
    connected: Arc<Mutex<bool>>,
}

impl CcConnection {
    pub fn new(server_url: String, agent_token: String, device_id: String) -> Self {
        Self {
            server_url,
            agent_token,
            device_id,
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
                    println!("CC connection established");
                    return Ok(());
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!("CC connection failed: {}", err_msg);
                    
                    // Jittered exponential backoff
                    let jitter = rand::thread_rng().gen_range(0..=(base_delay_ms / 10));
                    let delay = base_delay_ms + jitter;
                    
                    println!("Retrying in {}ms...", delay);
                    sleep(Duration::from_millis(delay)).await;
                    
                    // Exponential backoff: double delay, cap at max
                    base_delay_ms = (base_delay_ms * 2).min(max_delay_ms);
                }
            }
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

        // Send initial status_snapshot
        let seq = self.next_seq().await;
        let snapshot = Envelope::status_snapshot(self.device_id.clone(), seq);
        let msg = serde_json::to_string(&snapshot)?;
        write.send(Message::Text(msg)).await?;

        // Start heartbeat task
        let device_id = self.device_id.clone();
        let seq_counter = self.seq.clone();
        let connected = self.connected.clone();
        let write_clone = Arc::new(Mutex::new(write));
        
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
                    let mut w = write_clone.lock().await;
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
                        // TODO: handle different message types
                    }
                }
                Ok(Message::Close(_)) => {
                    println!("CC connection closed by server");
                    break;
                }
                Err(e) => {
                    eprintln!("CC read error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        *self.connected.lock().await = false;
        Ok(())
    }

    async fn next_seq(&self) -> u64 {
        let mut seq = self.seq.lock().await;
        *seq += 1;
        *seq
    }
}
