use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

use scp_core::boundary_ops::parse_boundary_targets;
use scp_core::models::{
    BoundaryAuthKind, BoundaryOidcLoginMode, KEYRING_BOUNDARY_APP_TOKEN, KEYRING_BOUNDARY_PASSWORD,
};
use scp_core::profile::ProfileLocationInput;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::MemoryKeyringBackend;
use scp_core::supervisor::CancelToken;

fn open_boundary_runtime(
    tmp: &TempDir,
    app_cfg: serde_json::Value,
    script_name: &str,
    script_body: &str,
) -> (ProfileRuntime<MemoryKeyringBackend>, PathBuf) {
    let script_path = tmp.path().join(script_name);
    fs::write(&script_path, script_body).unwrap();
    fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();

    let cfg_dir = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let pdir = cfg_dir.join("profiles").join("default");
    fs::create_dir_all(&pdir).unwrap();
    fs::write(pdir.join("config.json"), app_cfg.to_string()).unwrap();
    fs::write(
        pdir.join("sync-local.json"),
        json!({ "boundaryCliPath": script_path.to_string_lossy() }).to_string(),
    )
    .unwrap();
    let rt = ProfileRuntime::open_with_backend(
        &ProfileLocationInput {
            config_dir: Some(&cfg_dir),
            desktop_config_dir: Some(&desktop),
            profile: Some("default"),
            ..Default::default()
        },
        "test",
        MemoryKeyringBackend::default(),
    )
    .unwrap();
    (rt, cfg_dir)
}

