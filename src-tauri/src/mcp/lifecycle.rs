//! Transport work is bounded independently of SDK sessions and listener generations.
#[cfg(test)]
use super::contracts::{ErrorCode, OperationError};
pub(super) use scp_mcp::lifecycle::{CancellationToken, TransportControl};
#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;

/// Cancels SDK sessions first, then bounds the HTTP drain. App stores are never touched.
pub(super) async fn serve_transport(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    control: TransportControl,
) {
    let stop = control.stop.clone();
    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        stop.cancelled().await;
    });
    let mut server = tokio::spawn(async move {
        let _ = server.await;
    });
    tokio::select! {
        _ = &mut server => { control.stop.cancel(); },
        _ = control.stop.cancelled() => {
            if tokio::time::timeout(Duration::from_secs(2), &mut server).await.is_err() {
                server.abort();
                let _ = server.await;
            }
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
