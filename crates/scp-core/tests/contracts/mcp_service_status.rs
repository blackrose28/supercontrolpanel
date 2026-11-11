use super::{McpBoundaryGetStatusResult, McpOpenVpnGetStatusResult};
use serde_json::Value;

#[test]
fn service_status_results_preserve_bare_status_or_null_without_normalizing_values() {
    let expected: Vec<Value> =
        serde_json::from_str(include_str!("mcp_service_status.json")).unwrap();
    let statuses = [
        None,
        Some(scp_core::runtime::FunctionStatus {
            function_id: "unrelated-雪\n\u{1b}".to_owned(),
            state: "future-state".to_owned(),
            message: "雪\n\0\u{1b}".to_owned(),
            boundary_token_source: None,
            session_id: None,
            updated_at_ms: u64::MAX,
        }),
        Some(scp_core::runtime::FunctionStatus {
            function_id: String::new(),
            state: String::new(),
            message: String::new(),
            boundary_token_source: Some(String::new()),
            session_id: Some(String::new()),
            updated_at_ms: 0,
        }),
    ];
    assert_eq!(statuses.len(), expected.len());
    for (status, expected) in statuses.into_iter().zip(expected) {
        let openvpn: McpOpenVpnGetStatusResult = status.clone();
        let boundary: McpBoundaryGetStatusResult = status;
        assert_eq!(serde_json::to_value(openvpn).unwrap(), expected);
        assert_eq!(serde_json::to_value(boundary).unwrap(), expected);
    }
}
