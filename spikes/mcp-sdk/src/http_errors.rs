//! Isolated HTTP-status error boundary, not a complete MCP or logging redactor.
use axum::{
    extract::Request,
    http::header::ALLOW,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};

/// Discard HTTP 4xx/5xx bodies/headers, retaining status and known SDK Allow values.
///
/// Mount outside admission/auth middleware so their statuses are also preserved.
/// No request or response body is read here: successful JSON, SSE and notification
/// responses pass through unchanged. In particular, HTTP 200 JSON-RPC errors are
/// outside this boundary, as are SDK tracing, panics and successful result values.
/// No raw error/body/header values are logged by this layer. Production adoption
/// still requires separate protocol-error and logging policies.
pub fn with_fixed_http_errors(router: Router) -> Router {
    router.layer(middleware::from_fn(fixed_http_errors))
}

async fn fixed_http_errors(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        let mut fixed = (status, "MCP HTTP request failed").into_response();
        // Preserve required method guidance without forwarding arbitrary header text.
        if let Some(allowed) = response.headers().get(ALLOW).filter(|value| {
            matches!(
                value.as_bytes(),
                b"POST" | b"GET, POST" | b"GET, POST, DELETE"
            )
        }) {
            fixed.headers_mut().insert(ALLOW, allowed.clone());
        }
        return fixed;
    }
    response
}
