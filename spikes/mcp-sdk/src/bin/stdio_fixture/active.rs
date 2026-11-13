//! Active synthetic service, including the controlled EOF-expiry experiment.
use super::{OverviewAdapter, SyntheticHost};
use rmcp::ServiceExt;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub(super) async fn run(
    adapter: OverviewAdapter<SyntheticHost>,
    completion: Option<Arc<Notify>>,
) -> Result<(), Box<dyn std::error::Error>> {
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
