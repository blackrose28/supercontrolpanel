use std::io::{Error as IoError, ErrorKind};
use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use serde_json::{json, Value};
use tauri::{AppHandle, Listener, Manager};
use tokio::sync::Mutex;
use tokio::time::{interval, sleep};

use super::commands::set_cc_status;
use super::protocol::Envelope;
use crate::stores::{CcRuntimeState, RemotePtyStore};

const POLL_INTERVAL_SECS: u64 = 60; // Reduced from 5s to reduce load
const HEARTBEAT_INTERVAL_SECS: u64 = 120; // 2 minutes heartbeat

pub struct SseConnection {
    server_url: String,
    agent_token: String,
    device_id: String,
    app: AppHandle,
    generation: u64,
    seq: Arc<Mutex<u64>>,
}

impl SseConnection {
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
            println!("CC SSE connection attempt #{}", attempt);
            let _ = set_cc_status(
                &self.app,
                "connecting",
                &format!(
                    "Connecting to {} as {} via polling (attempt #{}).",
                    self.server_url, self.device_id, attempt
                ),
            );

            match self.connect().await {
                Ok(_) => {
                    eprintln!("CC SSE connection ended; retrying");
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!("CC SSE connection failed: {}", err_msg);
                }
            }

            if !self.is_current_generation() {
                return Ok(());
            }

            let jitter = rand::thread_rng().gen_range(0..=(base_delay_ms / 10));
            let delay = base_delay_ms + jitter;
            let _ = set_cc_status(
                &self.app,
                "connecting",
                &format!(
                    "Disconnected from {}. Retrying in {}ms (attempt #{}).",
                    self.server_url, delay, attempt
                ),
            );

            println!("Retrying polling in {}ms...", delay);
            sleep(Duration::from_millis(delay)).await;

