use super::McpToolCallEnvelope;
use serde_json::{json, Value};

#[test]
fn success_envelope_preserves_exact_desktop_wire_contract() {
    let envelope = McpToolCallEnvelope {
        ok: true,
        message: "function_status_list completed".to_owned(),
        data: json!([{"functionId": "boundary-auth", "sessionId": null, "status": "idle"}]),
        warnings: vec!["Minimal localhost HTTP transport is enabled; full MCP spec compatibility is still in progress.".to_owned()],
        timestamp: 1730000000123,
    };
    let expected: Value = serde_json::from_str(include_str!("mcp_envelope.json")).unwrap();
    assert_eq!(serde_json::to_value(envelope).unwrap(), expected);
}

#[test]
fn envelope_preserves_explicit_empty_null_and_false_values() {
    let envelope = McpToolCallEnvelope {
        ok: false,
        message: String::new(),
        data: Value::Null,
        warnings: Vec::new(),
        timestamp: 0,
    };
    assert_eq!(
        serde_json::to_value(envelope).unwrap(),
        json!({
            "ok": false, "message": "", "data": null, "warnings": [], "timestamp": 0
        })
    );
}

#[test]
fn envelope_preserves_opaque_data_warning_order_and_full_timestamp_range() {
    let data = json!({"future": [null, false, 0, "雪\n\u{1b}"], "timestamp": "opaque"});
    let envelope = McpToolCallEnvelope {
        ok: true,
        message: "arbitrary\nmessage".to_owned(),
        data: data.clone(),
        warnings: vec!["second".to_owned(), "first".to_owned(), "second".to_owned()],
        timestamp: u64::MAX,
    };
    assert_eq!(
        serde_json::to_value(envelope).unwrap(),
        json!({
            "ok": true, "message": "arbitrary\nmessage", "data": data,
            "warnings": ["second", "first", "second"], "timestamp": u64::MAX
        })
    );
}
