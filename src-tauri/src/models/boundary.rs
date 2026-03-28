use serde::Serialize;

/// One target from `boundary targets list` for search results.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryTargetSummary {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target_type: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub address: String,
}
