use super::{McpScpTransferGetResult, McpScpTransferListResult, ScpTransferState};
use serde_json::json;

#[test]
fn transfer_results_keep_all_fields_states_nulls_and_unvalidated_values() {
    let mut entries: McpScpTransferListResult = Vec::new();
    let mut expected = Vec::new();
    for (state, wire, progress, text) in [
        (ScpTransferState::Pending, "pending", None, None),
        (ScpTransferState::Success, "success", Some(100), Some("")),
        (
            ScpTransferState::Error,
            "error",
            Some(u8::MAX),
            Some(" 雪\n\u{1b}"),
        ),
    ] {
        let entry = McpScpTransferGetResult {
            id: "same-id".into(),
            title: "Title".into(),
            detail: "Detail".into(),
            state,
            created_at_ms: u64::MAX,
            updated_at_ms: 0,
            progress_percent: progress,
            source: text.map(str::to_owned),
            destination: text.map(str::to_owned),
            session_id: text.map(str::to_owned),
        };
        let value = json!({"id":"same-id", "title":"Title", "detail":"Detail", "state":wire,
            "createdAtMs":u64::MAX, "updatedAtMs":0, "progressPercent":progress,
            "source":text, "destination":text, "sessionId":text});
        assert_eq!(serde_json::to_value(entry.clone()).unwrap(), value);
        entries.push(entry);
        expected.push(value);
    }
    entries.push(entries[0].clone());
    expected.push(expected[0].clone());
    assert_eq!(serde_json::to_value(entries).unwrap(), json!(expected));
    let empty: McpScpTransferListResult = vec![];
    assert_eq!(serde_json::to_value(empty).unwrap(), json!([]));
}
