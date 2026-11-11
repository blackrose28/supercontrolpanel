use scp_core::config::{read_shared_config, AppConfig, AppConfigForDisk};
use serde_json::{json, Value};
use std::fs;

#[test]
fn shared_models_preserve_desktop_defaults_and_disk_projection() {
    let config: AppConfig = serde_json::from_value(json!({
        "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 30},
        "openvpn": {"terminateOpenvpnWhenHelperExits": true}
    }))
    .unwrap();
    assert_eq!(config.openvpn.auto_reconnect_base_seconds, 5);
    assert_eq!(config.openvpn.auto_reconnect_max_seconds, 3600);
    assert!(config.openvpn.terminate_open_vpn_when_helper_exits);
    assert_eq!(
        serde_json::to_value(AppConfigForDisk::from(&config)).unwrap(),
        json!({
            "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 30},
            "openvpn": {
                "command": "", "extraArgs": "", "username": "",
                "terminateOpenVpnWhenHelperExits": true, "autoReconnectEnabled": false,
                "autoReconnectBaseSeconds": 5, "autoReconnectMaxSeconds": 3600
            },
            "boundary": {
                "addr": "", "authMethodId": "", "loginName": "", "authKind": "oidc",
                "oidcLoginMode": "browser", "oidcUsername": "", "connectSshOptions": ""
            }
        })
    );
}

#[test]
fn missing_config_is_absent_without_profile_initialization() {
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path().join("absent");
    for directory in [&profile, temp.path()] {
        assert!(read_shared_config(directory).unwrap().is_none());
        assert!(!profile.exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}

#[test]
fn read_projects_shared_fields_without_rewriting_legacy_secrets_or_paths() {
    let temp = tempfile::tempdir().unwrap();
    let raw = json!({
        "app": {"defaultTerminalCols": 91, "defaultTerminalRows": 27},
        "openvpn": {
            "command": "legacy-command", "executablePath": "local-vpn", "ovpnPath": "local-ovpn",
            "password": "vpn-secret", "otpSecret": "vpn-otp", "extraArgs": "--verb 3",
            "username": "alice", "autoReconnectEnabled": true,
            "autoReconnectBaseSeconds": 7, "autoReconnectMaxSeconds": 90,
            "terminateOpenVpnWhenHelperExits": true
        },
        "boundary": {
            "cliPath": "local-boundary", "addr": "https://controller.test",
            "authMethodId": "ampw_test", "loginName": "bob", "password": "boundary-secret",
            "authKind": "password", "oidcLoginMode": "automated", "oidcUsername": "carol",
            "oidcPassword": "oidc-secret", "oidcOtpSecret": "oidc-otp",
            "connectSshOptions": "-o Compression=yes"
        }
    })
    .to_string();
    let path = temp.path().join("config.json");
    fs::write(&path, &raw).unwrap();
    // These must never be read by this API, even when malformed.
    fs::write(
        temp.path().join("sync-local.json"),
        "private invalid settings",
    )
    .unwrap();
    fs::write(temp.path().join("profile.json"), "invalid identity").unwrap();
    fs::write(
        temp.path().join("runtime-owner.lock"),
        "existing owner sentinel",
    )
    .unwrap();
    let projected =
        serde_json::to_value(read_shared_config(temp.path()).unwrap().unwrap()).unwrap();
    assert_eq!(projected["app"]["defaultTerminalCols"], 91);
    assert_eq!(
        projected["openvpn"],
        json!({
            "command": "", "extraArgs": "--verb 3", "username": "alice",
            "autoReconnectEnabled": true, "autoReconnectBaseSeconds": 7,
            "autoReconnectMaxSeconds": 90, "terminateOpenVpnWhenHelperExits": true
        })
    );
    assert_eq!(
        projected["boundary"],
        json!({
            "addr": "https://controller.test", "authMethodId": "ampw_test", "loginName": "bob",
            "authKind": "password", "oidcLoginMode": "automated", "oidcUsername": "carol",
            "connectSshOptions": "-o Compression=yes"
        })
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), raw);
    assert_eq!(
        fs::read_to_string(temp.path().join("runtime-owner.lock")).unwrap(),
        "existing owner sentinel"
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 4);
}

#[test]
fn parse_and_io_failures_are_explicit_and_do_not_echo_config_values() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    for raw in [
        r#"{"app":{"defaultTerminalCols":"sensitive-value","defaultTerminalRows":24}}"#,
        r#"{"app":{"defaultTerminalCols":80,"defaultTerminalRows":24},"boundary":{"authKind":"sensitive-value"}}"#,
        r#"{"password":"sensitive-value"}"#,
        r#"{"app": "sensitive-value""#,
    ] {
        fs::write(&path, raw).unwrap();
        let error = read_shared_config(temp.path()).err().unwrap();
        assert!(error.contains("parse config"), "{error}");
        assert!(!error.contains("sensitive-value"), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), raw);
    }
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(read_shared_config(temp.path())
        .err()
        .unwrap()
        .contains("read config"));
}

#[test]
fn read_projection_is_not_a_general_free_text_redactor() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("config.json"),
        json!({
            "app": {"defaultTerminalCols": 80, "defaultTerminalRows": 24},
            "openvpn": {"extraArgs": "user-supplied-sensitive-argument"}
        })
        .to_string(),
    )
    .unwrap();
    let projected: Value =
        serde_json::to_value(read_shared_config(temp.path()).unwrap().unwrap()).unwrap();
    assert_eq!(
        projected["openvpn"]["extraArgs"],
        "user-supplied-sensitive-argument"
    );
}
