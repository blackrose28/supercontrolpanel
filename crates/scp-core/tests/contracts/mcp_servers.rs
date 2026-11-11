use super::{McpServerGetResult, McpServersListResult};
use serde_json::{json, Value};

#[test]
fn inventory_results_preserve_bare_objects_fields_order_and_duplicates() {
    let expected: Vec<Value> = serde_json::from_str(include_str!("mcp_servers.json")).unwrap();
    let servers: Vec<scp_core::servers::Server> = expected
        .iter()
        .cloned()
        .map(|value| serde_json::from_value(value).unwrap())
        .collect();
    assert_eq!(servers.len(), 2);
    for (server, expected) in servers.iter().zip(&expected) {
        let result: McpServerGetResult = server.clone();
        assert_eq!(serde_json::to_value(result).unwrap(), *expected);
    }
    let list: McpServersListResult =
        vec![servers[0].clone(), servers[1].clone(), servers[0].clone()];
    assert_eq!(
        serde_json::to_value(list).unwrap(),
        json!([expected[0], expected[1], expected[0]])
    );
    let empty: McpServersListResult = vec![];
    assert_eq!(serde_json::to_value(empty).unwrap(), json!([]));
}

#[test]
fn server_get_arguments_match_permissive_lookup_for_valid_and_invalid_shapes() {
    let invalid = [
        Value::Null,
        json!(true),
        json!(0),
        json!("serverId"),
        json!([]),
        json!({}),
        json!({"server_id":"s"}),
        json!({"serverId":null}),
        json!({"serverId":false}),
        json!({"serverId":1}),
        json!({"serverId":[]}),
        json!({"serverId":{}}),
    ];
    let valid = ["server-1", "", " \t", "雪\n\u{1b}"]
        .map(|id| json!({"serverId":id, "ignored":[null, false]}));
    for input in invalid.into_iter().chain(valid) {
        match (
            super::McpServerGetArgs::from_args(&input),
            input.get("serverId").and_then(Value::as_str),
        ) {
            (Ok(args), Some(id)) => {
                assert_eq!(args.server_id, id);
                assert_eq!(args.server_id.as_ptr(), id.as_ptr());
                assert_eq!(serde_json::to_value(args).unwrap(), json!({"serverId":id}));
            }
            (Err(error), None) => assert_eq!(error, "serverId is required"),
            _ => panic!("unexpected argument parsing for {input}"),
        }
    }
}
