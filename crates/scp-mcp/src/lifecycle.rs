//! Shared bounded worker transport control used by HTTP and stdio hosts.
use crate::contracts::{ErrorCode, OperationError};
use std::sync::{Arc, OnceLock};
use tokio::sync::{watch, Semaphore};
pub use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct TransportControl {
    pub stop: CancellationToken,
    pub updates: watch::Sender<u64>,
    pub workers: Arc<Semaphore>,
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
