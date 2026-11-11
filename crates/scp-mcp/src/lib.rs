//! Shared desktop MCP registry and schemas. Transport and host authority remain in adapters.
pub mod contracts;
pub mod public_dto;
pub mod validation;

/// Discovery is metadata only: entries are not executable by the standalone host yet.
pub fn standalone_catalog() -> serde_json::Value {
    use serde_json::json;
    let mut operations = contracts::OPERATIONS.iter().collect::<Vec<_>>();
    operations.sort_by_key(|op| op.name);
    let tools = operations
        .iter()
        .map(|op| {
            json!({
                "name":op.name,"description":op.title,"category":op.category,
                "readOnly":op.read_only,"inputSchema":validation::input_schema(op),
                "outputSchema":validation::output_schema(op),
                "available":false,"reason":"runtime_not_extracted"
            })
        })
        .collect::<Vec<_>>();
    let mut resources = operations
        .iter()
        .filter_map(|op| {
            op.resource.map(|(uri, title, category)| {
                json!({
                    "uri":uri,"name":title,"category":category,"operation":op.name,
                    "available":false,"reason":"runtime_not_extracted"
                })
            })
        })
        .collect::<Vec<_>>();
    resources.sort_by(|a, b| a["uri"].as_str().cmp(&b["uri"].as_str()));
    json!({"mcpOperations":tools,"mcpResources":resources})
}
