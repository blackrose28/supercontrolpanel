//! Observe fixed pinned SDK events; never forward SDK fields or diagnostics.
use tracing::{field::Visit, span, Event, Metadata, Subscriber};

pub(super) fn install() {
    tracing::subscriber::set_global_default(EofObserver).expect("fixture observer installed once");
}

struct EofObserver;

impl Subscriber for EofObserver {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_event()
            && metadata.target() == "rmcp::service"
            && matches!(
                *metadata.level(),
                tracing::Level::INFO | tracing::Level::WARN
            )
    }

    fn event(&self, event: &Event<'_>) {
        let mut message = EofMessage(None);
        event.record(&mut message);
        if let Some(marker) = message.0 {
            eprintln!("{marker}");
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

struct EofMessage(Option<&'static str>);

impl Visit for EofMessage {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = match format!("{value:?}").as_str() {
                "input stream terminated" => Some("fixture sdk input eof"),
                "timed out draining in-flight responses" => Some("fixture sdk drain expired"),
                _ => None,
            };
        }
    }
}
