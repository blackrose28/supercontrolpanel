//! SDK adapter for the deliberately small Slice 2 surface. The desktop remains the host.
use super::{
    contracts::{operation, ErrorCode, OperationError},
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

const TOOL: &str = "function_status_list";
const RESOURCE: &str = "scp://status/functions";
type LegacyHandler = dyn Fn(&str, &str, &[u8]) -> (u16, Value) + Send + Sync;

#[derive(Clone)]
pub(super) struct ProtocolHost {
    pub permitted: Arc<dyn Fn() -> Result<bool, OperationError> + Send + Sync>,
    pub execute: Arc<dyn Fn(Value) -> Result<Value, OperationError> + Send + Sync>,
    pub legacy: Arc<LegacyHandler>,
}

impl ProtocolHost {
    async fn statuses(&self, args: Value) -> Result<Value, OperationError> {
        let execute = self.execute.clone();
        let data = tokio::task::spawn_blocking(move || execute(args))
            .await
            .map_err(|_| OperationError::new(ErrorCode::OperationFailed))??;
        if !validation::conforms(&validation::output_schema(operation(TOOL).unwrap()), &data) {
            return Err(OperationError::new(ErrorCode::OperationFailed));
        }
        Ok(json!({"statuses":data}))
    }
    async fn visible(&self) -> Result<bool, ErrorData> {
        let permitted = self.permitted.clone();
        tokio::task::spawn_blocking(move || permitted())
            .await
            .map_err(|_| ErrorData::internal_error("OPERATION_FAILED", None))?
            .map_err(|e| ErrorData::internal_error(e.legacy_message(), None))
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
        let mut tools = Vec::new();
        if self.visible().await? {
            let op = operation(TOOL).unwrap();
            let tool: Tool = serde_json::from_value(json!({
                "name":op.name,"title":op.title,"description":"Safe desktop function status snapshot. Query again to refresh; runtime diagnostic text is omitted.",
                "inputSchema":validation::input_schema(op),
                "outputSchema":{"type":"object","properties":{"statuses":validation::output_schema(op)},"required":["statuses"],"additionalProperties":false},
                "annotations":{"readOnlyHint":op.read_only,"destructiveHint":false}
            })).map_err(|_| ErrorData::internal_error("OPERATION_FAILED", None))?;
            tools.push(tool);
        }
        Ok(ListToolsResult::with_all_items(tools))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if request.name != TOOL {
            return Err(ErrorData::invalid_params(
                "Unknown or unimplemented tool",
                None,
            ));
        }
        let args = Value::Object(request.arguments.unwrap_or_default());
        validation::validate_arguments(operation(TOOL).unwrap(), &args)
            .map_err(|_| ErrorData::invalid_params("Invalid function status arguments", None))?;
        match self.statuses(args).await {
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
        let resources = if self.visible().await? {
            let (_, title, _) = operation(TOOL).unwrap().resource.unwrap();
            vec![Resource::new(RESOURCE, title).with_mime_type("application/json")]
        } else {
            vec![]
        };
        Ok(ListResourcesResult::with_all_items(resources))
    }
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != RESOURCE {
            return Err(ErrorData::resource_not_found(
                "Unknown or unimplemented resource",
                None,
            ));
        }
        let data = self
            .statuses(json!({}))
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
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
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

    async fn check_clients(json_response: bool) {
        let statuses = json!([{"functionId":"boundary-auth","state":"running","message":"","boundaryTokenSource":"app","sessionId":"synthetic-session","updatedAtMs":123}]);
        let legacy_statuses = statuses.clone();
        let permitted = Arc::new(AtomicBool::new(true));
        let policy = permitted.clone();
        let execution_policy = permitted.clone();
        let host = ProtocolHost {
            permitted: Arc::new(move || Ok(policy.load(Ordering::SeqCst))),
            execute: Arc::new(move |args| {
                let categories: &[&str] = if execution_policy.load(Ordering::SeqCst) {
                    &["read-only"]
                } else {
                    &[]
                };
                super::super::contracts::execute(
                    "function_status_list",
                    "require-ui-approval",
                    categories,
                    &args,
                    |_| Ok(statuses.clone()),
                )
            }),
            legacy: Arc::new(move |method, path, _| {
                if method == "POST" && path == "/test/deny" {
                    permitted.store(false, Ordering::SeqCst);
                }
                (
                    200,
                    json!({"ok":true,"message":"MCP operation completed","data":legacy_statuses,"warnings":[],"timestamp":123}),
                )
            }),
        };
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
}
