//! Synthetic malformed-input evidence; the original raw SDK fixtures stay unchanged.
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use scp_mcp_sdk_spike::{fixture_router, http_errors::with_fixed_http_errors};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
#[path = "support/server.rs"]
mod server;
use server::BoundServer;

const MARKER: &str = "SYNTHETIC_SECRET_MARKER";
const VERSIONS: [&str; 2] = ["2026-07-28", "2025-11-25"];

fn message(version: &str, method: &str, mut params: Value) -> Value {
    if version == VERSIONS[0] {
        params["_meta"] =
            serde_json::to_value(rmcp::model::RequestMetaObject::with_client_context(
                rmcp::model::ProtocolVersion::V_2026_07_28,
                rmcp::model::Implementation::new("boundary-probe", "0"),
                Default::default(),
            ))
            .unwrap();
    }
    json!({"jsonrpc":"2.0","id":7,"method":method,"params":params})
}
fn malformed_cases(version: &str) -> Vec<(&'static str, String)> {
    vec![
        ("syntax", format!("{{\"{MARKER}\":")),
        ("scalar", json!(MARKER).to_string()),
        ("batch", json!([MARKER]).to_string()),
        ("version", json!({"jsonrpc":MARKER,"id":7,"method":"tools/list"}).to_string()),
        ("id", json!({"jsonrpc":"2.0","id":{"secret":MARKER},"method":"tools/list"}).to_string()),
        ("method_type", json!({"jsonrpc":"2.0","id":7,"method":{"secret":MARKER}}).to_string()),
        ("arguments_type", message(version,"tools/call",json!({"name":"spike_echo","arguments":MARKER})).to_string()),
        ("null_arguments", message(version,"tools/call",json!({"name":"spike_echo","arguments":null,"secret":MARKER})).to_string()),
        ("unknown_method", message(version,MARKER,json!({})).to_string()),
        ("unknown_variant", json!({"jsonrpc":"2.0","id":7,"result":{"action":MARKER}}).to_string()),
        ("init_type", message(version,"initialize",json!({"protocolVersion":version,"capabilities":MARKER,"clientInfo":{"name":"probe","version":"0"}})).to_string()),
    ]
}
async fn start(fixed_errors: bool) -> BoundServer {
    let stop = CancellationToken::new();
    let mut config = StreamableHttpServerConfig::default().enforce_origin_validation();
    config.json_response = true;
    config.legacy_session_mode = false;
    config.max_request_body_bytes = 4096;
    config.cancellation_token = stop.child_token();
    let router = fixture_router(config);
    let router = if fixed_errors {
        with_fixed_http_errors(router)
    } else {
        router
    };
    BoundServer::bind(router, stop).await
}
fn post(server: &BoundServer, version: &str, method: &str) -> reqwest::RequestBuilder {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in [
        ("authorization", "Bearer spike-only-token"),
        ("accept", "application/json, text/event-stream"),
        ("content-type", "application/json"),
        ("mcp-protocol-version", version),
        ("mcp-method", method),
        ("mcp-name", "spike_echo"),
    ] {
        headers.insert(
            reqwest::header::HeaderName::from_static(name),
            value.parse().unwrap(),
        );
    }
    reqwest::Client::new()
        .post(&server.url)
        .headers(headers)
        .timeout(Duration::from_secs(5))
}
fn expected(label: &str, modern: bool) -> (u16, Option<i64>, bool) {
    match label {
        "syntax" | "scalar" | "batch" | "version" | "id" | "method_type" => (415, None, false),
        "arguments_type" | "init_type" => (if modern { 404 } else { 200 }, Some(-32601), false),
        "null_arguments" => (200, Some(-32602), false),
        "unknown_method" => (if modern { 404 } else { 200 }, Some(-32601), true),
        "unknown_variant" => (400, Some(-32600), false),
        _ => unreachable!(),
    }
}
async fn check_inputs(fixed_errors: bool) {
    let server = start(fixed_errors).await;
    for version in VERSIONS {
        for (label, body) in malformed_cases(version) {
            let method = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["method"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "tools/list".into());
            let response = post(&server, version, &method)
                .body(body)
                .send()
                .await
                .unwrap();
            let (status, code, reflected) = expected(label, version == VERSIONS[0]);
            assert_eq!(response.status(), status, "{version} {label}");
            let text = response.text().await.unwrap();
            if fixed_errors && status >= 400 {
                assert_eq!(text, "MCP HTTP request failed", "{version} {label}");
            } else {
                let parsed = serde_json::from_str::<Value>(&text).ok();
                assert_eq!(
                    parsed.as_ref().and_then(|v| v["error"]["code"].as_i64()),
                    code,
                    "{version} {label}"
                );
                assert_eq!(text.contains(MARKER), reflected, "{version} {label}");
                if status == 415 {
                    assert!(text.starts_with("fail to deserialize request body"));
                }
            }
        }
    }
    server.finish().await;
}
#[tokio::test]
async fn native_secret_input_characterization() {
    check_inputs(false).await;
}
#[tokio::test]
async fn fixed_http_errors_preserve_status_without_native_text() {
    check_inputs(true).await;
}

