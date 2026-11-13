use serde_json::{json, Value};
use std::collections::BTreeMap;

type Files = BTreeMap<String, Option<Value>>;

fn files() -> Files {
    BTreeMap::from([
        (
            "config.json".into(),
            Some(json!({"app":{"defaultTerminalCols":120,"defaultTerminalRows":36}})),
        ),
        ("servers.json".into(), Some(json!({"servers":[]}))),
        ("sync-local.json".into(), Some(json!({}))),
    ])
}

#[test]
fn declarations_are_exact_even_when_absent() {
    assert_eq!(super::validate_sections(&files()), Ok(()));
    for name in ["config.json", "servers.json", "sync-local.json"] {
        let mut value = files();
        value.remove(name);
        assert_eq!(
            super::validate_sections(&value),
            Err("Backup must declare all settings sections".into())
        );
        value.insert("other.json".into(), None);
        assert_eq!(
            super::validate_sections(&value),
            Err("Backup must declare all settings sections".into())
        );
    }
    let mut value = files();
    value.insert("../extra".into(), None);
    value.insert("config.json".into(), Some(json!({})));
    assert_eq!(
        super::validate_sections(&value),
        Err("Backup must declare all settings sections".into())
    );
}

#[test]
fn absent_files_are_valid_but_present_nonobjects_are_not() {
    let mut value = files();
    for name in ["config.json", "servers.json", "sync-local.json"] {
        value.insert(name.into(), None);
        assert_eq!(super::validate_sections(&value), Ok(()));
    }
    for name in ["config.json", "servers.json", "sync-local.json"] {
        for invalid in [
            Value::Null,
            json!([]),
            json!("private-sentinel"),
            json!(false),
            json!(1),
        ] {
            let mut value = files();
            value.insert(name.into(), Some(invalid));
            assert_eq!(
                super::validate_sections(&value),
                Err("Invalid settings section".into())
            );
        }
    }
}

#[test]
fn fixed_file_order_interleaves_object_and_typed_checks() {
    let mut value = files();
    value.insert("config.json".into(), Some(json!({})));
    value.insert("servers.json".into(), Some(json!([])));
    value.insert(
        "sync-local.json".into(),
        Some(json!({"mcpPort":"private-sentinel"})),
    );
    assert_eq!(
        super::validate_sections(&value),
        Err("Invalid app settings".into())
    );
    value.insert("config.json".into(), None);
    assert_eq!(
        super::validate_sections(&value),
        Err("Invalid settings section".into())
    );
    value.insert("servers.json".into(), None);
    assert_eq!(
        super::validate_sections(&value),
        Err("Invalid local settings".into())
    );
}

#[test]
fn app_settings_use_full_schema_required_fields_defaults_enums_and_types() {
    for invalid in [
        json!({}),
        json!({"app":{}}),
        json!({"app":{"defaultTerminalCols":120}}),
        json!({"app":{"defaultTerminalCols":65536,"defaultTerminalRows":36}}),
        json!({"app":{"defaultTerminalCols":120,"defaultTerminalRows":null}}),
    ] {
        let mut value = files();
        value.insert("config.json".into(), Some(invalid));
        assert_eq!(
            super::validate_sections(&value),
            Err("Invalid app settings".into())
        );
    }
    for (section, field, invalid) in [
        ("boundary", "authKind", json!("unknown")),
        ("boundary", "oidcLoginMode", json!("unknown")),
        ("openvpn", "password", Value::Null),
        ("openvpn", "extraArgs", json!(false)),
    ] {
        let mut value = files();
        value.get_mut("config.json").unwrap().as_mut().unwrap()[section] = json!({field: invalid});
        assert_eq!(
            super::validate_sections(&value),
            Err("Invalid app settings".into())
        );
    }
    let mut value = files();
    value.get_mut("config.json").unwrap().as_mut().unwrap()["app"] =
        json!({"defaultTerminalCols":0,"defaultTerminalRows":65535});
    assert_eq!(super::validate_sections(&value), Ok(()));
}

#[test]
fn local_settings_check_types_without_runtime_policy_or_path_validation() {
    for invalid in [
        json!({"mcpPort":65536}),
        json!({"mcpPort":-1}),
        json!({"mcpPort":"1"}),
        json!({"mcpEnabled":null}),
        json!({"repoRoot":null}),
        json!({"mcpAllowedToolCategories":[1]}),
        json!({"ccAgentToken":false}),
    ] {
        let mut value = files();
        value.insert("sync-local.json".into(), Some(invalid));
        assert_eq!(
            super::validate_sections(&value),
            Err("Invalid local settings".into())
        );
    }
    for port in [0, 65535] {
        let mut value = files();
        value.insert("sync-local.json".into(), Some(json!({"mcpPort":port,"mcpBindAddress":"foreign","mcpSensitiveActionPolicy":"future","repoRoot":"../foreign","mcpAllowedToolCategories":["future"],"ccAuthMode":"future"})));
        assert_eq!(super::validate_sections(&value), Ok(()));
    }
}

#[test]
fn validation_preserves_unknown_fields_legacy_secrets_and_missing_defaults() {
    let mut value = files();
    value.get_mut("config.json").unwrap().as_mut().unwrap()["openvpn"] =
        json!({"password":"synthetic-legacy","future":{"nested":[null,1]}});
    value.get_mut("config.json").unwrap().as_mut().unwrap()["future"] = json!(true);
    value.get_mut("servers.json").unwrap().as_mut().unwrap()["future"] = json!([1]);
    value.insert(
        "sync-local.json".into(),
        Some(json!({"mcpAuthToken":"synthetic-token","future":{"nested":true}})),
    );
    let original = value.clone();
    assert_eq!(super::validate_sections(&value), Ok(()));
    assert_eq!(value, original);
}
