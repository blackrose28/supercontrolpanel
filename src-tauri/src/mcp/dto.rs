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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallEnvelope {
    pub ok: bool,
    pub message: String,
    pub data: serde_json::Value,
    pub warnings: Vec<String>,
    pub timestamp: u64,
}
