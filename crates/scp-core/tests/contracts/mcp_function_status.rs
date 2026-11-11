use super::{McpFunctionStatusGetResult, McpFunctionStatusListResult};
use serde_json::{json, Value};

fn status() -> McpFunctionStatusGetResult {
    McpFunctionStatusGetResult {
        function_id: "boundary-auth".to_owned(),
        state: "future-state".to_owned(),
        message: "雪\n\u{1b}".to_owned(),
        boundary_token_source: None,
        session_id: None,
        updated_at_ms: u64::MAX,
    }
}

#[test]
fn get_and_list_results_preserve_the_exact_status_wire_contract() {
    let expected: Value = serde_json::from_str(include_str!("mcp_function_status.json")).unwrap();
    assert_eq!(serde_json::to_value(status()).unwrap(), expected);
    let mut active = status();
    active.function_id = "z-before-a".to_owned();
    active.state = String::new();
    active.message = String::new();
    active.boundary_token_source = Some("".to_owned());
    active.session_id = Some("pty-雪".to_owned());
    active.updated_at_ms = 0;
    let list: McpFunctionStatusListResult = vec![active, status(), status()];
    assert_eq!(
        serde_json::to_value(list).unwrap(),
        json!([
            {"functionId":"z-before-a", "state":"", "message":"", "boundaryTokenSource":"",
             "sessionId":"pty-雪", "updatedAtMs":0}, expected, expected
        ])
    );
    let empty: McpFunctionStatusListResult = Vec::new();
    assert_eq!(serde_json::to_value(empty).unwrap(), json!([]));
}

#[test]
fn get_args_preserve_strings_and_ignore_extra_fields() {
    for id in ["boundary-auth", "", " \t", "雪\n\u{1b}"] {
        let input = json!({"functionId":id, "future": {"ignored": true}});
        let parsed = super::McpFunctionStatusGetArgs::from_args(&input).unwrap();
        assert_eq!(parsed.function_id, id);
        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            json!({"functionId":id})
        );
    }
}

#[test]
fn get_args_keep_the_existing_missing_or_wrong_type_error() {
    for input in [
        Value::Null,
        json!(false),
        json!(0),
        json!("functionId"),
        json!([]),
        json!({}),
        json!({"function_id":"boundary-auth"}),
        json!({"functionId":null}),
        json!({"functionId":false}),
        json!({"functionId":1}),
        json!({"functionId":[]}),
        json!({"functionId":{}}),
    ] {
        assert_eq!(
            super::McpFunctionStatusGetArgs::from_args(&input)
                .err()
                .as_deref(),
            Some("functionId is required"),
            "input: {input}"
        );
    }
}
