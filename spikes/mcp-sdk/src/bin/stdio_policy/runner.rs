use rmcp::{
    service::{QuitReason, ServerInitializeError},
    ServiceExt,
};
use scp_core::mcp_dto::McpAppOverviewResult;
use scp_mcp_sdk_spike::overview::{OverviewAdapter, OverviewHost, OverviewPolicy};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

pub(super) enum Failure {
    Setup,
    Startup(Box<ServerInitializeError>),
    Service,
}

pub(super) async fn serve<R, W>(input: R, output: W) -> Result<(), Failure>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let adapter = OverviewAdapter::new(
        UnreachableHost,
        OverviewPolicy {
            allowed_categories: vec!["read-only".into()],
            sensitive_action_policy: "require-ui-approval".into(),
        },
    );
    let service = adapter
        .serve((input, output))
        .await
        .map_err(|error| Failure::Startup(Box::new(error)))?;
    // This synthetic host owns no native work. Closed is not native cleanup proof.
    match service.waiting().await {
        Ok(QuitReason::Closed) => Ok(()),
        // No host-requested cancellation exists in this focused fixture.
        _ => Err(Failure::Service),
    }
}

#[derive(Clone)]
struct UnreachableHost;

impl OverviewHost for UnreachableHost {
    async fn read_overview(&self, _: CancellationToken) -> Result<McpAppOverviewResult, String> {
        // Fixture tripwire: none of these probes may enter a host operation.
        panic!("policy fixture unexpectedly entered host");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ClientJsonRpcMessage;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn policy_runner_retains_original_typed_startup_peer_error() {
        let (mut peer, input) = tokio::io::duplex(4096);
        peer.write_all(concat!("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,",
            "\"message\":\"SYNTHETIC_STDIO_SECRET\",\"data\":{\"password\":\"SYNTHETIC_STDIO_SECRET\"}}}\n")
            .as_bytes()).await.unwrap();
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            serve(input, tokio::io::sink()),
        )
        .await
        .expect("typed startup fixture watchdog");
        // No Debug, Display, source traversal or error-to-string conversion.
        let Err(Failure::Startup(error)) = outcome else {
            panic!("typed startup failure required");
        };
        let ServerInitializeError::ExpectedInitializeRequest(Some(ClientJsonRpcMessage::Error(
            message,
        ))) = *error
        else {
            panic!("original typed startup peer error required");
        };
        assert!(message.id.is_none());
        assert_eq!(message.error.code.0, -32600);
        assert_eq!(message.error.message, "SYNTHETIC_STDIO_SECRET");
        assert_eq!(
            message.error.data,
            Some(serde_json::json!({"password":"SYNTHETIC_STDIO_SECRET"}))
        );
    }
}
