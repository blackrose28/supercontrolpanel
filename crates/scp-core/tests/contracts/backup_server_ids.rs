use super::server_ids;
use serde_json::{json, Value};

fn record(id: &str) -> Value {
    json!({"id":id,"name":"server","host":"host","connectionMethod":"ssh","sshPort":22,"sshUser":"user"})
}

#[test]
fn absent_file_differs_from_present_invalid_list() {
    assert_eq!(server_ids(None), Ok(vec![]));
    assert_eq!(server_ids(Some(&json!({"servers":[]}))), Ok(vec![]));
    for invalid in [
        Value::Null,
        json!({}),
        json!({"servers":null}),
        json!({"servers":{}}),
        json!([]),
    ] {
        assert_eq!(
            server_ids(Some(&invalid)),
            Err("Invalid backup server list".into())
        );
    }
}

#[test]
fn ids_are_sorted_case_sensitive_and_preserve_desktop_alphabet() {
    let value =
        json!({"unknown":"preserved", "servers":[record("z:_.-9"),record("a"),record("A")]});
    assert_eq!(
        server_ids(Some(&value)),
        Ok(vec!["A".into(), "a".into(), "z:_.-9".into()])
    );
}

#[test]
fn records_use_full_server_deserialization_with_defaults_and_unknown_fields() {
    let mut valid = record("id");
    valid["future"] = json!({"opaque":true});
    assert_eq!(
        server_ids(Some(&json!({"servers":[valid.clone()]}))),
        Ok(vec!["id".into()])
    );
    for field in [
        "id",
        "name",
        "host",
        "connectionMethod",
        "sshPort",
        "sshUser",
    ] {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert_eq!(
            server_ids(Some(&json!({"servers":[missing]}))),
            Err("Invalid backup server record".into()),
            "{field}"
        );
    }
    for (field, invalid) in [
        ("os", json!("macos")),
        ("os", Value::Null),
        ("connectionMethod", json!("SSH")),
        ("sshPort", json!(65536)),
        ("sshUser", json!(7)),
        ("boundaryTargetId", Value::Null),
        ("boundaryListenPort", json!(-1)),
        ("services", Value::Null),
        ("services", json!([{"name":"service","port":65536}])),
        ("services", json!([{"name":"service"}])),
    ] {
        let mut bad = valid.clone();
        bad[field] = invalid;
        assert_eq!(
            server_ids(Some(&json!({"servers":[bad]}))),
            Err("Invalid backup server record".into()),
            "{field}"
        );
    }
    valid["os"] = json!("windows");
    valid["connectionMethod"] = json!("boundary");
    valid["sshPort"] = json!(0);
    valid["services"] = json!([{"name":"", "port":65535, "future":true}]);
    assert_eq!(
        server_ids(Some(&json!({"servers":[valid]}))),
        Ok(vec!["id".into()])
    );
}

#[test]
fn id_limits_and_duplicate_rejection_are_exact() {
    let longest = "a".repeat(256);
    assert_eq!(
        server_ids(Some(&json!({"servers":[record(&longest)]}))),
        Ok(vec![longest])
    );
    for invalid in [
        "".into(),
        "a".repeat(257),
        "é".repeat(128),
        "a b".into(),
        "a/b".into(),
        "a\\b".into(),
        "a\n".into(),
    ] {
        assert_eq!(
            server_ids(Some(&json!({"servers":[record(&invalid)]}))),
            Err("Invalid or duplicate backup server ID".into())
        );
    }
    assert_eq!(
        server_ids(Some(&json!({"servers":[record("a:b"),record("a:b")]}))),
        Err("Invalid or duplicate backup server ID".into())
    );
}

#[test]
fn count_limit_is_inclusive_and_precedes_record_errors() {
    let mut records: Vec<_> = (0..10000).map(|n| record(&format!("id-{n}"))).collect();
    assert_eq!(
        server_ids(Some(&json!({"servers":records}))).unwrap().len(),
        10000
    );
    records[0] = Value::Null;
    records.push(Value::Null);
    assert_eq!(
        server_ids(Some(&json!({"servers":records}))),
        Err("Too many servers in backup".into())
    );
}

#[test]
fn record_error_precedes_its_id_check_but_records_are_processed_in_order() {
    let mut malformed = record("");
    malformed["sshPort"] = json!("secret-sentinel");
    assert_eq!(
        server_ids(Some(&json!({"servers":[malformed]}))),
        Err("Invalid backup server record".into())
    );
    for records in [
        vec![record(""), Value::Null],
        vec![record("dup"), record("dup"), Value::Null],
    ] {
        assert_eq!(
            server_ids(Some(&json!({"servers":records}))),
            Err("Invalid or duplicate backup server ID".into())
        );
    }
    assert_eq!(
        server_ids(Some(&json!({"servers":[Value::Null,record("")]}))),
        Err("Invalid backup server record".into())
    );
}