            base_delay_ms = (base_delay_ms * 2).min(max_delay_ms);
        }
    }

    async fn connect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Verify connection by sending a test POST
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .danger_accept_invalid_certs(true)
            .build()?;

        // Build the agent URL - handle both cases where URL ends with /agent or not
        let agent_url = if self.server_url.trim_end_matches('/').ends_with("/agent") {
            self.server_url.trim_end_matches('/').to_string()
        } else {
            format!("{}/agent", self.server_url.trim_end_matches('/'))
        };

        // First, do a test POST to verify the server is reachable
        let agent_run_id = self
            .app
            .state::<CcRuntimeState>()
            .agent_run_id()
            .to_string();
        let test_heartbeat =
            Envelope::heartbeat(self.device_id.clone(), 1, agent_run_id.clone(), 0);
        match post_to_server(&agent_url, &self.agent_token, json!(test_heartbeat)).await {
            Ok(resp) => {
                println!("CC polling connection verified: {:?}", resp);
            }
            Err(e) => {
                return Err(Box::new(IoError::new(
                    ErrorKind::ConnectionRefused,
                    format!("Failed to connect to polling endpoint: {}", e),
                )));
            }
        }

        if !self.is_current_generation() {
            return Ok(());
        }

        self.app.state::<CcRuntimeState>().set_connected(true);
        let _ = self.app.state::<RemotePtyStore>().mark_all_attached();
        let _ = set_cc_status(
            &self.app,
            "connected",
            &format!(
                "Connected as {} to {} via polling.",
                self.device_id, self.server_url
            ),
        );
        println!("CC polling connection established");

        // Start heartbeat task - sends POST with heartbeat
        let heartbeat_device_id = self.device_id.clone();
        let heartbeat_server_url = self.server_url.clone();
        let heartbeat_agent_token = self.agent_token.clone();
        let status_app = self.app.clone();
        let generation = self.generation;
        let heartbeat_seq = self.seq.clone();
        let heartbeat_agent_run_id = agent_run_id.clone();

        tokio::spawn(async move {
            let mut heartbeat_interval = interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
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
                    let mut s = heartbeat_seq.lock().await;
                    *s += 1;
                    *s
                };

                let heartbeat = Envelope::heartbeat(
                    heartbeat_device_id.clone(),
                    seq,
                    heartbeat_agent_run_id.clone(),
                    uptime_sec,
                );

                if let Err(e) = post_to_server(
                    &heartbeat_server_url,
                    &heartbeat_agent_token,
                    json!(heartbeat),
                )
                .await
                {
                    eprintln!("Polling heartbeat failed: {}", e);
                }
            }
        });

        // Start polling task for commands - sends POST and receives commands
        let _poll_device_id = self.device_id.clone();
        let poll_server_url = self.server_url.clone();
        let poll_agent_token = self.agent_token.clone();
        let poll_app = self.app.clone();
        let poll_generation = self.generation;
        let poll_seq = self.seq.clone();

        tokio::spawn(async move {
            let mut poll_interval = interval(Duration::from_secs(POLL_INTERVAL_SECS));

            loop {
                poll_interval.tick().await;

                if !poll_app.state::<CcRuntimeState>().is_connected()
                    || !poll_app
                        .state::<CcRuntimeState>()
                        .is_generation_current(poll_generation)
                {
                    break;
                }

                let _seq = {
                    let mut s = poll_seq.lock().await;
                    *s += 1;
                    *s
                };

                match poll_for_commands_get(&poll_server_url, &poll_agent_token).await {
                    Ok(commands) => {
                        if let Some(commands_array) =
                            commands.get("commands").and_then(Value::as_array)
                        {
                            if !commands_array.is_empty() {
                                println!("Polling: Received {} commands", commands_array.len());
                                for command in commands_array {
                                    println!("{}", polling_command_log_line(command));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Command polling failed: {}", e);
                    }
                }
            }
        });

        // Listen for PTY output events to forward via POST
        let pty_output_device_id = self.device_id.clone();
        let pty_output_seq = self.seq.clone();
        let pty_output_app = self.app.clone();
        let pty_output_generation = self.generation;
        let pty_output_server_url = self.server_url.clone();
        let pty_output_agent_token = self.agent_token.clone();

        let pty_output_listener = self.app.listen("pty-output", move |event| {
            let payload = event.payload();
            let Ok(payload_value) = serde_json::from_str::<Value>(payload) else {
                return;
            };

            let server_url = pty_output_server_url.clone();
            let agent_token = pty_output_agent_token.clone();
            let device_id = pty_output_device_id.clone();
            let seq_counter = pty_output_seq.clone();
            let app = pty_output_app.clone();
            let generation = pty_output_generation;

            tauri::async_runtime::spawn(async move {
                if !app.state::<CcRuntimeState>().is_connected()
                    || !app
                        .state::<CcRuntimeState>()
                        .is_generation_current(generation)
                {
                    return;
                }

                let seq = {
                    let mut s = seq_counter.lock().await;
                    *s += 1;
                    *s
                };

                let remote_session_id = payload_value
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|local_session_id| {
                        app.state::<RemotePtyStore>()
                            .remote_session_for_local(local_session_id)
                    });
                let payload_value =
                    pty_output_payload_for_polling(payload_value.clone(), remote_session_id);

                let envelope = Envelope::new(
                    "pty-output",
                    device_id.clone(),
                    seq,
                    Some(payload_value),
                );

                if let Err(e) = post_to_server(&server_url, &agent_token, json!(envelope)).await {
                    eprintln!("Polling pty-output send failed: {}", e);
                }
            });
        });

        // Listen for PTY exit events to forward via POST
        let pty_exit_device_id = self.device_id.clone();
        let pty_exit_seq = self.seq.clone();
        let pty_exit_app = self.app.clone();
        let pty_exit_generation = self.generation;
        let pty_exit_server_url = self.server_url.clone();
        let pty_exit_agent_token = self.agent_token.clone();

        let pty_exit_listener = self.app.listen("pty-exit", move |event| {
            let payload = event.payload();
            let Ok(payload_value) = serde_json::from_str::<Value>(payload) else {
                return;
            };

            let server_url = pty_exit_server_url.clone();
            let agent_token = pty_exit_agent_token.clone();
            let device_id = pty_exit_device_id.clone();
            let seq_counter = pty_exit_seq.clone();
            let app = pty_exit_app.clone();
            let generation = pty_exit_generation;

            tauri::async_runtime::spawn(async move {
                if !app.state::<CcRuntimeState>().is_connected()
                    || !app
                        .state::<CcRuntimeState>()
                        .is_generation_current(generation)
                {
                    return;
                }

                let seq = {
                    let mut s = seq_counter.lock().await;
                    *s += 1;
                    *s
                };

                let envelope = Envelope::new(
                    "pty-exit",
                    device_id.clone(),
                    seq,
                    Some(payload_value.clone()),
                );

                if let Err(e) = post_to_server(&server_url, &agent_token, json!(envelope)).await {
                    eprintln!("Polling pty-exit send failed: {}", e);
                }
            });
        });

        // Keep the connection alive - tasks continue running
        // The polling loop handles command delivery
        loop {
            sleep(Duration::from_secs(60)).await;

            if !self.is_current_generation() {
                break;
            }
        }

        self.app.unlisten(pty_output_listener);
        self.app.unlisten(pty_exit_listener);

        if !self.is_current_generation() {
            return Ok(());
        }
        self.app.state::<CcRuntimeState>().set_connected(false);
        let _ = set_cc_status(&self.app, "disconnected", "Polling connection ended.");
        Err(IoError::new(ErrorKind::UnexpectedEof, "Polling connection ended").into())
    }

    async fn next_seq(&self) -> u64 {
        let mut seq = self.seq.lock().await;
        *seq += 1;
        *seq
    }

    fn is_current_generation(&self) -> bool {
        self.app
            .state::<CcRuntimeState>()
            .is_generation_current(self.generation)
    }
}

