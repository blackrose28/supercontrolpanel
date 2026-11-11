use super::McpAppOverviewResult;
use serde_json::{json, Value};

#[test]
fn overview_preserves_the_exact_three_field_wire_contract() {
    let expected: Value = serde_json::from_str(include_str!("mcp_app_overview.json")).unwrap();
    let result = McpAppOverviewResult {
        app_version: "雪\n\u{1b}".to_owned(),
        active_functions: 2,
        known_function_count: 3,
    };
    assert_eq!(serde_json::to_value(result).unwrap(), expected);
}

#[test]
fn overview_keeps_empty_versions_zero_counts_and_full_host_count_range() {
    for (active_functions, known_function_count) in [(0, 0), (usize::MAX, usize::MAX), (3, 1)] {
        let result = McpAppOverviewResult {
            app_version: String::new(),
            active_functions,
            known_function_count,
        };
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({"appVersion":"", "activeFunctions":active_functions,
                   "knownFunctionCount":known_function_count})
        );
    }
}
