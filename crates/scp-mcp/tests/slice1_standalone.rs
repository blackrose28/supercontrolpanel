#[path = "support/mod.rs"]
mod support;

use serde_json::json;
use std::fs;
use tempfile::TempDir;

use scp_core::models::StdioPolicy;
use scp_core::profile::{resolve_profile_paths, ProfileLocationInput};
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::{KeyringHealth, MemoryKeyringBackend};
use scp_mcp::contracts::{
    authorize, authorize_standalone, operation, resource_operation, Capability, ErrorCode,
    Lifetime, OperationError, OPERATIONS,
};
use scp_mcp::standalone::{
    capabilities_report, execute_standalone, permitted_standalone_operations,
    standalone_schema_catalog,
};
use scp_mcp::validation::{conforms, output_schema};

fn setup_runtime(health: KeyringHealth) -> (TempDir, ProfileRuntime<MemoryKeyringBackend>) {
    let sync_local = json!({
        "mcpSensitiveActionPolicy": "allow-with-token",
        "mcpAllowedToolCategories": ["read-only", "runtime-control", "config-mutation", "file-transfer"],
        "mcpAuthToken": "super-secret-mcp-token",
        "ccAgentToken": "super-secret-cc-token"
    });
    let (tmp, rt, _) =
        support::open_seeded_stdio_runtime("10.1.2.3", health, Some(sync_local), None);
    (tmp, rt)
}

#[test]
fn generalized_capability_and_lifetime_preserve_desktop_contract() {
    let _caps = [
        Capability::Runtime,
        Capability::InteractiveHost,
        Capability::ProtectedImport,
        Capability::ReviewedGit,
    ];
    let _lifetimes = [
        Lifetime::Query,
        Lifetime::Request,
        Lifetime::HostOwned,
        Lifetime::Stop,
        Lifetime::Mutation,
    ];
    assert!(authorize(
        "servers_list",
        "require-ui-approval",
        &["read-only", "runtime-control"],
    )
    .is_ok());
    assert_eq!(
        authorize(
            "server_upsert",
            "require-ui-approval",
            &["read-only", "config-mutation"],
        )
        .unwrap_err()
        .code,
        ErrorCode::PolicyDenied
    );
    // Standalone host-neutral errors differ cleanly from desktop legacy wording.
    let standalone_err = OperationError::standalone_new(ErrorCode::PolicyDenied);
    assert!(!standalone_err.message.contains("desktop"));
}

