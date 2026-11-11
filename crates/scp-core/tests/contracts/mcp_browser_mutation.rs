use super::{
    McpRemoteBrowserCreateDirectoryArgs, McpRemoteBrowserCreateDirectoryResult,
    McpRemoteBrowserDeletePathArgs, McpRemoteBrowserDeletePathResult,
};
use serde_json::{json, Value};

#[test]
fn mutations_require_browser_id_before_path_with_exact_errors() {
    let invalid = [Value::Null, json!(false), json!(3), json!([]), json!({})];
    let mut inputs = invalid.to_vec();
    inputs.extend([json!("id"), json!({"browser_id":"b","path":"p"})]);
    for value in &invalid {
        inputs.push(json!({"browserId":value,"path":"valid"}));
        inputs.push(json!({"browserId":value,"path":value}));
    }
    let mut cases = inputs
        .into_iter()
        .map(|input| (input, "browserId is required"))
        .collect::<Vec<_>>();
    cases.push((json!({"browserId":"b"}), "path is required"));
    cases.extend(
        invalid
            .iter()
            .map(|path| (json!({"browserId":"b","path":path}), "path is required")),
    );
    let observed = cases
        .iter()
        .map(|(input, _)| {
            (
                McpRemoteBrowserCreateDirectoryArgs::from_args(input).err(),
                McpRemoteBrowserDeletePathArgs::from_args(input).err(),
            )
        })
        .collect::<Vec<_>>();
    let expected = cases
        .iter()
        .map(|(_, error)| (Some(error.to_string()), Some(error.to_string())))
        .collect::<Vec<_>>();
    assert_eq!(observed, expected);
}

#[test]
fn mutation_strings_are_borrowed_unchanged_and_extras_are_ignored() {
    let cases = ["", " \t", "雪\n\u{1b}", "browser-4"]
        .into_iter()
        .flat_map(|browser_id| {
            ["", " \t", "../雪\n\u{0}", "/", "C:\\Users\\name\\dir"]
                .map(|path| json!({"browserId":browser_id,"path":path,"recursive":true,"extra":{}}))
        })
        .collect::<Vec<_>>();
    let observed = cases
        .iter()
        .map(|input| {
            let create = McpRemoteBrowserCreateDirectoryArgs::from_args(input).unwrap();
            let delete = McpRemoteBrowserDeletePathArgs::from_args(input).unwrap();
            let borrowed = [
                (create.browser_id, create.path),
                (delete.browser_id, delete.path),
            ]
            .map(|(id, path)| {
                (
                    std::ptr::eq(id, input["browserId"].as_str().unwrap()),
                    std::ptr::eq(path, input["path"].as_str().unwrap()),
                )
            });
            (
                serde_json::to_value(create).unwrap(),
                serde_json::to_value(delete).unwrap(),
                borrowed,
            )
        })
        .collect::<Vec<_>>();
    let expected = cases
        .iter()
        .map(|input| {
            (
                json!({"browserId":input["browserId"],"path":input["path"]}),
                json!({"browserId":input["browserId"],"path":input["path"],"recursive":true}),
                [(true, true); 2],
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(observed, expected);
}

#[test]
fn delete_preserves_optional_boolean_without_coercion() {
    let mut inputs = vec![(json!({"browserId":"b","path":"p"}), None)];
    for (recursive, expected) in [
        (Value::Null, None),
        (json!(0), None),
        (json!(1), None),
        (json!("true"), None),
        (json!([]), None),
        (json!({}), None),
        (json!(false), Some(false)),
        (json!(true), Some(true)),
    ] {
        inputs.push((
            json!({"browserId":"b","path":"p","recursive":recursive}),
            expected,
        ));
    }
    for (input, expected) in inputs {
        let args = McpRemoteBrowserDeletePathArgs::from_args(&input).unwrap();
        assert_eq!(args.recursive, expected);
        assert_eq!(
            serde_json::to_value(args).unwrap(),
            json!({"browserId":"b","path":"p","recursive":expected})
        );
    }
}

#[test]
fn mutation_success_results_reuse_the_empty_object_contract() {
    let create: McpRemoteBrowserCreateDirectoryResult = super::McpRemoteBrowserCloseResult {};
    let delete: McpRemoteBrowserDeletePathResult = super::McpRemoteBrowserCloseResult {};
    assert_eq!(serde_json::to_value(create).unwrap(), json!({}));
    assert_eq!(serde_json::to_value(delete).unwrap(), json!({}));
}