#[test]
fn boundary_target_parser_characterization() {
    let doc = json!({
        "items": [
            {
                "id": "t_ops",
                "name": "Gateway-East",
                "type": "ssh",
                "description": "TopLevel Desc",
                "address": "gw.top.example"
            },
            {
                "id": "t_fin",
                "name": "Ledger-Node",
                "type": "tcp",
                "attributes": { "description": "Billing Postgres", "address": "10.9.8.7" }
            },
            { "id": "", "name": "empty-id" },
            { "name": "missing-id" },
            "not-an-object"
        ]
    })
    .to_string();
    let bytes = doc.as_bytes();

    let cases: &[(Option<&str>, &[&str])] = &[
        (None, &["t_ops", "t_fin"]),
        (Some("  "), &["t_ops", "t_fin"]),
        (Some("gateway"), &["t_ops"]),
        (Some("BILLING"), &["t_fin"]),
        (Some("T_FIN"), &["t_fin"]),
        (Some("nowhere"), &[]),
    ];
    for (needle, expected_ids) in cases {
        let parsed = parse_boundary_targets(bytes, *needle).unwrap();
        let ids: Vec<&str> = parsed.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(&ids, expected_ids, "needle={needle:?}");
    }

    let first = &parse_boundary_targets(bytes, Some("t_ops")).unwrap()[0];
    assert_eq!(first.target_type, "ssh");
    assert_eq!(first.description, "TopLevel Desc");
    assert_eq!(first.address, "gw.top.example");

    assert!(parse_boundary_targets(b"{broken", None).is_err());
    assert!(parse_boundary_targets(br#"{"unexpected":1}"#, None).is_err());
    assert!(parse_boundary_targets(br#"{"items":"bad"}"#, None).is_err());
}

fn fake_boundary_search_script(env_log: &Path) -> String {
    format!(
        r#"#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
with open({:?}, "a") as f:
    f.write(json.dumps({{
        "args": args,
        "skip": os.environ.get("BOUNDARY_SKIP_CACHE_DAEMON"),
        "tname": os.environ.get("BOUNDARY_TOKEN_NAME"),
        "addr": os.environ.get("BOUNDARY_ADDR"),
        "kr": os.environ.get("BOUNDARY_KEYRING_TYPE"),
        "tok": os.environ.get("BOUNDARY_TOKEN"),
    }}) + "\n")
if args[:2] == ["targets", "list"]:
    tok = os.environ.get("BOUNDARY_TOKEN") or ""
    if tok == "trigger-redacted-fail":
        sys.stderr.write(f"fatal with secret {{tok}} and pw pw-redact-777\n")
        sys.exit(1)
    print(json.dumps({{"items": [
        {{"id": "ttcp_1", "name": "Prod-A", "type": "tcp"}},
        {{"id": "ttcp_2", "name": "Prod-B", "type": "tcp"}},
        {{"id": "ttcp_3", "name": "Prod-C", "type": "tcp"}}
    ]}}))
elif args[:2] == ["config", "get-token"]:
    tok = os.environ.get("BOUNDARY_TOKEN") or "cli-stored-token"
    print(tok)
else:
    sys.exit(2)
"#,
        env_log.to_string_lossy()
    )
}

#[test]
fn boundary_external_state_isolation() {
    let tmp = TempDir::new().unwrap();
    let env_log = tmp.path().join("env.jsonl");
    let cfg = json!({
        "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 36},
        "boundary": {"addr": "https://boundary.example.test", "authKind": "password"}
    });
    let (rt, _) = open_boundary_runtime(
        &tmp,
        cfg,
        "fake-boundary",
        &fake_boundary_search_script(&env_log),
    );
    assert_eq!(
        rt.search_boundary_targets(None, Some(0)).unwrap_err().code,
        "INVALID_ARGUMENTS"
    );
    assert_eq!(
        rt.search_boundary_targets(None, Some(101))
            .unwrap_err()
            .code,
        "INVALID_ARGUMENTS"
    );

    rt.write_secret(KEYRING_BOUNDARY_APP_TOKEN, Some("super-app-tok-999"))
        .unwrap();
    let rows = rt.search_boundary_targets(Some("prod"), Some(2)).unwrap();
    assert_eq!(rows.len(), 2);

    let status = rt.refresh_boundary_status().unwrap();
    assert_eq!(status.boundary_token_source.as_deref(), Some("app"));
    assert_eq!(status.state, "authenticated");

    let recorded: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&env_log)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(recorded["skip"], "true");
    assert_eq!(
        recorded["tname"],
        format!("scpctl-{}", rt.credential_namespace())
    );
    assert_eq!(recorded["kr"], "none");
    assert_eq!(recorded["tok"], "super-app-tok-999");

    rt.write_secret(KEYRING_BOUNDARY_PASSWORD, Some("pw-redact-777"))
        .unwrap();
    rt.write_secret(KEYRING_BOUNDARY_APP_TOKEN, Some("trigger-redacted-fail"))
        .unwrap();
    let err = rt.search_boundary_targets(None, None).unwrap_err();
    assert!(!err.message.contains("trigger-redacted-fail"));
    assert!(!err.message.contains("pw-redact-777"));
    assert!(err.message.contains("[REDACTED]"));
}

#[test]
fn boundary_auth_capability_cleanup() {
    let script_body = "#!/usr/bin/env python3\nimport json\nprint(json.dumps({\"attributes\": {\"token\": \"tbat_from_pw\"}}))\n";
    assert_oidc_rejected(
        script_body,
        BoundaryOidcLoginMode::Automated,
        "webview_oidc_desktop_only",
    );
    assert_oidc_rejected(
        script_body,
        BoundaryOidcLoginMode::Browser,
        "browser_oidc_unverified",
    );
    assert_password_auth_succeeds(script_body);
}

fn assert_oidc_rejected(script_body: &str, mode: BoundaryOidcLoginMode, expected_reason: &str) {
    let tmp = TempDir::new().unwrap();
    let cfg = json!({
        "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 36},
        "boundary": {
            "addr": "https://boundary.example.test",
            "authMethodId": "amoidc_1",
            "authKind": BoundaryAuthKind::Oidc,
            "oidcLoginMode": mode
        }
    });
    let (rt, _) = open_boundary_runtime(&tmp, cfg, "fake-boundary-oidc", script_body);
    let err = rt.start_boundary_auth(&CancelToken::new()).unwrap_err();
    assert_eq!(err.code, "CAPABILITY_UNAVAILABLE");
    assert_eq!(err.reason, expected_reason);
}

fn assert_password_auth_succeeds(script_body: &str) {
    let tmp = TempDir::new().unwrap();
    let cfg = json!({
        "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 36},
        "boundary": {
            "addr": "https://boundary.example.test",
            "authMethodId": "ampw_1",
            "loginName": "alice",
            "authKind": "password"
        }
    });
    let (rt, _) = open_boundary_runtime(&tmp, cfg, "fake-boundary-auth", script_body);
    let cancel = CancelToken::new();
    let err_missing = rt.start_boundary_auth(&cancel).unwrap_err();
    assert_eq!(err_missing.code, "CAPABILITY_UNAVAILABLE");
    assert_eq!(err_missing.reason, "credentials_missing");

    rt.write_secret(KEYRING_BOUNDARY_PASSWORD, Some("pw-secret-123"))
        .unwrap();
    let started = rt.start_boundary_auth(&cancel).unwrap();
    assert!(rt
        .supervisor()
        .wait_for_session_exit(&started.session_id, Duration::from_secs(5)));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let st = rt.get_function_status(&started.function_id).unwrap();
        if st.session_id.is_none() {
            assert_eq!(st.state, "authenticated");
            assert_eq!(
                rt.read_optional_secret(KEYRING_BOUNDARY_APP_TOKEN)
                    .as_deref(),
                Some("tbat_from_pw")
            );
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(15));
    }
}

#[test]
fn boundary_auth_owned_stop() {
    let tmp = TempDir::new().unwrap();
    let script_body = "#!/usr/bin/env python3\nimport signal, sys, time\nsignal.signal(signal.SIGTERM, lambda *_: sys.exit(0))\ntime.sleep(30)\n";
    let pw_cfg = json!({
        "app": {"defaultTerminalCols": 120, "defaultTerminalRows": 36},
        "boundary": {
            "addr": "https://boundary.example.test",
            "authMethodId": "ampw_1",
            "loginName": "alice",
            "authKind": "password"
        }
    });
    let (rt, _) = open_boundary_runtime(&tmp, pw_cfg, "slow-boundary-auth", script_body);
    rt.write_secret(KEYRING_BOUNDARY_PASSWORD, Some("pw-secret-123"))
        .unwrap();
    let cancel = CancelToken::new();
    let _started = rt.start_boundary_auth(&cancel).unwrap();
    assert!(rt.stop_boundary_auth(Duration::from_secs(2)).unwrap());
    assert!(!rt.stop_boundary_auth(Duration::from_secs(2)).unwrap());
}
