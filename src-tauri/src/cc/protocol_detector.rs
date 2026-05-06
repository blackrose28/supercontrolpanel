use std::time::Duration;

/// Supported connection protocols for cc-server communication
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionProtocol {
    /// WebSocket protocol (bidirectional persistent connection)
    WebSocket,
    /// Server-Sent Events (SSE) protocol with polling for outgoing messages
    Sse,
}

fn websocket_upgrade_probe_url(server_url: &str) -> String {
    let url = server_url.trim_end_matches('/').to_string();
    if url.ends_with("/agent") {
        url
    } else {
        format!("{}/agent", url)
    }
}

fn protocol_probe_url(server_url: &str) -> String {
    format!("{}/protocol", websocket_upgrade_probe_url(server_url))
}

fn protocol_probe_supports_websocket(body: &str) -> bool {
    body.contains("\"websocket\"")
}

/// Detect the supported protocol through a lightweight HTTP endpoint.
///
/// Do not perform a throwaway WebSocket upgrade here: wrangler dev can retain
/// abandoned Durable Object WebSockets until the local Node process runs out of
/// heap.
pub async fn detect_protocol(
    server_url: &str,
    agent_token: &str,
) -> Result<ConnectionProtocol, String> {
    let client = match reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(e) => return Err(format!("Failed to create HTTP client: {}", e)),
    };

    match client
        .get(protocol_probe_url(server_url))
        .header("Authorization", format!("Bearer {}", agent_token))
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            if status.is_success()
                && protocol_probe_supports_websocket(&response.text().await.unwrap_or_default())
            {
                return Ok(ConnectionProtocol::WebSocket);
            }
            println!(
                "Protocol probe returned HTTP {}; falling back to polling",
                status
            );
            Ok(ConnectionProtocol::Sse)
        }
        Err(e) => {
            println!("Protocol probe failed ({}), falling back to polling", e);
            Ok(ConnectionProtocol::Sse)
        }
    }
}

/// Check if a specific protocol is supported by the server
pub async fn is_protocol_supported(
    server_url: &str,
    agent_token: &str,
    protocol: ConnectionProtocol,
) -> Result<bool, String> {
    match protocol {
        ConnectionProtocol::WebSocket => {
            let client = match reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(Duration::from_secs(5))
                .build()
            {
                Ok(client) => client,
                Err(e) => return Err(format!("Failed to create HTTP client: {}", e)),
            };

            match client
                .get(protocol_probe_url(server_url))
                .header("Authorization", format!("Bearer {}", agent_token))
                .send()
                .await
            {
                Ok(response) => {
                    if !response.status().is_success() {
                        return Ok(false);
                    }
                    Ok(protocol_probe_supports_websocket(
                        &response.text().await.unwrap_or_default(),
                    ))
                }
                Err(_) => Ok(false),
            }
        }
        ConnectionProtocol::Sse => {
            let sse_url = if server_url.trim_end_matches('/').ends_with("/agent") {
                server_url.trim_end_matches('/').to_string()
            } else {
                format!("{}/agent", server_url.trim_end_matches('/'))
            };

            let client = match reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(Duration::from_secs(5))
                .build()
            {
                Ok(client) => client,
                Err(e) => return Err(format!("Failed to create HTTP client: {}", e)),
            };

            match client
                .get(&sse_url)
                .header("Authorization", format!("Bearer {}", agent_token))
                .header("Accept", "text/event-stream")
                .send()
                .await
            {
                Ok(response) => {
                    let status = response.status();
                    // SSE might work even if we get a 400 or 426 (WebSocket required)
                    // because the server might say WebSocket is required for this endpoint
                    // but SSE is available at the same endpoint
                    Ok(status.is_success() || status.as_u16() == 426)
                }
                Err(_) => Ok(false),
            }
        }
    }
}

/// Generate a random WebSocket key for the upgrade request
/// Returns a base64-encoded 16-byte random key
fn generate_websocket_key() -> String {
    use rand::Rng;

    let mut key = [0u8; 16];
    rand::thread_rng().fill(&mut key);

    // Simple base64 encoding
    const BASE64_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(24);
    let mut i = 0;
    while i < key.len() {
        let b1 = key[i];
        let b2 = if i + 1 < key.len() { key[i + 1] } else { 0 };
        let b3 = if i + 2 < key.len() { key[i + 2] } else { 0 };

        result.push(BASE64_CHARS.chars().nth((b1 >> 2) as usize).unwrap());
        result.push(
            BASE64_CHARS
                .chars()
                .nth((((b1 & 0x03) << 4) | (b2 >> 4)) as usize)
                .unwrap(),
        );
        result.push(
            BASE64_CHARS
                .chars()
                .nth((((b2 & 0x0F) << 2) | (b3 >> 6)) as usize)
                .unwrap(),
        );
        result.push(BASE64_CHARS.chars().nth((b3 & 0x3F) as usize).unwrap());

        i += 3;
    }

    // Remove padding if any (16 bytes should have no padding)
    result.truncate(24);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_upgrade_probe_uses_http_scheme() {
        assert_eq!(
            websocket_upgrade_probe_url("https://cc.example.test"),
            "https://cc.example.test/agent"
        );
        assert_eq!(
            websocket_upgrade_probe_url("http://localhost:8787/agent"),
            "http://localhost:8787/agent"
        );
    }

    #[test]
    fn protocol_probe_uses_http_endpoint_without_websocket_upgrade() {
        assert_eq!(
            protocol_probe_url("https://cc.example.test"),
            "https://cc.example.test/agent/protocol"
        );
        assert_eq!(
            protocol_probe_url("http://localhost:8787/agent"),
            "http://localhost:8787/agent/protocol"
        );
    }

    #[test]
    fn connection_protocol_equality() {
        assert_eq!(ConnectionProtocol::WebSocket, ConnectionProtocol::WebSocket);
        assert_eq!(ConnectionProtocol::Sse, ConnectionProtocol::Sse);
        assert_ne!(ConnectionProtocol::WebSocket, ConnectionProtocol::Sse);
    }

    #[test]
    fn websocket_key_generation() {
        let key1 = generate_websocket_key();
        let key2 = generate_websocket_key();
        assert_eq!(key1.len(), 24); // base64 of 16 bytes
        assert_ne!(key1, key2); // Should be random

        // Verify it's valid base64 characters
        for c in key1.chars() {
            assert!(c.is_alphanumeric() || c == '+' || c == '/');
        }
    }
}
