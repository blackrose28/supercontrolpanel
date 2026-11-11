use super::*;
use serde_json::json;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

#[tokio::test]
async fn official_clients_and_legacy_routes() {
    check_clients(true).await;
}

#[tokio::test]
async fn official_clients_sse_and_legacy_routes() {
    check_clients(false).await;
}

fn test_policy(mode: u8) -> (&'static str, &'static [&'static str]) {
    let policy = if mode == 1 {
        "require-ui-approval"
    } else {
        "allow-with-token"
    };
    let categories: &[&str] = if mode == 2 {
        &[]
    } else {
        &[
            "read-only",
            "runtime-control",
            "config-mutation",
            "file-transfer",
        ]
    };
    (policy, categories)
}

fn fixture_host() -> ProtocolHost {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../experiments/mcp-http-slice0/tests/desktop-surface.json"
    ))
    .unwrap();
    assert_eq!(
        fixtures.as_object().unwrap().len(),
        super::super::contracts::OPERATIONS.len()
    );
    let legacy_statuses = fixtures["function_status_list"]["data"].clone();
    let permitted = Arc::new(AtomicU8::new(0));
    let policy = permitted.clone();
    let execution_policy = permitted.clone();
    ProtocolHost {
        permitted: Arc::new(move || {
            Ok(super::super::contracts::OPERATIONS
                .iter()
                .filter(|op| {
                    let (policy, categories) = test_policy(policy.load(Ordering::SeqCst));
                    super::super::contracts::authorize(op.name, policy, categories).is_ok()
                })
                .map(|op| op.name.to_string())
                .collect())
        }),
        execute: Arc::new(move |name, args| {
            let mode = execution_policy.load(Ordering::SeqCst);
            let (policy, categories) = test_policy(mode);
            super::super::contracts::execute(name, policy, categories, &args, |op| {
                assert_eq!(
                    args, fixtures[op.name]["args"],
                    "adapter must preserve arguments for {}",
                    op.name
                );
                if mode == 3 {
                    return Err("keyring/remote error: unlabelled-secret".into());
                }
                if mode == 4 {
                    return Ok(json!({"password":"unlabelled-secret"}));
                }
                Ok(fixtures[op.name]["data"].clone())
            })
        }),
        legacy: Arc::new(move |method, path, _| {
            if method == "POST" {
                let mode = match path {
                    "/test/approval" => 1,
                    "/test/deny" => 2,
                    "/test/failure" => 3,
                    "/test/invalid-output" => 4,
                    _ => 0,
                };
                permitted.store(mode, Ordering::SeqCst);
            }
            (
                200,
                json!({"ok":true,"message":"MCP operation completed","data":legacy_statuses,"warnings":[],"timestamp":123}),
            )
        }),
    }
}

async fn check_clients(json_response: bool) {
    let host = fixture_host();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = protocol_router(host, address.port(), "test-only".into(), json_response);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        tokio::process::Command::new("node")
            .kill_on_drop(true)
            .arg("../experiments/mcp-http-slice0/tests/desktop.test.mjs")
            .env("SLICE2_URL", format!("http://{address}"))
            .env(
                "SLICE2_HELPER",
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join(format!(
                        "supercontrolpanelhelper{}",
                        std::env::consts::EXE_SUFFIX
                    )),
            )
            .env("SLICE2_MODE", if json_response { "json" } else { "sse" })
            .output(),
    )
    .await
    .expect("official client integration timed out")
    .unwrap();
    server.abort();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
