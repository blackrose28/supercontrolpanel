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

fn surface_fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../../../experiments/mcp-http-slice0/tests/desktop-surface.json"
    ))
    .unwrap()
}

fn fixture_result(name: &str, args: &Value, mode: u8, fixtures: &Value) -> Result<Value, String> {
    assert_eq!(
        *args, fixtures[name]["args"],
        "adapter must preserve arguments for {}",
        name
    );
    if mode == 3 {
        return Err("keyring/remote error: unlabelled-secret".into());
    }
    if mode == 4 {
        return Ok(json!({"password":"unlabelled-secret"}));
    }
    Ok(fixtures[name]["data"].clone())
}

fn fixture_host() -> ProtocolHost {
    let fixtures = surface_fixtures();
    assert_eq!(
        fixtures.as_object().unwrap().len(),
        super::super::contracts::OPERATIONS.len()
    );
    let legacy_statuses = fixtures["function_status_list"]["data"].clone();
    let permitted = Arc::new(AtomicU8::new(0));
    let policy = permitted.clone();
    let execution_policy = permitted.clone();
    let control = TransportControl::default();
    ProtocolHost {
        subscriptions: Default::default(),
        control: control.clone(),
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
                fixture_result(op.name, &args, mode, &fixtures)
            })
        }),
        legacy: Arc::new(move |method, path, _| {
            if path == "/test/event" {
                control.invalidate();
            }
            if path == "/test/stop" {
                control.stop.cancel();
            }
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

#[tokio::test]
async fn snapshot_invalidations_expose_no_event_payloads() {
    let host = fixture_host();
    let info = host.get_info();
    let capabilities = serde_json::to_value(info.capabilities).unwrap();
    assert_eq!(capabilities["resources"]["subscribe"], true);
    let requested: SubscriptionFilter = serde_json::from_value(json!({
        "resourceSubscriptions": ["scp://status/functions", "scp://status/transfers", "secret://unknown"]
    })).unwrap();
    let accepted = host.accepted_subscription_filter(&requested).unwrap();
    assert_eq!(
        accepted.resource_subscriptions.unwrap(),
        vec!["scp://status/functions", "scp://status/transfers"]
    );
}

#[tokio::test]
async fn revoked_router_rejects_old_authority_and_rotation_rebinds_port() {
    let host = fixture_host();
    let control = host.control.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let address = listener.local_addr().unwrap();
    let router = protocol_router(host, address.port(), "old".into(), true);
    let serve_control = control.clone();
    let server = tokio::spawn(super::super::lifecycle::serve_transport(
        listener,
        router,
        serve_control,
    ));
    let client = reqwest::Client::new();
    let url = format!("http://{address}/health");
    assert_eq!(
        client
            .get(&url)
            .bearer_auth("old")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    control.stop.cancel();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    let listener = std::net::TcpListener::bind(address).unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    let router = protocol_router(fixture_host(), address.port(), "new".into(), true);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    assert_eq!(
        client
            .get(&url)
            .bearer_auth("old")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(&url)
            .bearer_auth("new")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    server.abort();
}

#[tokio::test]
async fn slow_body_receive_times_out_before_dispatch() {
    let body = axum::body::Body::from_stream(futures_util::stream::pending::<
        Result<Bytes, std::io::Error>,
    >());
    let request = Request::new(body);
    let result = tokio::time::timeout(Duration::from_secs(12), receive_body(request))
        .await
        .unwrap();
    assert_eq!(result.unwrap_err().0, StatusCode::REQUEST_TIMEOUT);
}
