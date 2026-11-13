//! Synthetic subprocess only; never reads real configuration or credentials.
//! Stdout belongs exclusively to the SDK. No production scpctl entry point.
#[path = "stdio_fixture/eof_observer.rs"]
mod eof_observer;

use rmcp::ServiceExt;
use scp_core::mcp_dto::McpAppOverviewResult;
use scp_mcp_sdk_spike::overview::{OverviewAdapter, OverviewHost, OverviewPolicy};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::Notify,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct SyntheticHost {
    fail: bool,
    gate: Option<SocketAddr>,
    completion: Option<Arc<Notify>>,
}

impl OverviewHost for SyntheticHost {
    async fn read_overview(
        &self,
        cancellation: CancellationToken,
    ) -> Result<McpAppOverviewResult, String> {
        eprintln!("fixture host read");
        if let Some(address) = self.gate {
            let outcome = gated_read(address, cancellation).await;
            if let Some(completion) = &self.completion {
                // Test-host acknowledgment of gate work, not an SDK handler join.
                completion.notify_one();
            }
            outcome.map_err(|_| "fixture gate failed")?;
        }
        if self.fail {
            return Err("password=TOP_SECRET native /private/path".into());
        }
        Ok(McpAppOverviewResult {
            app_version: "stdio-fake".into(),
            active_functions: 2,
            known_function_count: 5,
        })
    }
}

/// Test-only side channel: no progress/tool additions or reads from MCP stdin.
async fn gated_read(address: SocketAddr, cancellation: CancellationToken) -> std::io::Result<()> {
    assert!(address.ip().is_loopback());
    let mut stream = TcpStream::connect(address).await?;
    stream.write_all(b"scp-stdio-gate-v1\nentered\n").await?;
    let mut control = BufReader::new(stream);
    loop {
        let mut command = String::new();
        let mut limited = (&mut control).take(128);
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                control.get_mut().write_all(b"cancelled\nfinished\n").await?;
                return Err(std::io::Error::other("fixture cancelled"));
            }
            read = limited.read_line(&mut command) => {
                read?;
                match command.as_str() {
                    "check\n" => control.get_mut().write_all(b"active\n").await?,
                    "release\n" => {
                        control.get_mut().write_all(b"released\nfinished\n").await?;
                        return Ok(());
                    }
                    _ => return Err(std::io::Error::other("invalid fixture control")),
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    if matches!(mode.as_str(), "gated-eof" | "gated-eof-expiry") {
        eof_observer::install();
    }
    let allowed_categories = match mode.as_str() {
        "denied" => vec![],
        "bad-category" => vec!["read-only".into(), "SECRET_CATEGORY".into()],
        _ => vec!["read-only".into()],
    };
    let sensitive_action_policy = if mode == "bad-policy" {
        "SECRET_POLICY"
    } else {
        "require-ui-approval"
    };
    let completion = (mode == "gated-eof-expiry").then(|| Arc::new(Notify::new()));
    let adapter = OverviewAdapter::new(
        SyntheticHost {
            fail: mode == "error",
            completion: completion.clone(),
            gate: if matches!(mode.as_str(), "gated" | "gated-eof" | "gated-eof-expiry") {
                Some(
                    std::env::args()
                        .nth(2)
                        .ok_or("missing fixture address")?
                        .parse()?,
                )
            } else {
                None
            },
        },
        OverviewPolicy {
            allowed_categories,
            sensitive_action_policy: sensitive_action_policy.into(),
        },
    );
    eprintln!("fixture started");
    let external = CancellationToken::new();
    let service = adapter
        .serve_with_ct(rmcp::transport::stdio_pipes(), external.clone())
        .await?;
    if let Some(completion) = completion {
        // Retain RunningService: waiting() consumes it and drops its external-token guard.
        // This test host waits for synthetic cooperation, not a native/handler join.
        completion.notified().await;
        assert!(!external.is_cancelled());
        eprintln!("fixture host completion observed external active");
        assert!(matches!(
            service.waiting().await?,
            rmcp::service::QuitReason::Closed
        ));
        assert!(external.is_cancelled());
        eprintln!("fixture waiting closed external cancelled");
    } else {
        service.waiting().await?;
    }
    eprintln!("fixture stopped");
    Ok(())
}
