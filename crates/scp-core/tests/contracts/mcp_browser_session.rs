use serde_json::json;

#[test]
fn open_result_carries_only_the_host_browser_id_and_root_path() {
    let mut sessions = Vec::new();
    let mut expected = Vec::new();
    for browser_id in ["browser-42", "", " \t雪\n\u{1b}"] {
        for root_path in ["", "/", r"C:\Users\User Name", " ../雪\\home\n\u{1b} "] {
            sessions.push(super::RemoteBrowseSession {
                browser_id: browser_id.into(),
                root_path: root_path.into(),
            });
            expected.push(json!({"browserId":browser_id,"rootPath":root_path}));
        }
    }
    assert_eq!(
        serde_json::to_value(sessions.clone()).unwrap(),
        json!(expected)
    );
}
