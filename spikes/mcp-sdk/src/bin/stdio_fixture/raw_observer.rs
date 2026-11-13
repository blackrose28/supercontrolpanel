//! TEST ONLY: deliberately retain unsanitized pinned SDK DEBUG fields.
//! Synthetic bounded probes only; this is no production logging policy.
use tracing::{field::Visit, span, Event, Metadata, Subscriber};

pub(super) fn install() {
    tracing::subscriber::set_global_default(RawObserver).expect("fixture observer installed once");
}

struct RawObserver;

impl Subscriber for RawObserver {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_event()
            && *metadata.level() == tracing::Level::DEBUG
            && matches!(
                metadata.target(),
                "rmcp::transport::async_rw" | "rmcp::service"
            )
    }

    fn event(&self, event: &Event<'_>) {
        let mut fields = RawFields(serde_json::Map::new());
        event.record(&mut fields);
        // stderr only. Parent drains concurrently and enforces a total byte budget.
        eprintln!(
            "fixture raw {}",
            serde_json::json!({
                "target":event.metadata().target(),
                "level":event.metadata().level().as_str(),
                "fields":fields.0,
            })
        );
    }

    // Spans are disabled: no retained state, worker thread, stdin or stdout access.
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

struct RawFields(serde_json::Map<String, serde_json::Value>);

impl Visit for RawFields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().into(), format!("{value:?}").into());
    }
}
