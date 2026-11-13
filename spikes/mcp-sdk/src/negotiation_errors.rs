//! Opt-in mapping for the pinned SDK's finite HTTP 400 JSON negotiation errors.
use axum::{
    body::{to_bytes, HttpBody},
    extract::Request,
    http::{header::CONTENT_TYPE, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use rmcp::{model::JsonRpcError, ErrorData};
use serde_json::Value;

const MAX_ERROR_BYTES: u64 = 65536;

/// Mount immediately outside the SDK route, without `with_fixed_http_errors`.
/// Only exact-size JSON HTTP 400 bodies up to 64 KiB are inspected. Other
/// statuses (including HTTP 200 errors and SSE), unknown sizes, and larger bodies
/// pass untouched. Selected codes retain their ID/status/code with fixed text and
/// no error data. Unknown envelopes/codes retain their bytes and headers.
/// This does not prevent SDK logging or sanitize IDs, arbitrary headers, native
/// failures, parser errors, or successful data. It is not a production boundary.
pub fn with_fixed_negotiation_errors(router: Router) -> Router {
    router.layer(middleware::from_fn(fixed_negotiation_errors))
}

async fn fixed_negotiation_errors(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    if response.status() != StatusCode::BAD_REQUEST
        || response.headers().get(CONTENT_TYPE).map(|v| v.as_bytes()) != Some(b"application/json")
        || !matches!(response.body().size_hint().exact(), Some(size) if size <= MAX_ERROR_BYTES)
    {
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = match to_bytes(body, MAX_ERROR_BYTES as usize).await {
        Ok(bytes) => bytes,
        // Not emitted by the SDK's Infallible Full JSON bodies. Do not format a
        // host/body error if this experiment is incorrectly applied elsewhere.
        Err(_) => return (parts.status, "MCP negotiation response unavailable").into_response(),
    };
    if let Some(error) = fixed_error(&bytes) {
        return (
            parts.status,
            [(CONTENT_TYPE, "application/json")],
            serde_json::to_vec(&error).expect("JSON-RPC error serializes"),
        )
            .into_response();
    }
    Response::from_parts(parts, bytes.into())
}

fn fixed_error(bytes: &[u8]) -> Option<Value> {
    let original: Value = serde_json::from_slice(bytes).ok()?;
    let error: JsonRpcError = serde_json::from_value(original.clone()).ok()?;
    if !matches!(error.error.code.0, -32020 | -32022 | -32600 | -32602) {
        return None;
    }
    let mut fixed = serde_json::to_value(JsonRpcError::new(
        error.id,
        ErrorData::new(error.error.code, "MCP request negotiation failed", None),
    ))
    .expect("JSON-RPC error serializes");
    // SDK Option<RequestId> omits None on serialization; preserve an explicit
    // native null ID as well as the absence of an ID, without reinterpreting it.
    if let Some(id) = original.get("id") {
        fixed["id"] = id.clone();
    }
    Some(fixed)
}
