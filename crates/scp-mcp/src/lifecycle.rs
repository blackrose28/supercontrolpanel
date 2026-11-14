//! Shared bounded worker transport control used by HTTP and stdio hosts.
use crate::contracts::{ErrorCode, OperationError};
use scp_core::supervisor::CancelToken;
use std::cell::RefCell;
use std::sync::{Arc, OnceLock};
use tokio::sync::{watch, Semaphore};
pub use tokio_util::sync::CancellationToken;

const DEFAULT_MAX_WORKERS: usize = 16;
const DEFAULT_MAX_SUBSCRIPTIONS: usize = 8;
const MAX_LIMIT: usize = 64;

thread_local! {
    static CURRENT_CANCEL_TOKEN: RefCell<Option<CancelToken>> = const { RefCell::new(None) };
}

/// Returns the core cancellation token bound to the currently executing transport worker.
pub fn current_cancel_token() -> CancelToken {
    CURRENT_CANCEL_TOKEN.with(|slot| slot.borrow().clone().unwrap_or_default())
}

fn env_bounded_limit(names: &[&str], default: usize) -> usize {
    for name in names {
        if let Ok(raw) = std::env::var(name) {
            if let Ok(parsed) = raw.trim().parse::<usize>() {
                return parsed.clamp(1, MAX_LIMIT);
            }
        }
    }
    default
}

struct WorkerCancelGuard {
    done: CancellationToken,
    core: CancelToken,
    finished: bool,
}

impl Drop for WorkerCancelGuard {
    fn drop(&mut self) {
        self.done.cancel();
        if !self.finished {
            self.core.cancel();
        }
    }
}

struct CurrentCancelReset;

impl Drop for CurrentCancelReset {
    fn drop(&mut self) {
        CURRENT_CANCEL_TOKEN.with(|slot| *slot.borrow_mut() = None);
    }
}

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
    /// Constructs standalone stdio transport controls sized from environment variables.
    pub fn standalone_from_env() -> Self {
        let max_workers = env_bounded_limit(
            &["SCP_STDIO_MAX_WORKERS", "SCP_STDIO_MAX_CONCURRENT"],
            DEFAULT_MAX_WORKERS,
        );
        let max_streams =
            env_bounded_limit(&["SCP_STDIO_MAX_SUBSCRIPTIONS"], DEFAULT_MAX_SUBSCRIPTIONS);
        Self {
            stop: CancellationToken::new(),
            updates: watch::channel(0).0,
            workers: Arc::new(Semaphore::new(max_workers)),
            streams: Arc::new(Semaphore::new(max_streams)),
        }
    }

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
        let core_cancel = CancelToken::new();
        let done = CancellationToken::new();
        let mut guard = WorkerCancelGuard {
            done: done.clone(),
            core: core_cancel.clone(),
            finished: false,
        };
        let watcher_cancel = cancel.clone();
        let watcher_stop = stop.clone();
        let watcher_core = core_cancel.clone();
        let watcher_done = done.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = watcher_cancel.cancelled() => watcher_core.cancel(),
                _ = watcher_stop.cancelled() => watcher_core.cancel(),
                _ = watcher_done.cancelled() => {}
            }
        });
        let core_for_worker = core_cancel.clone();
        let work = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if cancel.is_cancelled() || stop.is_cancelled() || core_for_worker.is_cancelled() {
                return Err(OperationError::new(ErrorCode::Cancelled));
            }
            CURRENT_CANCEL_TOKEN.with(|slot| *slot.borrow_mut() = Some(core_for_worker));
            let _reset = CurrentCancelReset;
            work()
        });
        let result = work
            .await
            .map_err(|_| OperationError::new(ErrorCode::OperationFailed))?;
        guard.finished = true;
        result
    }
}
