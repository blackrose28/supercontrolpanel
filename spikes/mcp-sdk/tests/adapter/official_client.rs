//! Official-client qualification of the injected adapter; all data/work are synthetic.
use super::*;
use rmcp::{
    model::{
        CallToolRequest, CallToolRequestParams, CallToolResponse, ClientConfig, ClientRequest,
        ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, ProtocolVersion,
        ServerConfig,
    },
    service::{
        NotificationContext, PeerRequestOptions, RequestContext, RequestHandle, RunningService,
    },
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ClientHandler, ClientLifecycleMode, ClientServiceExt, ErrorData, RoleClient, RoleServer,
    ServerHandler, ServiceError,
};
use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
};

#[tokio::test]
async fn official_catalog_dto_and_host_error_matrix() {
    for version in [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25] {
        for json_response in [true, false] {
            let mut probe = OfficialProbe::start(
                version.clone(),
                json_response,
                &["read-only"],
                "require-ui-approval",
            )
            .await;
            check_catalog(&probe.client).await;
            for arguments in [None, Some(serde_json::Map::new())] {
                let mut params = CallToolRequestParams::new("app_get_overview");
                params.arguments = arguments;
                let call = probe.call(params).await;
                let read = bounded(probe.reads.recv()).await.unwrap();
                read.release.send(Ok(overview())).ok().unwrap();
                bounded(read.finished).await.unwrap();
                let result =
                    serde_json::to_value(bounded(call.await_response()).await.unwrap()).unwrap();
                if version == ProtocolVersion::V_2026_07_28 {
                    assert_eq!(result["resultType"], "complete");
                }
                // A request handle returns the standard ClientResult, without a JSON-RPC envelope.
                check_overview_result(&json!({"result":result}), version.as_str());
            }
            let call = probe
                .call(CallToolRequestParams::new("app_get_overview"))
                .await;
            let read = bounded(probe.reads.recv()).await.unwrap();
            read.release
                .send(Err("password=TOP_SECRET native /private/path".into()))
                .ok()
                .unwrap();
            bounded(read.finished).await.unwrap();
            let result =
                serde_json::to_value(bounded(call.await_response()).await.unwrap()).unwrap();
            assert_eq!(result["isError"], true);
            assert_eq!(
                result["content"],
                json!([{"type":"text","text":"App overview unavailable"}])
            );
            assert!(result.get("structuredContent").is_none());
            assert!(!result.to_string().contains("TOP_SECRET"));
            probe.finish().await;
        }
    }
}

#[tokio::test]
async fn official_rejections_filter_catalog_and_do_not_enter_host() {
    for version in [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25] {
        for json_response in [true, false] {
            for (categories, policy) in [
                (vec![], "allow-with-token"),
                (vec!["read-only"], "SECRET_POLICY"),
                (vec!["read-only", "SECRET_CATEGORY"], "allow-with-token"),
                (vec!["read-only"], "allow-with-token"),
            ] {
                let permitted = categories == ["read-only"] && policy == "allow-with-token";
                let mut probe =
                    OfficialProbe::start(version.clone(), json_response, &categories, policy).await;
                assert_eq!(
                    bounded(probe.client.list_tools(None))
                        .await
                        .unwrap()
                        .tools
                        .len(),
                    usize::from(permitted)
                );
                let mut requests = vec![
                    CallToolRequestParams::new("SECRET_UNKNOWN"),
                    CallToolRequestParams::new("function_status_list"),
                    CallToolRequestParams::new("app_get_overview")
                        .with_arguments(json!({"SECRET_ARG":true}).as_object().unwrap().clone()),
                ];
                if !permitted {
                    requests.push(CallToolRequestParams::new("app_get_overview"));
                }
                for request in requests {
                    check_rejection(&probe.client, request).await;
                    assert!(probe.reads.try_recv().is_err());
                }
                probe.finish().await;
            }
        }
    }
}

