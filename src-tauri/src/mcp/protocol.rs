//! Registry-driven SDK adapter. The desktop remains the runtime owner.
use super::{
    contracts::{operation, resource_operation, ErrorCode, Operation, OperationError},
    validation,
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
    model::*,
    service::RequestContext,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, RoleServer, ServerHandler,
};
use serde_json::{json, Value};
use std::{borrow::Cow, sync::Arc};

type DiscoveryHandler = dyn Fn() -> Result<Vec<String>, OperationError> + Send + Sync;
type OperationHandler = dyn Fn(&str, Value) -> Result<Value, OperationError> + Send + Sync;
type LegacyHandler = dyn Fn(&str, &str, &[u8]) -> (u16, Value) + Send + Sync;

#[derive(Clone)]
pub(super) struct ProtocolHost {
    pub permitted: Arc<DiscoveryHandler>,
    pub execute: Arc<OperationHandler>,
    pub legacy: Arc<LegacyHandler>,
}

impl ProtocolHost {
    async fn invoke(&self, op: &'static Operation, args: Value) -> Result<Value, OperationError> {
        let execute = self.execute.clone();
        let data = tokio::task::spawn_blocking(move || execute(op.name, args))
            .await
            .map_err(|_| OperationError::new(ErrorCode::OperationFailed))??;
        if !validation::conforms(&validation::output_schema(op), &data) {
            return Err(OperationError::new(ErrorCode::OperationFailed));
        }
        Ok(standard_data(op, data))
    }
    async fn visible(&self) -> Result<Vec<&'static Operation>, ErrorData> {
        let permitted = self.permitted.clone();
        let names = tokio::task::spawn_blocking(move || permitted())
            .await
            .map_err(|_| ErrorData::internal_error("OPERATION_FAILED", None))?
            .map_err(|e| ErrorData::internal_error(e.legacy_message(), None))?;
        Ok(super::contracts::OPERATIONS
            .iter()
            .filter(|op| names.iter().any(|name| name == op.name))
            .collect())
    }
}

impl ServerHandler for ProtocolHost {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            "super-control-panel",
            env!("CARGO_PKG_VERSION"),
        ))
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self
            .visible()
            .await?
            .into_iter()
            .map(standard_tool)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListToolsResult::with_all_items(tools))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let op = operation(&request.name)
            .ok_or_else(|| ErrorData::invalid_params("Unknown tool", None))?;
        let args = Value::Object(request.arguments.unwrap_or_default());
        validation::validate_arguments(op, &args)
            .map_err(|_| ErrorData::invalid_params("Invalid tool arguments", None))?;
        match self.invoke(op, args).await {
            Ok(data) => Ok(CallToolResult::structured(data).into()),
            Err(error) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(error.legacy_message())]).into())
            }
        }
    }
    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let resources = self
            .visible()
            .await?
            .into_iter()
            .filter_map(|op| op.resource)
            .map(|(uri, title, _)| Resource::new(uri, title).with_mime_type("application/json"))
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let op = resource_operation(&request.uri)
            .ok_or_else(|| ErrorData::resource_not_found("Unknown resource", None))?;
        let data = self
            .invoke(op, json!({}))
            .await
            .map_err(|e| ErrorData::resource_not_found(e.legacy_message(), None))?;
        Ok(
            ReadResourceResult::new(vec![ResourceContents::text(data.to_string(), request.uri)
                .with_mime_type("application/json")])
            .into(),
        )
    }
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![
            ProtocolVersion::V_2026_07_28,
            ProtocolVersion::V_2025_11_25,
        ])
    }
}

// Preserve the Slice 2 status contract; wrap other payloads so arrays/null are valid
// structuredContent objects without changing the shared legacy data contracts.
fn result_key(op: &Operation) -> &'static str {
    if op.name == "function_status_list" {
        "statuses"
    } else {
        "result"
    }
}
fn standard_data(op: &Operation, data: Value) -> Value {
    json!({(result_key(op)): data})
}
fn standard_tool(op: &Operation) -> Result<Tool, ErrorData> {
    serde_json::from_value(json!({
        "name":op.name,"title":op.title,
        "description":format!("{}. Category: {}. Desktop-owned starts continue after disconnect; query state before retrying. Sensitive calls require allow-with-token in local settings. Protected import/Git capabilities remain unavailable.", op.title, op.category),
        "inputSchema":validation::input_schema(op),
        "outputSchema":validation::output_object(json!({(result_key(op)):validation::output_schema(op)})),
        "annotations":{"readOnlyHint":op.read_only,"destructiveHint":!op.read_only}
    })).map_err(|_| ErrorData::internal_error("OPERATION_FAILED", None))
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
        .with_legacy_session_mode(true)
        .with_json_response(json_response)
        .with_allowed_hosts(authorities.clone())
        .enforce_origin_validation()
        .with_max_request_body_bytes(1024 * 1024);
    let sdk_host = host.clone();
    let service = StreamableHttpService::new(
        move || Ok(sdk_host.clone()),
        LocalSessionManager::default().into(),
        config,
    );
    Router::new()
        .route_service("/mcp", service)
        .fallback(move |method: Method, uri: Uri, body: Bytes| {
            let legacy = host.legacy.clone();
            async move {
                match tokio::task::spawn_blocking(move || {
                    legacy(method.as_str(), uri.path(), &body)
                })
                .await
                {
                    Ok((status, value)) => (
                        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                        Json(value),
                    ),
                    Err(_) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"ok":false,"message":"OPERATION_FAILED"})),
                    ),
                }
            }
        })
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let token = token.clone();
            let authorities = authorities.clone();
            async move {
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
                Ok::<Response, (StatusCode, Json<Value>)>(next.run(request).await)
            }
        }))
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
