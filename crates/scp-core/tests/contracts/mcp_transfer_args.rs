use serde_json::{json, Value};

#[test]
fn transfer_id_lookup_borrows_any_string_and_preserves_required_error() {
    for input in [
        Value::Null,
        json!(false),
        json!(0),
        json!("transferId"),
        json!([]),
        json!({}),
        json!({"transfer_id":"id"}),
        json!({"transferId":null}),
        json!({"transferId":false}),
        json!({"transferId":1}),
        json!({"transferId":[]}),
        json!({"transferId":{}}),
    ] {
        assert_eq!(
            super::McpScpTransferGetArgs::from_args(&input)
                .err()
                .unwrap(),
            "transferId is required"
        );
    }
    for id in ["scp-123", "", " \t", "雪\n\u{1b}"] {
        let input = json!({"transferId":id, "ignored":[null, false]});
        let args = super::McpScpTransferGetArgs::from_args(&input).unwrap();
        assert_eq!(args.transfer_id, id);
        assert_eq!(
            args.transfer_id.as_ptr(),
            input["transferId"].as_str().unwrap().as_ptr()
        );
        assert_eq!(
            serde_json::to_value(args).unwrap(),
            json!({"transferId":id})
        );
    }
}
