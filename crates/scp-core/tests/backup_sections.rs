use scp_core::settings_backup::validate_sections;

#[path = "contracts/backup_sections.rs"]
mod contracts;

#[test]
fn section_validation_does_not_parse_server_records() {
    let files = std::collections::BTreeMap::from([
        ("config.json".into(), None),
        (
            "servers.json".into(),
            Some(serde_json::json!({"servers":"invalid"})),
        ),
        ("sync-local.json".into(), None),
    ]);
    assert_eq!(validate_sections(&files), Ok(()));
}