#[tokio::test]
async fn admission_statuses_survive_the_boundary() {
    let server = start(true).await;
    for (header, value, status) in [
        ("Authorization", "Bearer wrong", 401),
        ("Origin", "https://evil.example", 403),
        ("Host", "evil.example", 403),
        ("Accept", "text/plain", 406),
        ("Content-Type", "text/plain", 415),
    ] {
        let mut request = post(&server, VERSIONS[0], "tools/list")
            .body("{".repeat(4097))
            .build()
            .unwrap();
        request.headers_mut().insert(
            reqwest::header::HeaderName::from_bytes(header.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        let response = reqwest::Client::new().execute(request).await.unwrap();
        assert_eq!(response.status(), status, "{header}");
        assert_eq!(response.text().await.unwrap(), "MCP HTTP request failed");
    }
    let response = post(&server, VERSIONS[0], "tools/list")
        .body("{".repeat(4097))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    assert_eq!(response.text().await.unwrap(), "MCP HTTP request failed");
    let response = reqwest::Client::new()
        .get(&server.url)
        .bearer_auth("spike-only-token")
        .header("MCP-Protocol-Version", VERSIONS[0])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 405);
    assert_eq!(response.headers()["allow"], "POST");
    assert_eq!(response.text().await.unwrap(), "MCP HTTP request failed");
    server.finish().await;
}

async fn synthetic_response(
    status: axum::http::StatusCode,
    headers: Vec<(&'static str, &'static str)>,
    body: &'static str,
) -> (BoundServer, reqwest::Response) {
    use axum::{routing::any, Router};
    let headers: axum::http::HeaderMap = headers
        .into_iter()
        .map(|(name, value)| {
            (
                axum::http::HeaderName::from_static(name),
                value.parse().unwrap(),
            )
        })
        .collect();
    let router = Router::new().route(
        "/mcp",
        any(move || {
            let headers = headers.clone();
            async move { (status, headers, body) }
        }),
    );
    let server = BoundServer::bind(with_fixed_http_errors(router), CancellationToken::new()).await;
    let response = reqwest::get(&server.url).await.unwrap();
    (server, response)
}

#[tokio::test]
async fn native_error_response_body_and_headers_are_discarded() {
    let (server, response) = synthetic_response(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        vec![("x-native-error", MARKER), ("allow", MARKER)],
        MARKER,
    )
    .await;
    assert_eq!(response.status(), 500);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    assert!(response.headers().get("x-native-error").is_none());
    assert!(response.headers().get("allow").is_none());
    assert_eq!(response.text().await.unwrap(), "MCP HTTP request failed");
    server.finish().await;
}

#[tokio::test]
async fn successful_http_responses_are_unchanged_including_protocol_errors() {
    use axum::http::StatusCode;
    for status in [StatusCode::OK, StatusCode::ACCEPTED, StatusCode::NO_CONTENT] {
        let body = if status == StatusCode::NO_CONTENT {
            ""
        } else {
            MARKER
        };
        let (server, response) =
            synthetic_response(status, vec![("x-success", "preserved")], body).await;
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["x-success"], "preserved");
        assert_eq!(response.text().await.unwrap(), body);
        server.finish().await;
    }
    // check_inputs(true) additionally proves legacy HTTP 200 JSON-RPC errors are unchanged.
}
