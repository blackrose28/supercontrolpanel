use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeStatus {
    pub enabled: bool,
    pub active: bool,
    pub transport: String,
    pub bind_address: String,
    pub port: u16,
    pub implementation_stage: String,
    pub last_error: Option<String>,
    pub last_started_at_ms: Option<u64>,
}

pub use scp_core::mcp_catalog::McpToolDescriptor;

pub use scp_core::mcp_resources::McpResourceDescriptor;

pub use scp_core::mcp_dto::McpToolCallEnvelope;

#[cfg(test)]
mod envelope_contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_envelope.rs"
    ));
}

#[cfg(test)]
mod shared_envelope_identity {
    #[test]
    fn desktop_public_paths_accept_the_core_envelope_without_conversion() {
        let shared = scp_core::mcp_dto::McpToolCallEnvelope {
            ok: true,
            message: String::new(),
            data: serde_json::Value::Null,
            warnings: Vec::new(),
            timestamp: 0,
        };
        let dto: super::McpToolCallEnvelope = shared;
        let public: crate::mcp::McpToolCallEnvelope = dto;
        let _: scp_core::mcp_dto::McpToolCallEnvelope = public;
    }
}
