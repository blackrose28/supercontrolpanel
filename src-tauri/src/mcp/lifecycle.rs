//! Transport work is bounded independently of SDK sessions and listener generations.
use super::contracts::{ErrorCode, OperationError};
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{watch, Semaphore};
pub(super) use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(super) struct TransportControl {
    pub stop: CancellationToken,
    pub updates: watch::Sender<u64>,
    workers: Arc<Semaphore>,
    pub streams: Arc<Semaphore>,
}
impl Default for TransportControl {
    fn default() -> Self {
        static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
        Self {
            stop: CancellationToken::new(),
            updates: watch::channel(0).0,
            workers: WORKERS.get_or_init(|| Arc::new(Semaphore::new(16))).clone(),
            streams: Arc::new(Semaphore::new(8)),
        }
    }
}
impl TransportControl {
    pub fn invalidate(&self) {
        self.updates
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    pub async fn run<T: Send + 'static>(
        &self,
        cancel: CancellationToken,
        work: impl FnOnce() -> Result<T, OperationError> + Send + 'static,
    ) -> Result<T, OperationError> {
        // Dropping the request cancels work still waiting in Tokio's blocking pool.
        // A started closure owns the permit until it finishes, even after timeout.
        let cancel = cancel.child_token();
        let _pending = cancel.clone().drop_guard();
        if cancel.is_cancelled() || self.stop.is_cancelled() {
            return Err(OperationError::new(ErrorCode::Cancelled));
        }
        let permit = self
            .workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| OperationError::new(ErrorCode::Conflict))?;
        let stop = self.stop.clone();
        let work = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if cancel.is_cancelled() || stop.is_cancelled() {
                return Err(OperationError::new(ErrorCode::Cancelled));
            }
            work()
        });
        work.await
            .map_err(|_| OperationError::new(ErrorCode::OperationFailed))?
    }
}

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
