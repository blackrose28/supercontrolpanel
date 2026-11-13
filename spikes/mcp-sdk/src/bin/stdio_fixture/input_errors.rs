//! Characterize raw SDK errors; deliberately no diagnostic sanitizer/subscriber.
use super::{OverviewAdapter, SyntheticHost};
use rmcp::{
    model::ClientJsonRpcMessage,
    service::{QuitReason, ServerInitializeError},
    ServiceExt,
};
use tokio_util::sync::CancellationToken;

pub(super) async fn run(
    adapter: OverviewAdapter<SyntheticHost>,
) -> Result<(), Box<dyn std::error::Error>> {
    let external = CancellationToken::new();
    let service = adapter
        .serve_with_ct(rmcp::transport::stdio_pipes(), external.clone())
        .await
        .inspect_err(|error| {
            let ServerInitializeError::ExpectedInitializeRequest(Some(
                ClientJsonRpcMessage::Error(message),
            )) = error
            else {
                panic!("unexpected fixture startup error variant");
            };
            assert!(message.id.is_none());
            assert_eq!(message.error.code.0, -32600);
            assert_eq!(message.error.message, "SYNTHETIC_STDIO_SECRET");
            assert_eq!(
                message.error.data,
                Some(serde_json::json!({
                    "password":"SYNTHETIC_STDIO_SECRET"
                }))
            );
            assert!(!external.is_cancelled());
            eprintln!("fixture unexpected peer error external active");
        })?; // Preserve the original typed error and Rust's raw Termination diagnostic.
    eprintln!("fixture input service created");
    assert!(matches!(service.waiting().await?, QuitReason::Closed));
    assert!(external.is_cancelled());
    eprintln!("fixture input waiting closed external cancelled");
    eprintln!("fixture stopped");
    Ok(())
}
