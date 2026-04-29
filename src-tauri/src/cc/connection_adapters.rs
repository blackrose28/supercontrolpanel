// Agent connection adapter - supports both WebSocket and polling modes
// Detects server capability and uses appropriate protocol

use reqwest::Client;
use serde_json::{json, Value};

pub struct AgentConnection {
    server_url: String,
    agent_token: String,
    polling_interval_ms: u64,
    client: Client,
}

impl AgentConnection {
    pub fn new(server_url: &str, agent_token: &str, polling_interval_ms: u64) -> Self {
        Self {
            server_url: server_url.to_string(),
            agent_token: agent_token.to_string(),
            polling_interval_ms,
            client: Client::new(),
        }
    }

    pub async fn connect(&self) -> Result<(), String> {
        // First check if server supports polling (D1 mode) or WebSocket (NestJS mode)
        let supports_polling = self.check_polling_support().await;
        
        if supports_polling {
            self.connect_polling().await?;
        } else {
            // Fall back to WebSocket
            self.connect_websocket().await?;
        }
        
        Ok(())
    }

    async fn check_polling_support(&self) -> bool {
        // Try polling to see if server supports it
        let test_url = format!("{}/agent", self.server_url);
        
        match self.client
            .post(&test_url)
            .header("Authorization", format!("Bearer {}", self.agent_token))
            .header("Content-Type", "application/json")
            .json(&json!({"heartbeat": true}))
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    async fn connect_polling(&self) -> Result<(), String> {
        // Poll for heartbeat and commands
        // This is a simplified version - in production would need to:
        // - Process any commands returned from the server
        // - Send ack/nack responses
        // - Handle reconnection
        
        loop {
            let url = format!("{}/agent", self.server_url);
            
            match self.client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.agent_token))
                .header("Content-Type", "application/json")
                .json(&json!({"heartbeat": true}))
                .send()
                .await
            {
                Ok(resp) => {
                    if resp.status().is_success() {
                        let body: Value = resp.json().await.map_err(|e| e.to_string())?;
                        // Process commands from server
                        // ...
                    }
                }
                Err(e) => {
                    eprintln!("Polling error: {}", e);
                }
            }
            
            tokio::time::sleep(std::time::Duration::from_millis(self.polling_interval_ms)).await;
        }
    }

    async fn connect_websocket(&self) -> Result<(), String> {
        // Original WebSocket implementation
        // ...
        Ok(())
    }
}
