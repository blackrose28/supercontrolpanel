use serde_json::{json, Value};

#[test]
fn browser_reads_require_a_string_browser_id_and_default_only_non_string_paths() {
    for input in [
        Value::Null,
        json!(false),
        json!(1),
        json!("browserId"),
        json!([]),
        json!({}),
        json!({"browser_id":"id"}),
        json!({"browserId":null}),
        json!({"browserId":false}),
        json!({"browserId":1}),
        json!({"browserId":[]}),
        json!({"browserId":{}}),
    ] {
        assert_eq!(
            super::McpRemoteBrowserReadArgs::from_args(&input)
                .err()
                .unwrap(),
            "browserId is required"
        );
    }
    for id in ["id", "", " \t", "雪\n\u{1b}"] {
        for path in [
            None,
            Some(Value::Null),
            Some(json!(false)),
            Some(json!(1)),
            Some(json!([])),
            Some(json!({})),
            Some(json!("")),
            Some(json!(" ../雪\\folder\n\u{1b} ")),
        ] {
            let mut input = json!({"browserId":id,"ignored":{"path":"other"}});
            if let Some(path) = path {
                input["path"] = path;
            }
            let expected = input.get("path").and_then(Value::as_str).unwrap_or("");
            let args = super::McpRemoteBrowserReadArgs::from_args(&input).unwrap();
            assert_eq!(args.browser_id, id);
            assert_eq!(args.path, expected);
            assert_eq!(
                args.browser_id.as_ptr(),
                input["browserId"].as_str().unwrap().as_ptr()
            );
            if let Some(path) = input.get("path").and_then(Value::as_str) {
                assert_eq!(args.path.as_ptr(), path.as_ptr());
            }
            assert_eq!(
                serde_json::to_value(args).unwrap(),
                json!({"browserId":id,"path":expected})
            );
        }
    }
}
