//! Startup experiment: preserve typed errors; do not define shipping EOF policy.
use super::{eof_observer, OverviewAdapter, SyntheticHost};
use rmcp::{
    service::{QuitReason, ServerInitializeError},
    ServiceExt,
};
use tokio_util::sync::CancellationToken;

pub(super) async fn run(
    adapter: OverviewAdapter<SyntheticHost>,
) -> Result<(), Box<dyn std::error::Error>> {
    eof_observer::install();
    let external = CancellationToken::new();
    let service = adapter
        .serve_with_ct(rmcp::transport::stdio_pipes(), external.clone())
        .await
        .inspect_err(|error| {
            assert!(
                matches!(error, ServerInitializeError::ConnectionClosed(context)
                if context == "initialize request")
            );
            assert!(!external.is_cancelled());
            eprintln!("fixture startup connection closed external active");
        })?;
    eprintln!("fixture running service created");
    assert!(matches!(service.waiting().await?, QuitReason::Closed));
    assert!(external.is_cancelled());
    eprintln!("fixture startup waiting closed external cancelled");
    eprintln!("fixture stopped");
    Ok(())
}
