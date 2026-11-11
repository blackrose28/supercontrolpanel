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

/// Existing `function_status_get` arguments; intentionally no trimming or ID validation.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpFunctionStatusGetArgs<'a> {
    pub function_id: &'a str,
}

impl<'a> McpFunctionStatusGetArgs<'a> {
    /// Match the custom dispatcher's permissive string lookup, including its error.
    /// Extra fields are ignored. Non-object, missing and non-string values fail alike.
    pub fn from_args(args: &'a serde_json::Value) -> Result<Self, String> {
        let function_id = args
            .get("functionId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "functionId is required".to_string())?;
        Ok(Self { function_id })
    }
}

/// Bare status object, with the runtime model's exact fields and null semantics.
pub type McpFunctionStatusGetResult = crate::runtime::FunctionStatus;

/// Bare array in host-supplied iteration order; no sorting or filtering.
pub type McpFunctionStatusListResult = Vec<McpFunctionStatusGetResult>;

/// Bare `openvpn_get_status` status or null when the host's fixed-ID lookup is absent.
pub type McpOpenVpnGetStatusResult = Option<crate::runtime::FunctionStatus>;

/// Bare `boundary_get_status` status or null when the host's fixed-ID lookup is absent.
pub type McpBoundaryGetStatusResult = Option<crate::runtime::FunctionStatus>;

/// Bare `app_get_overview` result; hosts supply version and store-derived counts.
/// Active functions count statuses with any session ID, independently of state.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppOverviewResult {
    pub app_version: String,
    pub active_functions: usize,
    pub known_function_count: usize,
}
