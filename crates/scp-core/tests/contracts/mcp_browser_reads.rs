use super::{McpRemoteBrowserListChildrenResult, McpRemoteBrowserStatPathResult, RemotePathKind};
use serde_json::json;

#[test]
fn read_results_preserve_fields_kinds_text_order_and_duplicates() {
    let kinds = [
        (RemotePathKind::File, "file"),
        (RemotePathKind::Directory, "directory"),
        (RemotePathKind::Symlink, "symlink"),
        (RemotePathKind::Other, "other"),
        (RemotePathKind::Missing, "missing"),
    ];
    let mut entries: McpRemoteBrowserListChildrenResult = Vec::new();
    let mut expected = Vec::new();
    for (kind, wire) in kinds {
        // DTOs carry host values; they do not enforce exists/kind consistency.
        for exists in [true, false] {
            let info = McpRemoteBrowserStatPathResult {
                path: " ../雪\n\u{1b} ".into(),
                kind: kind.clone(),
                exists,
            };
            assert_eq!(
                serde_json::to_value(info.clone()).unwrap(),
                json!({"path":" ../雪\n\u{1b} ", "kind":wire, "exists":exists})
            );
        }
        entries.push(super::RemoteBrowseEntry {
            name: " ".into(),
            path: "".into(),
            kind,
        });
        expected.push(json!({"name":" ", "path":"", "kind":wire}));
    }
    entries.push(entries[0].clone());
    expected.push(expected[0].clone());
    assert_eq!(
        serde_json::to_value(entries.clone()).unwrap(),
        json!(expected)
    );
    let empty: McpRemoteBrowserListChildrenResult = Vec::new();
    assert_eq!(serde_json::to_value(empty).unwrap(), json!([]));
}
