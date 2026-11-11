//! Disposable SDK fixture: no desktop imports, settings, credentials, or real operations.
use axum::{
    Router,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::*,
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone, Default)]
struct Fixture {
    active: Arc<AtomicUsize>,
}

struct ActiveRequest(Arc<AtomicUsize>);
impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl ServerHandler for Fixture {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("slice0-fixture", "0.1.0"))
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let schema = serde_json::json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false});
        let tools = ["fixture_echo", "fixture_slow"].into_iter().map(|name| serde_json::from_value(serde_json::json!({
            "name": name, "description": "Isolated compatibility fixture; no real side effects",
            "inputSchema": schema, "outputSchema": schema
        }))).collect::<Result<Vec<Tool>, _>>().map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if request.name != "fixture_echo" && request.name != "fixture_slow" {
            return Err(ErrorData::invalid_params("Unknown fixture tool", None));
        }
        let value = request
            .arguments
            .as_ref()
            .and_then(|a| a.get("value"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| ErrorData::invalid_params("value must be a string", None))?;
        if value == "deny" {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "POLICY_DENIED: fixture denial",
            )])
            .into());
        }
        if request.name == "fixture_slow" {
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = ActiveRequest(self.active.clone());
            tokio::select! {
                _ = context.ct.cancelled() => return Ok(CallToolResult::error(vec![ContentBlock::text("CANCELLED")]).into()),
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
            }
        }
        Ok(CallToolResult::structured(serde_json::json!({"value":value})).into())
    }

    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(vec![Resource::new(
            "fixture://status",
            "fixture-status",
        )]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri == "fixture://active" {
            return Ok(ReadResourceResult::new(vec![ResourceContents::text(
                self.active.load(Ordering::SeqCst).to_string(),
                request.uri,
            )])
            .into());
        }
        if request.uri != "fixture://status" {
            return Err(ErrorData::invalid_params("Unknown fixture resource", None));
        }
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text("{\"fixture\":true}", request.uri)
                .with_mime_type("application/json"),
        ])
        .into())
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![
            ProtocolVersion::V_2026_07_28,
            ProtocolVersion::V_2025_11_25,
        ])
    }
}

async fn authorize(request: Request, next: Next) -> Result<Response, StatusCode> {
    if request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        != Some("Bearer slice0-fixture-only")
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(request).await)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "json".into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(true)
        .with_json_response(mode == "json")
        .with_allowed_hosts([address.to_string()])
        .enforce_origin_validation();
    let fixture = Fixture::default();
    let service = StreamableHttpService::new(
        move || Ok(fixture.clone()),
        LocalSessionManager::default().into(),
        config,
    );
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn(authorize));
    println!("http://{address}/mcp");
    axum::serve(listener, router).await?;
    Ok(())
}
