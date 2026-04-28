pub mod commands;
pub mod connection;
pub mod dispatcher;
pub mod protocol;
pub mod telemetry;
pub mod telemetry_queue;
pub mod telemetry_queue_crypto;
pub mod telemetry_queue_disk;

pub(crate) use commands::sync_cc_status_from_settings;
pub use commands::{get_cc_status, init_cc_connection};
pub use connection::CcConnection;
pub use dispatcher::CcDispatcher;
pub use protocol::{Envelope, MessageType};

#[cfg(test)]
mod tests {
    use super::{CcDispatcher, Envelope, MessageType};

    #[test]
    fn public_exports_resolve_to_existing_modules() {
        let dispatcher_type = std::any::type_name::<CcDispatcher>();
        let envelope_type = std::any::type_name::<Envelope>();
        let message_type = std::any::type_name::<MessageType>();

        assert!(dispatcher_type.ends_with("cc::dispatcher::CcDispatcher"));
        assert!(envelope_type.ends_with("cc::protocol::Envelope"));
        assert!(message_type.ends_with("cc::protocol::MessageType"));
    }
}
