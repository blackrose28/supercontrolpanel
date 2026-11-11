use super::validate_mcp_tool_policy;
use scp_core::mcp_policy::McpPolicyError;

fn categories(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn all_desktop_operations_preserve_categories_and_explicit_sensitivity() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("mcp_tools.json")).unwrap();
    let sensitive = [
        "openvpn_start",
        "openvpn_stop",
        "boundary_auth_start",
        "server_delete",
        "remote_browser_delete_path",
        "scp_upload",
        "scp_download",
        "scp_start_upload",
        "scp_start_download",
        "config_sync_push",
        "config_git_push",
    ];
    let known_categories = [
        "read-only",
        "runtime-control",
        "config-mutation",
        "file-transfer",
    ];
    let tools = fixture.as_array().unwrap();
    assert_eq!(tools.len(), 40);
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let category = tool["category"].as_str().unwrap();
        assert_eq!(
            validate_mcp_tool_policy(name, &categories(&[category]), "allow-with-token"),
            Ok(()),
            "{name}"
        );
        let expected = if sensitive.contains(&name) {
            Err(McpPolicyError::ApprovalRequired)
        } else {
            Ok(())
        };
        assert_eq!(
            validate_mcp_tool_policy(name, &categories(&[category]), "require-ui-approval"),
            expected,
            "{name}"
        );
        for other in known_categories {
            if other != category {
                assert_eq!(
                    validate_mcp_tool_policy(name, &categories(&[other]), "allow-with-token"),
                    Err(McpPolicyError::CategoryDisabled),
                    "{name}: {other}"
                );
            }
        }
        assert_eq!(
            validate_mcp_tool_policy(name, &[], "allow-with-token"),
            Err(McpPolicyError::CategoryDisabled)
        );
    }
}

#[test]
fn unknown_operations_fail_closed() {
    for name in [
        "",
        "unknown",
        "servers_list ",
        "Servers_list",
        "secret\noperation",
    ] {
        assert_eq!(
            validate_mcp_tool_policy(name, &categories(&["read-only"]), "allow-with-token"),
            Err(McpPolicyError::UnknownOperation)
        );
    }
}

#[test]
fn unknown_policies_fail_closed() {
    // Invalid policies must block even read-only operations and every known category.
    for tool in [
        "servers_list",
        "caddy_start",
        "config_save",
        "scp_transfer_list",
    ] {
        for policy in [
            "",
            " ",
            "unknown",
            "ALLOW-WITH-TOKEN",
            "allow-with-token ",
            "secret\npolicy",
        ] {
            assert_eq!(
                validate_mcp_tool_policy(
                    tool,
                    &categories(&[
                        "read-only",
                        "runtime-control",
                        "config-mutation",
                        "file-transfer"
                    ]),
                    policy
                ),
                Err(McpPolicyError::UnknownPolicy)
            );
        }
    }
}

#[test]
fn unknown_categories_fail_closed() {
    for category in [
        "",
        " ",
        "unknown",
        "Read-only",
        "read-only ",
        "secret\ncategory",
    ] {
        // One legitimate category cannot mask an unknown setting.
        assert_eq!(
            validate_mcp_tool_policy(
                "servers_list",
                &categories(&["read-only", category]),
                "allow-with-token"
            ),
            Err(McpPolicyError::UnknownCategory)
        );
    }
}

#[test]
fn known_categories_are_exact_sets_and_approval_never_bypasses_them() {
    let duplicate = categories(&["read-only", "read-only"]);
    assert_eq!(
        validate_mcp_tool_policy("servers_list", &duplicate, "require-ui-approval"),
        Ok(())
    );
    assert_eq!(
        validate_mcp_tool_policy("openvpn_start", &duplicate, "require-ui-approval"),
        Err(McpPolicyError::CategoryDisabled)
    );
    // Mutating runtime controls are not implicitly sensitive; readOnly is discovery only.
    assert_eq!(
        validate_mcp_tool_policy(
            "caddy_start",
            &categories(&["runtime-control"]),
            "require-ui-approval"
        ),
        Ok(())
    );
    assert_eq!(
        validate_mcp_tool_policy(
            "scp_transfer_list",
            &categories(&["read-only"]),
            "allow-with-token"
        ),
        Err(McpPolicyError::CategoryDisabled)
    );
}

#[test]
fn policy_errors_have_fixed_transport_independent_diagnostics() {
    for (error, message) in [
        (McpPolicyError::UnknownOperation, "Unknown MCP operation"),
        (McpPolicyError::UnknownPolicy, "Unknown MCP sensitive-action policy; use desktop settings to select a supported policy"),
        (McpPolicyError::UnknownCategory, "Unknown MCP tool category in settings; use desktop settings to select supported categories"),
        (McpPolicyError::CategoryDisabled, "MCP tool category is disabled by settings"),
        (McpPolicyError::ApprovalRequired, "Sensitive MCP tool requires UI approval; no approval workflow is available"),
    ] {
        assert_eq!(error.to_string(), message);
        let _: &dyn std::error::Error = &error;
    }
}
