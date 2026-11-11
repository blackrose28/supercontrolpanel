use super::{McpRemoteBrowserCloseArgs, McpRemoteBrowserCloseResult, McpRemoteBrowserOpenArgs};
use serde_json::{json, Value};

#[test]
fn lifecycle_arguments_keep_required_errors_for_non_objects_and_non_strings() {
    let non_strings = [Value::Null, json!(false), json!(42), json!([]), json!({})];
    let mut inputs = non_strings.to_vec();
    inputs.extend([json!("id"), json!({"server_id":"s","browser_id":"b"})]);
    for invalid in non_strings {
        inputs.push(json!({"serverId":invalid,"browserId":invalid}));
    }
    for input in inputs {
        assert_eq!(
            McpRemoteBrowserOpenArgs::from_args(&input).err().unwrap(),
            "serverId is required"
        );
        assert_eq!(
            McpRemoteBrowserCloseArgs::from_args(&input).err().unwrap(),
            "browserId is required"
        );
    }
    assert!(McpRemoteBrowserOpenArgs::from_args(&json!({"browserId":"b"})).is_err());
    assert!(McpRemoteBrowserCloseArgs::from_args(&json!({"serverId":"s"})).is_err());
}

#[test]
fn lifecycle_ids_are_borrowed_unvalidated_strings_and_extras_are_ignored() {
    for id in ["", " \t", "雪\n\u{1b}", "browser-7"] {
        let input = json!({"serverId":id,"browserId":id,"path":false,"ignored":{}});
        let open = McpRemoteBrowserOpenArgs::from_args(&input).unwrap();
        let close = McpRemoteBrowserCloseArgs::from_args(&input).unwrap();
        assert_eq!(open.server_id, id);
        assert_eq!(close.browser_id, id);
        assert_eq!(
            open.server_id.as_ptr(),
            input["serverId"].as_str().unwrap().as_ptr()
        );
        assert_eq!(
            close.browser_id.as_ptr(),
            input["browserId"].as_str().unwrap().as_ptr()
        );
        assert_eq!(serde_json::to_value(open).unwrap(), json!({"serverId":id}));
        assert_eq!(
            serde_json::to_value(close).unwrap(),
            json!({"browserId":id})
        );
    }
}

#[test]
fn successful_close_serializes_as_an_empty_object() {
    assert_eq!(
        serde_json::to_value(McpRemoteBrowserCloseResult {}).unwrap(),
        json!({})
    );
}
