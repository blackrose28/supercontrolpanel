use serde_json::Value;
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scpctl"))
        .args(args)
        .env(
            "SCPCTL_CONFIG_DIR",
            "/nonexistent/scpctl-test-must-not-open",
        )
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn schema_is_one_json_envelope_without_profile_or_desktop() {
    let result = run(&["schema", "--json"]);
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    assert_eq!(result.stdout.iter().filter(|b| **b == b'\n').count(), 1);
    let result: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result["schemaVersion"], 1);
    assert_eq!(result["ok"], true);
    assert!(result["runtimeId"].is_null());
    assert!(result["error"].is_null());
    assert_eq!(result["data"]["commands"][0]["command"], "schema");
    let ops = result["data"]["mcpOperations"].as_array().unwrap();
    assert_eq!(ops.len(), 45);
    assert_eq!(ops.iter().filter(|op| op["available"] == true).count(), 11);
    assert_eq!(ops.iter().filter(|op| op["available"] == false).count(), 34);
    let resources = result["data"]["mcpResources"].as_array().unwrap();
    assert_eq!(resources.len(), 6);
    assert!(resources.iter().all(|r| r["available"] == true));
}

#[test]
fn unsupported_commands_fail_without_claiming_runtime_access() {
    for args in [
        vec!["unknown-cmd", "--json"],
        vec!["schema", "--json", "--interactive"],
        vec!["mcp", "serve", "--stdio", "--json"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        if args[0] == "mcp" {
            assert!(
                output.stdout.is_empty(),
                "stdio startup must never emit a shell envelope"
            );
        } else {
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"]["code"], "INVALID_ARGUMENTS");
            assert_eq!(result["error"]["outcome"], "not_started");
        }
    }
}

#[test]
fn help_and_version_do_not_open_profiles() {
    for arg in ["--help", "--version"] {
        let output = run(&[arg]);
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}
