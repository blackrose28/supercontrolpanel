//! Isolated, request-scoped read adapter. No desktop/runtime or authentication owner.

use std::{borrow::Cow, future::Future};

use rmcp::{
    model::{
        object, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
        ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
        Tool,
    },
    service::RequestContext,
    ErrorData, RoleServer, ServerHandler,
};
use scp_core::{
    mcp_catalog::list_mcp_tools, mcp_dto::McpAppOverviewResult,
    mcp_policy::validate_mcp_tool_policy,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const OVERVIEW: &str = "app_get_overview";

/// Hosts must cooperate with request cancellation; this seam cannot stop native work.
/// Raw failure strings are discarded at the adapter boundary, never sent or logged.
pub trait OverviewHost: Clone + Send + Sync + 'static {
    fn read_overview(
        &self,
        cancellation: CancellationToken,
    ) -> impl Future<Output = Result<McpAppOverviewResult, String>> + Send;
}

/// Trusted host settings, supplied independently of caller arguments/metadata.
#[derive(Clone)]
pub struct OverviewPolicy {
    pub allowed_categories: Vec<String>,
    pub sensitive_action_policy: String,
}

/// Only the one implemented read is available, regardless of catalog size.
#[derive(Clone)]
pub struct OverviewAdapter<H> {
    host: H,
    policy: OverviewPolicy,
}

impl<H: OverviewHost> OverviewAdapter<H> {
    pub fn new(host: H, policy: OverviewPolicy) -> Self {
        Self { host, policy }
    }

    fn authorize(&self, name: &str) -> Result<(), ErrorData> {
        validate_mcp_tool_policy(
            name,
            &self.policy.allowed_categories,
            &self.policy.sensitive_action_policy,
        )
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        if name != OVERVIEW {
            return Err(ErrorData::invalid_params(
                "MCP operation is not implemented by this host",
                None,
            ));
        }
        Ok(())
    }
}

impl<H: OverviewHost> ServerHandler for OverviewAdapter<H> {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        crate::Fixture.supported_protocol_versions()
    }

    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = list_mcp_tools()
            .into_iter()
            .filter(|entry| entry.name == OVERVIEW && self.authorize(&entry.name).is_ok())
            .map(|entry| {
                Tool::new(
                    entry.name,
                    entry.title,
                    object(json!({
                        "type":"object", "properties":{}, "additionalProperties":false
                    })),
                )
                .with_raw_output_schema(
                    object(json!({
                        "type":"object", "additionalProperties":false,
                        "properties":{
                            "appVersion":{"type":"string"},
                            "activeFunctions":{"type":"integer","minimum":0},
                            "knownFunctionCount":{"type":"integer","minimum":0}
                        }, "required":["appVersion","activeFunctions","knownFunctionCount"]
                    }))
                    .into(),
                )
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.authorize(&request.name)?;
        // A deliberately small explicit validator, not arbitrary JSON Schema support.
        if request
            .arguments
            .as_ref()
            .is_some_and(|args| !args.is_empty())
        {
            return Err(ErrorData::invalid_params(
                "app_get_overview expects no arguments",
                None,
            ));
        }
        let result = match self.host.read_overview(context.ct).await {
            Ok(overview) => CallToolResult::structured(json!(overview)),
            Err(_) => CallToolResult::error(vec![ContentBlock::text("App overview unavailable")]),
        };
        Ok(result.into())
    }
}