#[test]
fn standalone_authorization_defaults_to_read_only_and_fails_closed() {
    let default_policy = StdioPolicy::default();
    let default_permitted = [
        "boundary_search_targets",
        "detect_openvpn_executable",
        "app_get_overview",
        "function_status_list",
        "function_status_get",
        "openvpn_get_status",
        "boundary_get_status",
        "servers_list",
        "server_get",
        "config_get_paths",
        "sync_settings_get",
    ];
    for name in default_permitted {
        assert!(
            authorize_standalone(name, &default_policy).is_ok(),
            "{name}"
        );
    }
    assert_eq!(
        permitted_standalone_operations(&default_policy).len(),
        default_permitted.len()
    );
    // Transfer read queries carry category "file-transfer" and require that category enabled.
    for transfer_read in [
        "scp_transfer_list",
        "scp_transfer_get",
        "remote_browser_list_children",
        "remote_browser_stat_path",
    ] {
        assert_eq!(
            authorize_standalone(transfer_read, &default_policy)
                .unwrap_err()
                .code,
            ErrorCode::PolicyDenied,
            "{transfer_read}"
        );
    }

    let read_and_transfer_policy = StdioPolicy {
        sensitive_action_policy: "read-only".into(),
        allowed_tool_categories: vec!["read-only".into(), "file-transfer".into()],
    };
    assert_eq!(
        permitted_standalone_operations(&read_and_transfer_policy).len(),
        15
    );
    assert!(authorize_standalone("scp_transfer_list", &read_and_transfer_policy).is_ok());
    assert!(authorize_standalone("scp_transfer_get", &read_and_transfer_policy).is_ok());
    assert!(
        authorize_standalone("remote_browser_list_children", &read_and_transfer_policy).is_ok()
    );
    assert!(authorize_standalone("remote_browser_stat_path", &read_and_transfer_policy).is_ok());
    assert_eq!(
        authorize_standalone("scp_upload", &read_and_transfer_policy)
            .unwrap_err()
            .code,
        ErrorCode::PolicyDenied
    );

    // Interactive/protected/reviewed-git operations remain unavailable even under permissive policy.
    let all_cats_permissive = StdioPolicy {
        sensitive_action_policy: "allow-local-policy".into(),
        allowed_tool_categories: vec![
            "read-only".into(),
            "runtime-control".into(),
            "config-mutation".into(),
            "file-transfer".into(),
        ],
    };
    for excluded in [
        "server_rdp_start",
        "config_sync_pull",
        "config_git_pull",
        "config_git_push",
    ] {
        assert_eq!(
            authorize_standalone(excluded, &all_cats_permissive)
                .unwrap_err()
                .code,
            ErrorCode::CapabilityUnavailable,
            "{excluded}"
        );
    }

    // Non-read-only operations fail with PolicyDenied under read-only policy.
    for op in OPERATIONS {
        if !op.read_only {
            assert_eq!(
                authorize_standalone(op.name, &read_and_transfer_policy)
                    .unwrap_err()
                    .code,
                ErrorCode::PolicyDenied,
                "{}",
                op.name
            );
        }
    }

    // Unknown policies and categories always fail closed even for read-only ops.
    for bad_policy in ["", "allow-all", "ALLOW", "allow-with-token"] {
        let pol = StdioPolicy {
            sensitive_action_policy: bad_policy.into(),
            allowed_tool_categories: vec!["read-only".into()],
        };
        assert_eq!(
            authorize_standalone("servers_list", &pol).unwrap_err().code,
            ErrorCode::PolicyDenied
        );
    }
    let bad_cat = StdioPolicy {
        sensitive_action_policy: "read-only".into(),
        allowed_tool_categories: vec!["read-only".into(), "bogus-category".into()],
    };
    assert_eq!(
        authorize_standalone("servers_list", &bad_cat)
            .unwrap_err()
            .code,
        ErrorCode::PolicyDenied
    );
}

#[test]
fn execute_standalone_runs_all_11_ops_and_6_resources_with_redaction() {
    let (tmp, _rt) = setup_runtime(KeyringHealth::Locked);
    let paths = resolve_profile_paths(&ProfileLocationInput {
        config_dir: Some(&tmp.path().join("cli_root")),
        desktop_config_dir: Some(&tmp.path().join("desktop")),
        profile: Some("agent"),
        ..Default::default()
    })
    .unwrap();
    drop(_rt);
    fs::write(
        &paths.stdio_policy_path,
        serde_json::to_string_pretty(&json!({
            "sensitiveActionPolicy": "read-only",
            "allowedToolCategories": ["read-only", "file-transfer"]
        }))
        .unwrap(),
    )
    .unwrap();
    let rt = ProfileRuntime::open_with_backend(
        &ProfileLocationInput {
            config_dir: Some(&tmp.path().join("cli_root")),
            desktop_config_dir: Some(&tmp.path().join("desktop")),
            profile: Some("agent"),
            ..Default::default()
        },
        "stdio",
        MemoryKeyringBackend::new(KeyringHealth::Locked),
    )
    .unwrap();

    let cases = [
        ("app_get_overview", json!({})),
        ("function_status_list", json!({})),
        (
            "function_status_get",
            json!({"functionId": "openvpn-connect"}),
        ),
        ("openvpn_get_status", json!({})),
        ("boundary_get_status", json!({})),
        ("servers_list", json!({})),
        ("server_get", json!({"serverId": "srv-1"})),
        ("config_get_paths", json!({})),
        ("sync_settings_get", json!({})),
        ("scp_transfer_list", json!({})),
    ];
    for (name, args) in cases {
        let op = operation(name).unwrap();
        let data = execute_standalone(&rt, "1.3.3", name, &args).unwrap();
        assert!(
            conforms(&output_schema(op), &data),
            "schema mismatch {name}: {data}"
        );
        let serialized = data.to_string();
        assert!(!serialized.contains("super-secret-mcp-token"));
        assert!(!serialized.contains("super-secret-cc-token"));
    }

    // Missing transfer / server / function IDs return NotFound.
    assert_eq!(
        execute_standalone(
            &rt,
            "1.3.3",
            "scp_transfer_get",
            &json!({"transferId": "tx-1"})
        )
        .unwrap_err()
        .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        execute_standalone(&rt, "1.3.3", "server_get", &json!({"serverId": "missing"}))
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        execute_standalone(
            &rt,
            "1.3.3",
            "function_status_get",
            &json!({"functionId": "missing"}),
        )
        .unwrap_err()
        .code,
        ErrorCode::NotFound
    );

    // All 6 resources resolve through shared registry and succeed in Slice 1.
    for uri in [
        "scp://status/functions",
        "scp://status/openvpn",
        "scp://status/boundary",
        "scp://servers/list",
        "scp://config/paths",
        "scp://status/transfers",
    ] {
        let op = resource_operation(uri).unwrap();
        let data = execute_standalone(&rt, "1.3.3", op.name, &json!({})).unwrap();
        assert!(conforms(&output_schema(op), &data), "resource {uri}");
    }
}