#[tokio::test]
async fn official_cancel_after_consumed_progress_preserves_concurrent_read() {
    for version in [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25] {
        let mut probe =
            OfficialProbe::start(version.clone(), false, &["read-only"], "allow-with-token").await;
        let first = probe
            .call(CallToolRequestParams::new("app_get_overview"))
            .await;
        let cancelled = bounded(probe.reads.recv()).await.unwrap();
        let progress = bounded(probe.progress.recv()).await.unwrap();
        assert_eq!(progress.progress_token, first.progress_token);
        assert_eq!(progress.progress, 1.0);
        let other = probe
            .call(CallToolRequestParams::new("app_get_overview"))
            .await;
        let survivor = bounded(probe.reads.recv()).await.unwrap();
        let progress = bounded(probe.progress.recv()).await.unwrap();
        assert_eq!(progress.progress_token, other.progress_token);
        let cancelled_id = serde_json::to_value(&first.id).unwrap();
        // Explicit API, not an assumption that dropping a handle cancels work.
        bounded(first.cancel(Some("fixture cancellation".into())))
            .await
            .unwrap();
        bounded(cancelled.cancellation.cancelled()).await;
        bounded(cancelled.finished).await.unwrap();
        {
            let wire = probe.wire.lock().unwrap();
            let calls: Vec<_> = wire.iter().filter(|r| r.method == "tools/call").collect();
            assert_eq!(calls.len(), 2);
            let legacy = version == ProtocolVersion::V_2025_11_25;
            assert!(calls.iter().all(|r| r.session == legacy));
            let cancellations: Vec<_> = wire
                .iter()
                .filter(|r| r.method == "notifications/cancelled")
                .collect();
            if legacy {
                assert_eq!(cancellations.len(), 1);
                assert!(cancellations[0].session);
                assert_eq!(cancellations[0].params["requestId"], cancelled_id);
            } else {
                // Snapshot plus the published worker's ControlMessage path: modern
                // RequestHandle::cancel closes the SSE stream instead of POSTing.
                assert!(cancellations.is_empty());
            }
        }
        assert!(!survivor.cancellation.is_cancelled());
        survivor.release.send(Ok(overview())).ok().unwrap();
        bounded(survivor.finished).await.unwrap();
        let result = serde_json::to_value(bounded(other.await_response()).await.unwrap()).unwrap();
        check_overview_result(&json!({"result":result}), version.as_str());
        probe.finish().await;
    }
}

/// Progress belongs only to this transport probe, not the production host seam.
#[derive(Clone)]
struct ProgressAdapter {
    adapter: OverviewAdapter<FakeHost>,
    stream_progress: bool,
}

impl ServerHandler for ProgressAdapter {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        self.adapter.supported_protocol_versions()
    }
    fn get_info(&self) -> ServerConfig {
        self.adapter.get_info()
    }
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.adapter.list_tools(request, context).await
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if self.stream_progress {
            let token = context.meta.get_progress_token().unwrap();
            context
                .peer
                .notify_progress(ProgressNotificationParam::new(token, 1.0))
                .await
                .unwrap();
        }
        self.adapter.call_tool(request, context).await
    }
}

struct ProgressClient {
    config: ClientConfig,
    progress: mpsc::UnboundedSender<ProgressNotificationParam>,
}
impl ClientHandler for ProgressClient {
    fn get_info(&self) -> ClientConfig {
        self.config.clone()
    }
    async fn on_progress(
        &self,
        progress: ProgressNotificationParam,
        _: NotificationContext<RoleClient>,
    ) {
        self.progress.send(progress).unwrap();
    }
}

