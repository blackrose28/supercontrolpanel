//! Test-only SDK feasibility spike, not a production desktop transport.
//! Synthetic handlers never read app state or execute operations.

use std::borrow::Cow;

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    Router,
};
use rmcp::{
    model::{
        object, CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
        ListResourcesResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
        ResourceContents, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, RoleServer, ServerHandler,
};
use serde_json::{json, Value};

/// Synthetic handler shared by isolated HTTP and lifecycle probes.
#[derive(Clone)]
pub struct Fixture;

impl ServerHandler for Fixture {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25])
    }

    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let schema = object(json!({
            "type":"object", "properties":{"message":{"type":"string"}}, "required":["message"]
        }));
        let tool = Tool::new("spike_echo", "Synthetic SDK fixture", schema.clone())
            .with_raw_output_schema(schema.into());
        Ok(ListToolsResult::with_all_items(vec![tool]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if request.name != "spike_echo" {
            return Err(ErrorData::invalid_params("Unknown fixture tool", None));
        }
        let message = request
            .arguments
            .as_ref()
            .and_then(|args| args.get("message"))
            .and_then(Value::as_str)
            .ok_or_else(|| ErrorData::invalid_params("message must be a string", None))?;
        let result = if message == "fail" {
            CallToolResult::error(vec![ContentBlock::text("Synthetic operation failure")])
        } else {
            CallToolResult::structured(json!({"message":message}))
        };
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(vec![Resource::new(
            "spike://snapshot",
            "Fixture",
        )]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != "spike://snapshot" {
            return Err(ErrorData::resource_not_found(
                "Unknown fixture resource",
                None,
            ));
        }
        Ok(
            ReadResourceResult::new(vec![ResourceContents::text("fixture", request.uri)])
                .with_ttl_ms(0)
                .with_cache_scope(CacheScope::Private)
                .into(),
        )
    }
}

async fn fixture_bearer(request: Request, next: Next) -> Result<Response, StatusCode> {
    // Public test constant only. Production must inject trusted, rotatable local settings.
    if request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Bearer spike-only-token")
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(request).await)
}

/// Mount a synthetic SDK service. The caller owns binding and shutdown.
/// This is deliberately not a runnable server or a production authorization adapter.
pub fn fixture_router(config: StreamableHttpServerConfig) -> Router {
    let service: StreamableHttpService<Fixture, LocalSessionManager> =
        StreamableHttpService::new(|| Ok(Fixture), Default::default(), config);
    Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn(fixture_bearer))
}
