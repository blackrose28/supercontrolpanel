use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum ScpTransferState {
    Pending,
    Success,
    Error,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScpTransferEntry {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub state: ScpTransferState,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub progress_percent: Option<u8>,
    pub source: Option<String>,
    pub destination: Option<String>,
    pub session_id: Option<String>,
}
