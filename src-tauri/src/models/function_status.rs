use serde::Serialize;

pub use scp_core::runtime::{FunctionStatus, PtyExitPayload, PtyOutputPayload};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StartFunctionResult {
    pub function_id: String,
    pub session_id: String,
}

#[cfg(target_os = "windows")]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OpenVpnLogPayload {
    pub function_id: String,
    pub session_id: String,
    pub data: String,
}
