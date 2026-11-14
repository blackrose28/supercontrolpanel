//! Domain event sink for headless runtime state transitions and process output.
use crate::models::{FunctionStatus, ScpTransferEntry};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Event emitted by the core runtime without coupling to Tauri or MCP transports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DomainEvent {
    FunctionStatusChanged {
        status: FunctionStatus,
    },
    ProcessOutput {
        id: String,
        data: String,
        is_pty: bool,
    },
    ProcessExited {
        id: String,
        exit_code: Option<i32>,
        reason: String,
    },
    TransferUpdated {
        entry: ScpTransferEntry,
    },
    RemoteBrowserOpened {
        session_id: String,
        server_id: String,
    },
    RemoteBrowserClosed {
        session_id: String,
        server_id: String,
        reason: String,
    },
}

/// Thread-safe callback invoked whenever the runtime emits a `DomainEvent`.
pub type EventObserver = Arc<dyn Fn(&DomainEvent) + Send + Sync>;

/// Sink notified whenever runtime functions, processes, transfers, or browsers change.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: DomainEvent);
}

/// Default no-op sink used when no external subscriber is attached.
#[derive(Debug, Default)]
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _event: DomainEvent) {}
}

pub(crate) struct FanoutEventSink {
    primary: Arc<dyn EventSink>,
    observers: Mutex<Vec<EventObserver>>,
}

impl FanoutEventSink {
    pub(crate) fn wrap(primary: Arc<dyn EventSink>) -> Arc<Self> {
        Arc::new(Self {
            primary,
            observers: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn add_observer(&self, observer: EventObserver) {
        self.observers.lock().push(observer);
    }
}

impl EventSink for FanoutEventSink {
    fn emit(&self, event: DomainEvent) {
        self.primary.emit(event.clone());
        let snapshot: Vec<EventObserver> = self.observers.lock().clone();
        for observer in snapshot {
            observer(&event);
        }
    }
}

/// Thread-safe event buffer used in tests and short-lived CLI commands.
#[derive(Debug, Default)]
pub struct BufferedEventSink {
    events: Mutex<Vec<DomainEvent>>,
}

impl BufferedEventSink {
    /// Creates an empty buffered sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a cloned snapshot of all recorded events in emission order.
    pub fn snapshot(&self) -> Vec<DomainEvent> {
        self.events.lock().clone()
    }

    /// Drains and returns all recorded events in emission order.
    pub fn drain(&self) -> Vec<DomainEvent> {
        std::mem::take(&mut *self.events.lock())
    }
}

impl EventSink for BufferedEventSink {
    fn emit(&self, event: DomainEvent) {
        self.events.lock().push(event);
    }
}
