pub mod server;
use axum::Router;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ProtocolVersion, ServerConfig,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, RoleServer, ServerHandler,
};
use serde_json::{json, Value};
use server::BoundServer;
use std::{borrow::Cow, future::Future, time::Duration};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

pub async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("fixture watchdog expired")
}

pub struct Entered {
    pub cancelled: CancellationToken,
    pub dropped: CancellationToken,
    pub release: oneshot::Sender<()>,
}

impl Entered {
    pub async fn release_after_cancellation(self) {
        assert!(self.cancelled.is_cancelled());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), self.dropped.cancelled())
                .await
                .is_err(),
            "non-cooperative handler unexpectedly dropped"
        );
        self.release.send(()).unwrap();
        bounded(self.dropped.cancelled()).await;
    }
}

#[derive(Clone)]
struct GatedHandler(mpsc::UnboundedSender<Entered>);

impl ServerHandler for GatedHandler {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        scp_mcp_sdk_spike::Fixture.supported_protocol_versions()
    }
    fn get_info(&self) -> ServerConfig {
        scp_mcp_sdk_spike::Fixture.get_info()
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let dropped = CancellationToken::new();
        let _drop_guard = dropped.clone().drop_guard();
        let (release, wait) = oneshot::channel();
        self.0
            .send(Entered {
                cancelled: context.ct.clone(),
                dropped,
                release,
            })
            .unwrap();
        if request
            .arguments
            .as_ref()
            .and_then(|a| a.get("progress"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            let token = context.meta.get("progressToken").unwrap().clone();
            context
                .peer
                .notify_progress(rmcp::model::ProgressNotificationParam::new(
                    serde_json::from_value(token).unwrap(),
                    0.0,
                ))
                .await
                .unwrap();
        }
        // Intentionally does not cooperate with context.ct: observe whether the
        // published SDK drops an in-flight future independently of host cooperation.
        let _ = wait.await;
        Ok(CallToolResult::structured(json!({"state":"completed"})).into())
    }
}

pub struct LifecycleHost {
    version: &'static str,
    session: Option<String>,
    entered: mpsc::UnboundedReceiver<Entered>,
    pub server: BoundServer,
}

impl LifecycleHost {
    pub async fn start(version: &'static str, json_response: bool) -> Self {
        let stop = CancellationToken::new();
        let (tx, entered) = mpsc::unbounded_channel();
        let service: StreamableHttpService<GatedHandler, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(GatedHandler(tx.clone())),
                Default::default(),
                StreamableHttpServerConfig::default()
                    .with_json_response(json_response)
                    .with_legacy_session_mode(!json_response)
                    .enforce_origin_validation()
                    .with_cancellation_token(stop.child_token()),
            );
        let router = Router::new().nest_service("/mcp", service);
        let server = BoundServer::bind(router, stop).await;
        let mut host = Self {
            version,
            session: None,
            entered,
            server,
        };
        if version == "2025-11-25" {
            let response = bounded(host.post("initialize", json!({"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"lifecycle","version":"0"}}), Some(1)).send()).await.unwrap();
            assert_eq!(response.status(), 200);
            host.session = response
                .headers()
                .get("Mcp-Session-Id")
                .map(|v| v.to_str().unwrap().to_owned());
            bounded(response.text()).await.unwrap();
            let initialized = bounded(
                host.post("notifications/initialized", json!({}), None)
                    .send(),
            )
            .await
            .unwrap();
            assert_eq!(initialized.status(), 202);
        }
        host
    }

    fn post(&self, method: &str, mut params: Value, id: Option<u64>) -> reqwest::RequestBuilder {
        if self.version == "2026-07-28" {
            params["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!(self.version);
            params["_meta"]["io.modelcontextprotocol/clientInfo"] =
                json!({"name":"lifecycle","version":"0"});
            params["_meta"]["io.modelcontextprotocol/clientCapabilities"] = json!({});
        }
        let mut body = json!({"jsonrpc":"2.0","method":method,"params":params});
        if let Some(id) = id {
            body["id"] = json!(id);
        }
        let mut request = reqwest::Client::new()
            .post(&self.server.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", self.version)
            .header("Mcp-Method", method)
            .json(&body);
        if let Some(session) = &self.session {
            request = request.header("Mcp-Session-Id", session);
        }
        request
    }

    pub fn call(
        &self,
        id: u64,
        progress: bool,
    ) -> JoinHandle<Result<reqwest::Response, reqwest::Error>> {
        let request = self.post("tools/call", json!({"name":"gate","arguments":{"progress":progress},"_meta":{"progressToken":id}}), Some(id)).header("Mcp-Name", "gate");
        tokio::spawn(async move { request.send().await })
    }
    pub async fn entered(&mut self) -> Entered {
        bounded(self.entered.recv()).await.unwrap()
    }
    pub async fn cancel(&self, id: u64) {
        let response = bounded(
            self.post("notifications/cancelled", json!({"requestId":id}), None)
                .send(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 202);
    }
    pub async fn finish(self) {
        self.server.finish().await;
    }
}
