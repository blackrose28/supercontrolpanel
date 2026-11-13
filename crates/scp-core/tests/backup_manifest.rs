use scp_core::settings_backup::{validate_manifest, ExternalFile};
use serde_json::json;

const LIMIT: &str = "Backup manifest metadata exceeds size limits";
const SCHEMA: &str = "Unsupported settings schema; update the application";

#[test]
fn external_reference_wire_shape_stays_strict_and_required() {
    let wire = json!({"kind":"foreign-key", "path":"C:\\keys\\id"});
    let reference: ExternalFile = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(reference).unwrap(), wire);
    for invalid in [
        json!({"kind":"key"}),
        json!({"path":"x"}),
        json!({"kind":"key","path":"x","contents":"forbidden"}),
        json!({"kind":null,"path":"x"}),
    ] {
        assert!(serde_json::from_value::<ExternalFile>(invalid).is_err());
    }
}

#[test]
fn manifest_limits_are_inclusive_bytes_without_path_or_kind_policy() {
    let references = vec![ExternalFile {
        kind: "é".repeat(64),
        path: "é".repeat(2048),
    }];
    assert_eq!(
        validate_manifest(1, &"é".repeat(32), &"x".repeat(64), &references),
        Ok(())
    );
    let empty = vec![ExternalFile {
        kind: String::new(),
        path: String::new(),
    }];
    assert_eq!(validate_manifest(1, "", "", &empty), Ok(()));
    let many: Vec<_> = (0..10000)
        .map(|_| ExternalFile {
            kind: "unknown".into(),
            path: "../foreign".into(),
        })
        .collect();
    assert_eq!(validate_manifest(1, "", "", &many), Ok(()));
}

#[test]
fn manifest_rejects_each_oversize_dimension_before_schema() {
    for (version, platform, kind, path, count) in [
        (
            "x".repeat(65),
            String::new(),
            String::new(),
            String::new(),
            0,
        ),
        (
            String::new(),
            "é".repeat(33),
            String::new(),
            String::new(),
            0,
        ),
        (
            String::new(),
            String::new(),
            "x".repeat(129),
            String::new(),
            1,
        ),
        (
            String::new(),
            String::new(),
            String::new(),
            "x".repeat(4097),
            1,
        ),
        (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            10001,
        ),
    ] {
        let references: Vec<_> = (0..count)
            .map(|_| ExternalFile {
                kind: kind.clone(),
                path: path.clone(),
            })
            .collect();
        for schema in [1, 2] {
            assert_eq!(
                validate_manifest(schema, &version, &platform, &references),
                Err(LIMIT.into())
            );
        }
    }
}

#[test]
fn unsupported_schema_is_fixed_error_after_valid_metadata() {
    for schema in [0, 2, u32::MAX] {
        assert_eq!(validate_manifest(schema, "", "", &[]), Err(SCHEMA.into()));
    }
    assert_eq!(validate_manifest(1, "", "", &[]), Ok(()));
}
