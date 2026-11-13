//! Fixture-only probes. Never attach alternate thread/task dispatchers.
pub(super) fn all_levels() {
    macro_rules! probe {
        ($level:expr) => {{
            assert!(!tracing::enabled!($level));
            let span = tracing::span!(
                $level,
                "synthetic span",
                password = "SYNTHETIC_SPAN_PASSWORD"
            );
            assert!(span.is_disabled());
            let _entered = span.enter();
            span.record("password", "SYNTHETIC_RECORDED_PASSWORD");
            tracing::event!(
                $level,
                password = "SYNTHETIC_EVENT_PASSWORD",
                "synthetic event"
            );
        }};
    }
    probe!(tracing::Level::TRACE);
    probe!(tracing::Level::DEBUG);
    probe!(tracing::Level::INFO);
    probe!(tracing::Level::WARN);
    probe!(tracing::Level::ERROR);
}
