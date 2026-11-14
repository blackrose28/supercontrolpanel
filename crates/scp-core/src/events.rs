//! Domain event sink for headless runtime state transitions and process output.
use crate::models::{FunctionStatus, ScpTransferEntry};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

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
    RemoteBrowserClosed {
        session_id: String,
        server_id: String,
        reason: String,
    },
}

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
