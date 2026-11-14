//! Registry-driven SDK adapter. The desktop remains the runtime owner.
use super::{
    contracts::{operation, ErrorCode, Operation, OperationError},
    lifecycle::{CancellationToken, TransportControl},
};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Request},
    http::{Method, StatusCode, Uri},
    middleware::{self, Next},
    response::Response,
    Json, Router,
};
use rmcp::{
    model::{ServerConfig, SubscriptionFilter},
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ServerHandler,
};
use scp_mcp::protocol::{DiscoveryHandler, HostKind, OperationHandler, SdkHost};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

type LegacyHandler = dyn Fn(&str, &str, &[u8]) -> (u16, Value) + Send + Sync;

#[derive(Clone)]
pub(super) struct ProtocolHost {
    pub control: TransportControl,
    pub subscriptions: Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub permitted: Arc<DiscoveryHandler>,
    pub execute: Arc<OperationHandler>,
    pub legacy: Arc<LegacyHandler>,
}

impl ProtocolHost {
    fn to_sdk_host(&self) -> SdkHost {
        SdkHost {
            kind: HostKind::Desktop {
                version: env!("CARGO_PKG_VERSION"),
            },
            control: self.control.clone(),
            subscriptions: self.subscriptions.clone(),
            permitted: self.permitted.clone(),
            execute: self.execute.clone(),
        }
    }

    #[cfg(test)]
    fn get_info(&self) -> ServerConfig {
        self.to_sdk_host().get_info()
    }

    #[cfg(test)]
    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        self.to_sdk_host().accepted_subscription_filter(requested)
    }
}

fn operation_deadline(op: Option<&Operation>) -> Duration {
    match op.map(|op| op.lifetime) {
        Some(super::contracts::Lifetime::Request) => Duration::from_secs(30 * 60),
        Some(super::contracts::Lifetime::Query) | None => Duration::from_secs(30),
        _ => Duration::from_secs(5 * 60),
    }
}

pub(super) fn protocol_router(
    host: ProtocolHost,
    port: u16,
    token: String,
    json_response: bool,
) -> Router {
    let authorities = [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ];
    let config = StreamableHttpServerConfig::default()
        .with_cancellation_token(host.control.stop.clone())
        .with_legacy_session_mode(true)
        .with_json_response(json_response)
        .with_allowed_hosts(authorities.clone())
        .enforce_origin_validation()
        .with_max_request_body_bytes(1024 * 1024);
    let sdk_host = host.to_sdk_host();
    let service = StreamableHttpService::new(
        move || {
            let mut session = sdk_host.clone();
            session.subscriptions = Default::default();
            Ok(session)
        },
        LocalSessionManager::default().into(),
        config,
    );
    let stop = host.control.stop.clone();
    Router::new()
        .route_service("/mcp", service)
        .fallback(move |method: Method, uri: Uri, body: Bytes| {
            let legacy = host.legacy.clone();
            let control = host.control.clone();
            async move {
                let op = if uri.path() == "/mcp/tools/call" {
                    serde_json::from_slice::<Value>(&body).ok().and_then(|body| body.get("toolName").and_then(Value::as_str).and_then(operation))
                } else { None };
                let result = tokio::time::timeout(operation_deadline(op), control.run(CancellationToken::new(), move || {
                    Ok(legacy(method.as_str(), uri.path(), &body))
                })).await.unwrap_or_else(|_| Err(OperationError::new(ErrorCode::Timeout)));
                match result {
                    Ok((status, value)) => (
                        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                        Json(value),
                    ),
                    Err(error) => (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({"ok":false,"message":error.legacy_message(),"data":{},"warnings":[],"timestamp":crate::services::now_ms()})),
                    ),
                }
            }
        })
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let token = token.clone();
            let stop = stop.clone();
            let authorities = authorities.clone();
            async move {
                if stop.is_cancelled() { return Err((StatusCode::SERVICE_UNAVAILABLE, Json(json!({"message":"MCP listener stopped"})))); }
                if request
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    != Some(format!("Bearer {token}").as_str())
                {
                    return Err((StatusCode::UNAUTHORIZED, Json(json!({"ok":false,"message":"Unauthorized","data":{},"warnings":[],"timestamp":crate::services::now_ms()}))));
                }
                if request.uri().authority().is_some_and(|authority| !authorities.iter().any(|allowed| allowed.eq_ignore_ascii_case(authority.as_str())))
                    || request.headers().contains_key("origin")
                    || !request
                        .headers()
                        .get("host")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|host| {
                            authorities.iter().any(|a| a.eq_ignore_ascii_case(host))
                        })
                {
                    return Err((StatusCode::FORBIDDEN, Json(json!({"ok":false,"message":"Forbidden","data":{},"warnings":[],"timestamp":crate::services::now_ms()}))));
                }
                let request = receive_body(request).await?;
                Ok::<Response, (StatusCode, Json<Value>)>(next.run(request).await)
            }
        }))
}

async fn receive_body(request: Request) -> Result<Request, (StatusCode, Json<Value>)> {
    let (parts, body) = request.into_parts();
    let body = match tokio::time::timeout(
        Duration::from_secs(10),
        axum::body::to_bytes(body, 1024 * 1024),
    )
    .await
    {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({"message":"Request body too large"})),
            ))
        }
        Err(_) => {
            return Err((
                StatusCode::REQUEST_TIMEOUT,
                Json(json!({"message":"Request body timeout"})),
            ))
        }
    };
    let request = Request::from_parts(parts, axum::body::Body::from(body));
    Ok(request)
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
