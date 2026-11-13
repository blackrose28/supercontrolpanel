//! Scope/exclusion and composition evidence for the opt-in negotiation mapper.
use axum::{
    body::Body,
    http::{header::CONTENT_TYPE, Response, StatusCode},
    routing::post,
    Router,
};
use scp_mcp_sdk_spike::{
    http_errors::with_fixed_http_errors, negotiation_errors::with_fixed_negotiation_errors,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
#[path = "support/server.rs"]
mod server;
use server::BoundServer;

async fn synthetic(
    status: u16,
    content_type: &'static str,
    body: String,
    hosting: fn(Router) -> Router,
) -> (BoundServer, reqwest::Response) {
    let router = Router::new().route(
        "/mcp",
        post(move || {
            let body = body.clone();
            async move {
                Response::builder()
                    .status(status)
                    .header(CONTENT_TYPE, content_type)
                    .header("x-original", "synthetic-diagnostic")
                    .body(Body::from(body))
                    .unwrap()
            }
        }),
    );
    let server = BoundServer::bind(hosting(router), CancellationToken::new()).await;
    let response = reqwest::Client::new()
        .post(&server.url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .unwrap();
    (server, response)
}

#[tokio::test]
async fn selected_envelopes_preserve_id_code_and_discard_diagnostics() {
    for id in [
        Some(json!(42)),
        Some(json!("caller-correlation")),
        Some(Value::Null),
        None,
    ] {
        for code in [-32020, -32022, -32600, -32602] {
            let mut native = json!({"jsonrpc":"2.0","error":{"code":code,"message":"SYNTHETIC_SECRET","data":{"secret":"SYNTHETIC_SECRET"}}});
            let mut expected = json!({"jsonrpc":"2.0","error":{"code":code,"message":"MCP request negotiation failed"}});
            if let Some(id) = &id {
                native["id"] = id.clone();
                expected["id"] = id.clone();
            }
            let (server, response) = synthetic(
                400,
                "application/json",
                native.to_string(),
                with_fixed_negotiation_errors,
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
            assert!(response.headers().get("x-original").is_none());
            assert_eq!(response.json::<Value>().await.unwrap(), expected);
            server.finish().await;
        }
    }
}

#[tokio::test]
async fn excluded_responses_keep_original_bytes_and_headers() {
    let envelope =
        json!({"jsonrpc":"2.0","id":42,"error":{"code":-32020,"message":"original"}}).to_string();
    let oversized =
        json!({"jsonrpc":"2.0","id":42,"error":{"code":-32020,"message":"x".repeat(65536)}})
            .to_string();
    for (status, content_type, body) in [
        (200, "application/json", envelope.clone()),
        (404, "application/json", envelope.clone()),
        (500, "application/json", envelope.clone()),
        (400, "text/plain", envelope),
        (400, "application/json", oversized),
        (400, "application/json", "malformed".into()),
        (
            400,
            "application/json",
            json!({"jsonrpc":"2.0","id":42,"error":{"code":-32603,"message":"original"}})
                .to_string(),
        ),
    ] {
        let (server, response) = synthetic(
            status,
            content_type,
            body.clone(),
            with_fixed_negotiation_errors,
        )
        .await;
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.headers()[CONTENT_TYPE], content_type);
        assert_eq!(response.headers()["x-original"], "synthetic-diagnostic");
        assert_eq!(response.text().await.unwrap(), body);
        server.finish().await;
    }
}

fn http_outer(router: Router) -> Router {
    with_fixed_http_errors(with_fixed_negotiation_errors(router))
}
fn negotiation_outer(router: Router) -> Router {
    with_fixed_negotiation_errors(with_fixed_http_errors(router))
}

#[tokio::test]
async fn experimental_http_wrapper_still_erases_protocol_envelope_in_both_orders() {
    for hosting in [http_outer as fn(Router) -> Router, negotiation_outer] {
        let native =
            json!({"jsonrpc":"2.0","id":42,"error":{"code":-32020,"message":"SYNTHETIC_SECRET"}});
        let (server, response) =
            synthetic(400, "application/json", native.to_string(), hosting).await;
        assert_eq!(response.status(), 400);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
        assert_eq!(response.text().await.unwrap(), "MCP HTTP request failed");
        server.finish().await;
    }
}
