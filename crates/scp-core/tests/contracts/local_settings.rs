use super::*;
use serde_json::{json, Value};

fn full_settings() -> Value {
    json!({
        "repoRoot": "repo", "openvpnExecutablePath": "vpn.exe",
        "openvpnOvpnPath": "client.ovpn", "boundaryCliPath": "boundary",
        "caddyFolder": "caddy", "mcpEnabled": true,
        "mcpBindAddress": "custom-address", "mcpPort": 1234,
        "mcpAuthToken": "private-mcp-token",
        "mcpAllowedToolCategories": ["unknown-category", "read-only", "read-only"],
        "mcpSensitiveActionPolicy": "unknown-policy", "ccEnabled": true,
        "ccServerUrl": "https://cc.invalid", "ccAuthMode": "custom-auth",
        "ccAgentToken": "private-cc-token", "ccDeviceId": "device-id",
        "ccDeviceName": "device\nname", "firstRunCompleted": true
    })
}

fn assert_token(token: &str) {
    assert_eq!(token.len(), 32);
    assert!(token
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    let uuid = uuid::Uuid::parse_str(token).unwrap();
    assert_eq!(uuid.get_version_num(), 4);
    assert_eq!(uuid.get_variant(), uuid::Variant::RFC4122);
}

#[test]
fn exact_defaults_and_missing_fields_preserve_distinct_cc_auth_modes() {
    let defaults = SyncSettings::default();
    let omitted: SyncSettings =
        serde_json::from_value(json!({"unknownFutureField": true})).unwrap();
    assert_token(&defaults.mcp_auth_token);
    assert_token(&omitted.mcp_auth_token);
    assert_ne!(defaults.mcp_auth_token, omitted.mcp_auth_token);
    let mut expected = json!({
        "repoRoot": "", "openvpnExecutablePath": "", "openvpnOvpnPath": "",
        "boundaryCliPath": "", "caddyFolder": "", "mcpEnabled": false,
        "mcpBindAddress": "127.0.0.1", "mcpPort": 46739,
        "mcpAuthToken": defaults.mcp_auth_token,
        "mcpAllowedToolCategories": ["read-only", "runtime-control"],
        "mcpSensitiveActionPolicy": "require-ui-approval", "ccEnabled": false,
        "ccServerUrl": "", "ccAuthMode": "token", "ccAgentToken": "",
        "ccDeviceId": "", "ccDeviceName": "", "firstRunCompleted": false
    });
    assert_eq!(serde_json::to_value(&defaults).unwrap(), expected);
    expected["ccAuthMode"] = json!("");
    expected["mcpAuthToken"] = json!(omitted.mcp_auth_token);
    assert_eq!(serde_json::to_value(&omitted).unwrap(), expected);
    assert_eq!(default_mcp_bind_address(), "127.0.0.1");
    assert_eq!(default_mcp_port(), 46739);
    assert_eq!(
        default_mcp_allowed_tool_categories(),
        ["read-only", "runtime-control"]
    );
    assert_eq!(default_mcp_sensitive_action_policy(), "require-ui-approval");
    let token = generate_mcp_auth_token();
    assert_token(&token);
    assert_ne!(token, generate_mcp_auth_token());
}

#[test]
fn exact_camel_case_round_trip_keeps_paths_tokens_and_unvalidated_policy() {
    let fixture = full_settings();
    let settings: SyncSettings = serde_json::from_value(fixture.clone()).unwrap();
    assert_eq!(serde_json::to_value(settings.clone()).unwrap(), fixture);
}

#[test]
fn explicit_empty_and_zero_values_are_not_normalized_by_schema() {
    let mut fixture = full_settings();
    for value in fixture.as_object_mut().unwrap().values_mut() {
        *value = match value {
            Value::String(_) => json!(""),
            Value::Bool(_) => json!(false),
            Value::Number(_) => json!(0),
            Value::Array(_) => json!([]),
            _ => unreachable!(),
        };
    }
    let settings: SyncSettings = serde_json::from_value(fixture.clone()).unwrap();
    assert_eq!(serde_json::to_value(settings).unwrap(), fixture);
}

#[test]
fn patches_preserve_omitted_null_and_explicit_values_for_every_field() {
    macro_rules! check_fields {
        ($($field:ident),+ $(,)?) => {{
            let empty: SyncSettingsPatch = serde_json::from_value(json!({"unknown": 1})).unwrap();
            let defaults = SyncSettingsPatch::default();
            let fixture = full_settings();
            let settings: SyncSettings = serde_json::from_value(fixture.clone()).unwrap();
            let full: SyncSettingsPatch = serde_json::from_value(fixture.clone()).unwrap();
            let nulls = fixture.as_object().unwrap().keys()
                .map(|key| (key.clone(), Value::Null)).collect::<serde_json::Map<_, _>>();
            let nulls: SyncSettingsPatch = serde_json::from_value(Value::Object(nulls)).unwrap();
            $(assert!(empty.$field.is_none());
              assert!(defaults.$field.is_none());
              assert!(nulls.$field.is_none());
              assert_eq!(full.$field, Some(settings.$field));)+
        }};
    }
    check_fields!(
        repo_root,
        openvpn_executable_path,
        openvpn_ovpn_path,
        boundary_cli_path,
        caddy_folder,
        mcp_enabled,
        mcp_bind_address,
        mcp_port,
        mcp_auth_token,
        mcp_allowed_tool_categories,
        mcp_sensitive_action_policy,
        cc_enabled,
        cc_server_url,
        cc_auth_mode,
        cc_agent_token,
        cc_device_id,
        cc_device_name,
        first_run_completed
    );
    let patch: SyncSettingsPatch = serde_json::from_value(json!({
        "mcpEnabled": false, "mcpPort": 0, "mcpAuthToken": "",
        "mcpAllowedToolCategories": [], "firstRunCompleted": false
    }))
    .unwrap();
    assert_eq!(patch.mcp_enabled, Some(false));
    assert_eq!(patch.mcp_port, Some(0));
    assert_eq!(patch.mcp_auth_token.as_deref(), Some(""));
    assert_eq!(patch.mcp_allowed_tool_categories, Some(vec![]));
    assert_eq!(patch.first_run_completed, Some(false));
}

#[test]
fn schema_rejects_wrong_types_and_out_of_range_ports() {
    for invalid in [
        json!({"mcpPort": -1}),
        json!({"mcpPort": 65536}),
        json!({"mcpEnabled": "true"}),
        json!({"mcpAuthToken": 1}),
        json!({"mcpAllowedToolCategories": [1]}),
        json!({"firstRunCompleted": 1}),
    ] {
        assert!(serde_json::from_value::<SyncSettings>(invalid.clone()).is_err());
        assert!(serde_json::from_value::<SyncSettingsPatch>(invalid).is_err());
    }
    for field in full_settings().as_object().unwrap().keys() {
        assert!(serde_json::from_value::<SyncSettings>(json!({field: null})).is_err());
    }
    let maximum: SyncSettings = serde_json::from_value(json!({"mcpPort": 65535})).unwrap();
    assert_eq!(maximum.mcp_port, u16::MAX);
}