#[test]
fn schema_catalog_and_capabilities_report_reflect_slice1_availability() {
    let full = standalone_schema_catalog(None).unwrap();
    let ops = full["mcpOperations"].as_array().unwrap();
    assert_eq!(ops.len(), OPERATIONS.len());
    let servers_entry = ops.iter().find(|o| o["name"] == "servers_list").unwrap();
    assert_eq!(servers_entry["available"], true);
    assert!(full["commands"].as_array().unwrap().len() >= 8);

    let filtered = standalone_schema_catalog(Some("server.list")).unwrap();
    let cmds = filtered["commands"].as_array().unwrap();
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0]["command"], "server.list");

    assert!(standalone_schema_catalog(Some("unknown.cmd")).is_err());

    let profile_free = capabilities_report(None::<&ProfileRuntime<MemoryKeyringBackend>>).unwrap();
    assert_eq!(profile_free["profileOpen"], false);

    let (_tmp, rt) = setup_runtime(KeyringHealth::Locked);
    let with_profile = capabilities_report(Some(&rt)).unwrap();
    assert_eq!(with_profile["profileOpen"], true);
    assert_eq!(with_profile["keyring"]["state"], "locked");
    assert_eq!(
        with_profile["stdioPolicy"]["sensitiveActionPolicy"],
        "read-only"
    );
}

