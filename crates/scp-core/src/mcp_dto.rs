//! Wire DTOs for the existing custom MCP adapters.
//!
//! These types carry values only: hosts retain result construction, policy,
//! clock and transport behavior. They do not define standard MCP results.

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallEnvelope {
    pub ok: bool,
    pub message: String,
    pub data: serde_json::Value,
    pub warnings: Vec<String>,
    pub timestamp: u64,
}