async fn post_to_server(
    server_url: &str,
    agent_token: &str,
    payload: Value,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(true)
        .build()?;

    let url = if server_url.trim_end_matches('/').ends_with("/agent") {
        server_url.trim_end_matches('/').to_string()
    } else {
        format!("{}/agent", server_url.trim_end_matches('/'))
    };
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", agent_token))
        .json(&payload)
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        return Err(Box::new(IoError::new(
            ErrorKind::Other,
            format!("POST {} returned HTTP {}: {}", url, status, body),
        )));
    }

    Ok(serde_json::from_str(&body)?)
}

async fn poll_for_commands_get(
    server_url: &str,
    agent_token: &str,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120)) // 2 minute timeout for long-poll
        .danger_accept_invalid_certs(true)
        .build()?;

    let url = if server_url.trim_end_matches('/').ends_with("/agent") {
        server_url.trim_end_matches('/').to_string()
    } else {
        format!("{}/agent", server_url.trim_end_matches('/'))
    };
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", agent_token))
        .header("Accept", "application/json") // Expect JSON response
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        return Err(Box::new(IoError::new(
            ErrorKind::Other,
            format!("Command poll returned HTTP {}: {}", status, body),
        )));
    }

    Ok(serde_json::from_str(&body)?)
}

fn pty_output_payload_for_polling(mut payload: Value, remote_session_id: Option<String>) -> Value {
    let Some(remote_session_id) = remote_session_id else {
        return payload;
    };
    let Some(payload_object) = payload.as_object_mut() else {
        return payload;
    };

    payload_object.remove("id");
    payload_object.insert("sessionId".to_string(), json!(remote_session_id));
    payload
}

fn polling_command_log_line(command: &Value) -> String {
    let command_type = command
        .get("commandType")
        .and_then(Value::as_str)
        .unwrap_or("<missing>");
    let command_id = command
        .get("commandId")
        .and_then(Value::as_str)
        .unwrap_or("<missing>");
    let payload = command.get("payload").cloned().unwrap_or(Value::Null);
    let payload_json = serde_json::to_string(&payload).unwrap_or_else(|_| "null".to_string());
    format!(
        "Polling: Command received commandType={command_type} commandId={command_id} payload={payload_json}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_command_log_line_includes_command_type_id_and_payload() {
        let command = json!({
            "commandId": "cmd-1",
            "commandType": "request_start_server_tunnel",
            "payload": {
                "serverId": "srv-1"
            }
        });

        let line = polling_command_log_line(&command);

        assert!(line.contains("commandType=request_start_server_tunnel"));
        assert!(line.contains("commandId=cmd-1"));
        assert!(line.contains("\"serverId\":\"srv-1\""));
    }

    #[test]
    fn pty_output_payload_for_polling_uses_remote_session_id() {
        let payload = json!({
            "id": "local-ssh-session-1",
            "data": "uptime\r\n",
        });

        let rewritten = pty_output_payload_for_polling(payload, Some("server-ssh-srv-1-remote".to_string()));

        assert_eq!(rewritten["sessionId"], json!("server-ssh-srv-1-remote"));
        assert_eq!(rewritten["data"], json!("uptime\r\n"));
        assert!(rewritten.get("id").is_none());
    }

    #[test]
    fn pty_output_payload_for_polling_leaves_unmapped_local_payloads_unchanged() {
        let payload = json!({
            "id": "local-openvpn-session-1",
            "data": "openvpn output",
        });

        let rewritten = pty_output_payload_for_polling(payload.clone(), None);

        assert_eq!(rewritten, payload);
    }
}