struct WireRequest {
    method: String,
    params: Value,
    session: bool,
}
struct OfficialProbe {
    server: BoundServer,
    client: RunningService<RoleClient, ProgressClient>,
    reads: mpsc::UnboundedReceiver<Read>,
    progress: mpsc::UnboundedReceiver<ProgressNotificationParam>,
    wire: Arc<Mutex<Vec<WireRequest>>>,
}
impl OfficialProbe {
    async fn start(
        version: ProtocolVersion,
        json_response: bool,
        categories: &[&str],
        policy: &str,
    ) -> Self {
        let (tx, reads) = mpsc::unbounded_channel();
        let handler = ProgressAdapter {
            adapter: OverviewAdapter::new(
                FakeHost(tx),
                OverviewPolicy {
                    allowed_categories: categories.iter().map(|s| (*s).into()).collect(),
                    sensitive_action_policy: policy.into(),
                },
            ),
            stream_progress: !json_response,
        };
        let stop = CancellationToken::new();
        let service: StreamableHttpService<ProgressAdapter, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(handler.clone()),
                Default::default(),
                StreamableHttpServerConfig::default()
                    .with_json_response(json_response)
                    .with_legacy_session_mode(!json_response)
                    .enforce_origin_validation()
                    .with_cancellation_token(stop.child_token()),
            );
        let wire = Arc::new(Mutex::new(Vec::new()));
        let router = record_requests(Router::new().nest_service("/mcp", service), wire.clone());
        let server = BoundServer::bind(router, stop).await;
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(server.url.clone()),
        );
        let lifecycle = if version == ProtocolVersion::V_2026_07_28 {
            ClientLifecycleMode::Discover {
                preferred_versions: vec![version.clone()],
            }
        } else {
            ClientLifecycleMode::Initialize
        };
        let (progress_tx, progress) = mpsc::unbounded_channel();
        let handler = ProgressClient {
            config: ClientConfig::default().with_protocol_version(version.clone()),
            progress: progress_tx,
        };
        let client = bounded(handler.serve_with_lifecycle(transport, lifecycle))
            .await
            .unwrap();
        let info = serde_json::to_value(client.peer_info().unwrap()).unwrap();
        assert_eq!(info["protocolVersion"], version.as_str());
        assert_eq!(info["capabilities"], json!({"tools":{}}));
        Self {
            server,
            client,
            reads,
            progress,
            wire,
        }
    }
    async fn call(&self, params: CallToolRequestParams) -> RequestHandle<RoleClient> {
        bounded(self.client.send_request_with_option(
            ClientRequest::CallToolRequest(CallToolRequest::new(params)),
            PeerRequestOptions::no_options(),
        ))
        .await
        .unwrap()
    }
    async fn finish(self) {
        bounded(self.client.cancel()).await.unwrap();
        self.server.finish().await;
    }
}

async fn check_catalog(client: &rmcp::Peer<RoleClient>) {
    let tools = bounded(client.list_tools(None)).await.unwrap();
    let tools = serde_json::to_value(tools).unwrap();
    let tool = &tools["tools"][0];
    assert_eq!(tools["tools"].as_array().unwrap().len(), 1);
    assert_eq!(tool["name"], "app_get_overview");
    assert_eq!(
        tool["inputSchema"],
        json!({"type":"object","properties":{},"additionalProperties":false})
    );
    assert_eq!(
        tool["outputSchema"],
        json!({
            "type":"object","additionalProperties":false,
            "properties":{"appVersion":{"type":"string"},"activeFunctions":{"type":"integer","minimum":0},"knownFunctionCount":{"type":"integer","minimum":0}},
            "required":["appVersion","activeFunctions","knownFunctionCount"]
        })
    );
}

async fn check_rejection(client: &rmcp::Peer<RoleClient>, request: CallToolRequestParams) {
    match bounded(client.call_tool(request)).await.unwrap_err() {
        ServiceError::McpError(error) => {
            assert_eq!(error.code.0, -32602);
            assert!(error.data.is_none());
            assert!(!serde_json::to_string(&error).unwrap().contains("SECRET"));
        }
        other => panic!("expected protocol rejection: {other:?}"),
    }
}

fn record_requests(router: Router, recorded: Arc<Mutex<Vec<WireRequest>>>) -> Router {
    router.layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let recorded = recorded.clone();
            async move {
                let (parts, body) = request.into_parts();
                let bytes = axum::body::to_bytes(body, 65536).await.unwrap();
                if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                    recorded.lock().unwrap().push(WireRequest {
                        method: value["method"].as_str().unwrap().into(),
                        params: value["params"].clone(),
                        session: parts.headers.contains_key("Mcp-Session-Id"),
                    });
                }
                next.run(axum::extract::Request::from_parts(
                    parts,
                    axum::body::Body::from(bytes),
                ))
                .await
            }
        },
    ))
}
