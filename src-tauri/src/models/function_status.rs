use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PtyOutputPayload {
    pub id: String,
    pub data: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PtyExitPayload {
    pub id: String,
    pub exit_code: Option<i32>,
    pub reason: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FunctionStatus {
    pub function_id: String,
    pub state: String,
    pub message: String,
    pub boundary_token_source: Option<String>,
    pub session_id: Option<String>,
    pub updated_at_ms: u64,
}

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
