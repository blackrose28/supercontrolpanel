//! Observe one pinned SDK event; never forward SDK fields or diagnostics.
use tracing::{field::Visit, span, Event, Metadata, Subscriber};

pub(super) fn install() {
    tracing::subscriber::set_global_default(EofObserver).expect("fixture observer installed once");
}

struct EofObserver;

impl Subscriber for EofObserver {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_event()
            && metadata.target() == "rmcp::service"
            && *metadata.level() == tracing::Level::INFO
    }

    fn event(&self, event: &Event<'_>) {
        let mut message = EofMessage(false);
        event.record(&mut message);
        if message.0 {
            eprintln!("fixture sdk input eof");
        }
    }

    // Spans are disabled; these required methods retain no span state.
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

struct EofMessage(bool);

impl Visit for EofMessage {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" && format!("{value:?}") == "input stream terminated" {
            self.0 = true;
        }
    }
}