#[cfg(feature = "stdio")]
#[test]
fn stdio_transport_serves_jsonrpc_discovery_calls_and_clean_eof() {
    use scp_mcp::stdio::serve_stdio_io;
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (_tmp, rt) = setup_runtime(KeyringHealth::Locked);
    let rt = Arc::new(rt);
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    tokio_rt.block_on(async move {
        let (client_in, server_in) = tokio::io::duplex(8192);
        let (server_out, client_out) = tokio::io::duplex(8192);
        let server_task =
            tokio::spawn(async move { serve_stdio_io(rt, "1.3.3", server_in, server_out).await });

        let mut writer = client_in;
        let mut reader = BufReader::new(client_out).lines();

        writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "initialize",
                        "params": {
                            "protocolVersion": "2025-11-25",
                            "capabilities": {},
                            "clientInfo": {"name": "rust-test", "version": "1.0"}
                        }
                    })
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let init_resp: serde_json::Value =
            serde_json::from_str(&reader.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(init_resp["result"]["serverInfo"]["name"], "scpctl-stdio");

        writer
            .write_all(
                format!(
                    "{}\n{}\n",
                    json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                    json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let tools_resp: serde_json::Value =
            serde_json::from_str(&reader.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(tools_resp["result"]["tools"].as_array().unwrap().len(), 11);

        writer
            .write_all(
                format!(
                    "{}\n{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "tools/call",
                        "params": {"name": "servers_list", "arguments": {}}
                    }),
                    json!({
                        "jsonrpc": "2.0",
                        "id": 4,
                        "method": "tools/call",
                        "params": {"name": "server_upsert", "arguments": {
                            "server": {
                                "id": "x", "name": "x", "host": "h",
                                "connectionMethod": "ssh", "sshPort": 22, "sshUser": "u"
                            }
                        }}
                    })
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut responses = Vec::with_capacity(2);
        for _ in 0..2 {
            let line = reader.next_line().await.unwrap().unwrap();
            responses.push(serde_json::from_str::<serde_json::Value>(&line).unwrap());
        }
        let ok_call = responses.iter().find(|v| v["id"] == 3).unwrap();
        assert_eq!(ok_call["result"]["isError"], false);
        assert_eq!(
            ok_call["result"]["structuredContent"]["result"][0]["id"],
            "srv-1"
        );

        let denied_call = responses.iter().find(|v| v["id"] == 4).unwrap();
        assert_eq!(denied_call["result"]["isError"], true);
        assert!(denied_call["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("POLICY_DENIED"));

        drop(writer);
        let code = server_task.await.unwrap();
        assert_eq!(code, 0);
    });
}

#[test]
fn slice3_cli_schemas_and_capabilities_are_complete() {
    let full = standalone_schema_catalog(None).unwrap();
    let cmds = full["commands"].as_array().unwrap();
    assert_eq!(cmds.len(), 31);

    let ops = full["mcpOperations"].as_array().unwrap();
    assert_eq!(ops.len(), 45);
    assert_eq!(ops.iter().filter(|o| o["available"] == true).count(), 41);
    assert_eq!(ops.iter().filter(|o| o["available"] == false).count(), 4);

    let resources = full["mcpResources"].as_array().unwrap();
    assert_eq!(
        resources.iter().filter(|r| r["available"] == true).count(),
        6
    );

    let expected = [
        ("completion", false, false, false, true),
        ("server.upsert", true, false, true, true),
        ("server.delete", true, true, true, false),
        ("server.credentials.set", true, false, false, true),
        ("server.credentials.clear", true, true, false, true),
        ("server.rdp", true, false, false, false),
        ("config.save", true, false, true, true),
        ("config.patch", true, false, true, true),
        ("config.secret.set", true, false, false, true),
        ("config.secret.clear", true, true, false, true),
        ("config.sync.push", true, false, true, true),
        ("settings.patch", true, false, true, true),
        ("settings.export", true, false, false, false),
        ("settings.import", true, true, true, false),
    ];

    for (name, profile_required, destructive, preview_supported, idempotent) in expected {
        let one = standalone_schema_catalog(Some(name)).unwrap();
        let arr = one["commands"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "missing {name}");
        let entry = &arr[0];
        assert_eq!(entry["command"], name);
        assert_eq!(entry["profileRequired"], profile_required, "{name}");
        assert_eq!(entry["available"], true, "{name}");
        assert_eq!(entry["destructive"], destructive, "{name}");
        assert_eq!(entry["previewSupported"], preview_supported, "{name}");
        assert_eq!(entry["idempotent"], idempotent, "{name}");
        let args = entry["arguments"].as_object().unwrap();
        for forbidden in ["password", "secret", "passphrase", "recoverIdentity"] {
            assert!(
                !args.contains_key(forbidden),
                "{name} must not advertise unsafe arg {forbidden}"
            );
        }
    }

    let caps = capabilities_report(None::<&ProfileRuntime<MemoryKeyringBackend>>).unwrap();
    assert_eq!(caps["runtimeFamilies"]["config"], "cli_local");
    assert_eq!(caps["runtimeFamilies"]["backup"], "cli_local");
    assert_eq!(caps["runtimeFamilies"]["rdp"], "cli_interactive_gui");
    assert_eq!(caps["runtimeFamilies"]["sftp"], "stdio_persistent_browser");
    assert_eq!(caps["unavailableFamilies"], json!({}));
}
