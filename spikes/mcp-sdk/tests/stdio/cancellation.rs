//! Separate loopback controls carry only synthetic gate state, never MCP stdin.
use super::*;
use rmcp::{
    model::{CallToolRequest, ClientRequest},
    service::PeerRequestOptions,
};
use tokio::net::{TcpListener, TcpStream};

pub(super) struct Gate(BufReader<TcpStream>);

impl Gate {
    pub(super) async fn entered(listener: &TcpListener) -> Self {
        let mut gate = bounded(async {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let mut preface = vec![0; b"scp-stdio-gate-v1\n".len()];
                // Managed devboxes can probe newly opened ports with HTTP/TLS.
                if reader.read_exact(&mut preface).await.is_ok()
                    && preface == b"scp-stdio-gate-v1\n"
                {
                    break Self(reader);
                }
            }
        })
        .await;
        gate.expect("entered").await;
        gate
    }

    async fn expect(&mut self, expected: &str) {
        let mut line = String::new();
        bounded((&mut self.0).take(128).read_line(&mut line))
            .await
            .unwrap();
        assert_eq!(line, format!("{expected}\n"));
    }

    pub(super) async fn send(&mut self, command: &str) {
        bounded(
            self.0
                .get_mut()
                .write_all(format!("{command}\n").as_bytes()),
        )
        .await
        .unwrap();
    }

    pub(super) async fn active(&mut self) {
        self.send("check").await;
        self.expect("active").await;
    }

    pub(super) async fn finished(&mut self, outcome: &str) {
        self.expect(outcome).await;
        self.expect("finished").await;
        let mut byte = [0];
        assert_eq!(bounded(self.0.read(&mut byte)).await.unwrap(), 0);
    }
}

/// The official SDK skips non-JSON lines. Validate and forward every stdout line.
async fn checked_stdout(output: ChildStdout, mut writer: tokio::io::DuplexStream) -> Vec<Value> {
    let mut output = BufReader::new(output);
    let mut responses = Vec::new();
    while let Some((bytes, value)) = read_response_line(&mut output).await {
        assert!(responses.len() < 16, "unexpected fixture output volume");
        responses.push(value);
        writer.write_all(&bytes).await.unwrap();
    }
    responses
}

#[tokio::test]
async fn official_stdio_cancel_reaches_host_and_preserves_concurrent_read() {
    for version in [ProtocolVersion::V_2025_11_25, ProtocolVersion::V_2026_07_28] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_stdio-fixture"))
            .arg("gated")
            .arg(listener.local_addr().unwrap().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let (reader, writer) = tokio::io::duplex(65536);
        let stdout = tokio::spawn(checked_stdout(child.stdout.take().unwrap(), writer));
        let lifecycle = if version.as_str() == LEGACY {
            ClientLifecycleMode::Initialize
        } else {
            ClientLifecycleMode::Discover {
                preferred_versions: vec![version.clone()],
            }
        };
        let client = bounded(
            ClientConfig::default()
                .with_protocol_version(version.clone())
                .serve_with_lifecycle((reader, input), lifecycle),
        )
        .await
        .unwrap();
        let request = || {
            ClientRequest::CallToolRequest(CallToolRequest::new(CallToolRequestParams::new(
                "app_get_overview",
            )))
        };
        let first =
            bounded(client.send_request_with_option(request(), PeerRequestOptions::no_options()))
                .await
                .unwrap();
        let mut cancelled = Gate::entered(&listener).await;
        let other =
            bounded(client.send_request_with_option(request(), PeerRequestOptions::no_options()))
                .await
                .unwrap();
        let mut survivor = Gate::entered(&listener).await;
        cancelled.active().await;
        survivor.active().await;
        let cancelled_id = serde_json::to_value(&first.id).unwrap();
        let survivor_id = serde_json::to_value(&other.id).unwrap();
        // Explicit official API; entry evidence replaces HTTP progress/stream priming.
        bounded(first.cancel(Some("fixture cancellation".into())))
            .await
            .unwrap();
        cancelled.finished("cancelled").await;
        survivor.active().await;
        survivor.send("release").await;
        survivor.finished("released").await;
        let result = serde_json::to_value(bounded(other.await_response()).await.unwrap()).unwrap();
        check_result(&result, version.as_str(), false);
        bounded(client.cancel()).await.unwrap();
        finish(child, 2).await;
        let responses = bounded(stdout).await.unwrap();
        assert_eq!(responses.len(), 2, "handshake and survivor responses only");
        assert!(!responses
            .iter()
            .any(|response| response["id"] == cancelled_id));
        let survivor = responses
            .iter()
            .find(|response| response["id"] == survivor_id)
            .unwrap();
        check_result(&survivor["result"], version.as_str(), false);
    }
}
