use std::{future::Future, time::Duration};

use axum::Router;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use scp_core::mcp_dto::McpAppOverviewResult;
use scp_mcp_sdk_spike::overview::{OverviewAdapter, OverviewHost, OverviewPolicy};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

#[path = "support/server.rs"]
mod server;
use server::BoundServer;

struct Read {
    cancellation: CancellationToken,
    release: oneshot::Sender<Result<McpAppOverviewResult, String>>,
    finished: oneshot::Receiver<()>,
}

#[derive(Clone)]
struct FakeHost(mpsc::UnboundedSender<Read>);

impl OverviewHost for FakeHost {
    async fn read_overview(
        &self,
        cancellation: CancellationToken,
    ) -> Result<McpAppOverviewResult, String> {
        let (release, wait) = oneshot::channel();
        let (finished, completed) = oneshot::channel();
        self.0
            .send(Read {
                cancellation: cancellation.clone(),
                release,
                finished: completed,
            })
            .unwrap();
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err("secret cancellation detail".into()),
            result = wait => result.unwrap_or_else(|_| Err("secret dropped gate".into())),
        };
        let _ = finished.send(());
        result
    }
}

struct Probe {
    server: BoundServer,
    version: &'static str,
    reads: mpsc::UnboundedReceiver<Read>,
}

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("adapter fixture watchdog")
}

impl Probe {
    async fn start(version: &'static str, categories: &[&str], policy: &str) -> Self {
        let (tx, reads) = mpsc::unbounded_channel();
        let handler = OverviewAdapter::new(
            FakeHost(tx),
            OverviewPolicy {
                allowed_categories: categories.iter().map(|s| (*s).into()).collect(),
                sensitive_action_policy: policy.into(),
            },
        );
        let stop = CancellationToken::new();
        let service: StreamableHttpService<OverviewAdapter<FakeHost>, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(handler.clone()),
                Default::default(),
                StreamableHttpServerConfig::default()
                    .with_json_response(true)
                    .with_legacy_session_mode(false)
                    .enforce_origin_validation()
                    .with_cancellation_token(stop.child_token()),
            );
        let server = BoundServer::bind(Router::new().nest_service("/mcp", service), stop).await;
        let probe = Self {
            server,
            version,
            reads,
        };
        if version == "2025-11-25" {
            let initialized = probe.request("initialize", json!({"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"adapter-probe","version":"0"}}), 1).await;
            assert_eq!(initialized["result"]["protocolVersion"], version);
            assert_eq!(initialized["result"]["capabilities"], json!({"tools":{}}));
            let response = bounded(
                probe
                    .post("notifications/initialized", json!({}), 0)
                    .json(
                        &json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
                    )
                    .send(),
            )
            .await
            .unwrap();
            assert_eq!(response.status(), 202);
        }
        if version == "2026-07-28" {
            let discovered = probe.request("server/discover", json!({}), 1).await;
            assert_eq!(discovered["result"]["capabilities"], json!({"tools":{}}));
        }
        probe
    }

    fn post(&self, method: &str, mut params: Value, id: u64) -> reqwest::RequestBuilder {
        if self.version == "2026-07-28" {
            params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":self.version,
                "io.modelcontextprotocol/clientInfo":{"name":"adapter-probe","version":"0"},
                "io.modelcontextprotocol/clientCapabilities":{}});
        }
        reqwest::Client::new()
            .post(&self.server.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", self.version)
            .header("Mcp-Method", method)
            .header(
                "Mcp-Name",
                params["name"].as_str().unwrap_or("app_get_overview"),
            )
            .json(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
    }

    async fn request(&self, method: &str, params: Value, id: u64) -> Value {
        bounded(
            bounded(self.post(method, params, id).send())
                .await
                .unwrap()
                .json(),
        )
        .await
        .unwrap()
    }

    fn call(&self, id: u64) -> tokio::task::JoinHandle<Value> {
        let request = self.post(
            "tools/call",
            json!({"name":"app_get_overview","arguments":{}}),
            id,
        );
        tokio::spawn(async move {
            bounded(bounded(request.send()).await.unwrap().json())
                .await
                .unwrap()
        })
    }
}

fn overview() -> McpAppOverviewResult {
    McpAppOverviewResult {
        app_version: "fake-version".into(),
        active_functions: 2,
        known_function_count: 5,
    }
}

fn check_overview_result(result: &Value, version: &str) {
    let expected = json!({"appVersion":"fake-version","activeFunctions":2,"knownFunctionCount":5});
    assert_eq!(result["result"]["structuredContent"], expected);
    assert_eq!(
        serde_json::from_str::<Value>(result["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        expected
    );
    assert_eq!(result["result"]["isError"], false);
    assert_eq!(
        result["result"].get("resultType").is_some(),
        version == "2026-07-28"
    );
}

#[tokio::test]
async fn overview_catalog_schema_dto_and_redacted_host_errors() {
    for version in ["2026-07-28", "2025-11-25"] {
        let mut probe = Probe::start(version, &["read-only"], "require-ui-approval").await;
        let listed = probe.request("tools/list", json!({}), 2).await;
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "app_get_overview");
        assert_eq!(tools[0]["description"], "App overview");
        assert_eq!(
            tools[0]["inputSchema"],
            json!({"type":"object","properties":{},"additionalProperties":false})
        );
        assert_eq!(
            tools[0]["outputSchema"]["required"],
            json!(["appVersion", "activeFunctions", "knownFunctionCount"])
        );
        assert_eq!(
            tools[0]["outputSchema"]["properties"]["activeFunctions"],
            json!({"type":"integer","minimum":0})
        );
        let call = probe.call(10);
        bounded(probe.reads.recv())
            .await
            .unwrap()
            .release
            .send(Ok(overview()))
            .ok()
            .unwrap();
        let result = bounded(call).await.unwrap();
        check_overview_result(&result, version);
        let failed = probe.call(11);
        bounded(probe.reads.recv())
            .await
            .unwrap()
            .release
            .send(Err("password=TOP_SECRET native /private/path".into()))
            .ok()
            .unwrap();
        let result = bounded(failed).await.unwrap();
        assert_eq!(result["result"]["isError"], true);
        assert_eq!(
            result["result"]["content"][0]["text"],
            "App overview unavailable"
        );
        assert!(!result.to_string().contains("TOP_SECRET"));
        assert!(result["result"].get("structuredContent").is_none());
        probe.server.finish().await;
    }
}

#[tokio::test]
async fn rejected_calls_and_discovery_never_enter_host() {
    for version in ["2026-07-28", "2025-11-25"] {
        for (categories, policy) in [
            (vec![], "allow-with-token"),
            (vec!["read-only"], "SECRET_POLICY"),
            (vec!["read-only", "SECRET_CATEGORY"], "allow-with-token"),
        ] {
            let mut probe = Probe::start(version, &categories, policy).await;
            assert_eq!(
                probe.request("tools/list", json!({}), 2).await["result"]["tools"],
                json!([])
            );
            let denied = probe
                .request("tools/call", json!({"name":"app_get_overview"}), 3)
                .await;
            assert_eq!(denied["error"]["code"], -32602);
            assert!(!denied.to_string().contains("SECRET"));
            assert!(probe.reads.try_recv().is_err());
            probe.server.finish().await;
        }
        let mut probe = Probe::start(version, &["read-only"], "allow-with-token").await;
        for params in [
            json!({"name":"SECRET_UNKNOWN"}),
            json!({"name":"function_status_list"}),
            json!({"name":"app_get_overview","arguments":{"SECRET_ARG":true}}),
        ] {
            let result = probe.request("tools/call", params, 4).await;
            assert_eq!(result["error"]["code"], -32602);
            assert!(!result.to_string().contains("SECRET"));
            assert!(probe.reads.try_recv().is_err());
        }
        // Omitted arguments are an empty object for this no-argument operation.
        let request = probe.post("tools/call", json!({"name":"app_get_overview"}), 5);
        let call = tokio::spawn(async move { request.send().await.unwrap() });
        bounded(probe.reads.recv())
            .await
            .unwrap()
            .release
            .send(Ok(overview()))
            .ok()
            .unwrap();
        assert_eq!(bounded(call).await.unwrap().status(), 200);
        probe.server.finish().await;
    }
}

#[tokio::test]
async fn stateless_disconnect_reaches_cooperative_host_and_preserves_other_read() {
    for version in ["2026-07-28", "2025-11-25"] {
        let mut probe = Probe::start(version, &["read-only"], "allow-with-token").await;
        let cancelled_call = probe.call(20);
        let cancelled = bounded(probe.reads.recv()).await.unwrap();
        let surviving_call = probe.call(21);
        let surviving = bounded(probe.reads.recv()).await.unwrap();
        cancelled_call.abort();
        assert!(bounded(cancelled_call).await.unwrap_err().is_cancelled());
        bounded(cancelled.cancellation.cancelled()).await;
        bounded(cancelled.finished).await.unwrap();
        assert!(!surviving.cancellation.is_cancelled());
        surviving.release.send(Ok(overview())).ok().unwrap();
        bounded(surviving.finished).await.unwrap();
        let result = bounded(surviving_call).await.unwrap();
        assert_eq!(result["result"]["structuredContent"]["activeFunctions"], 2);
        assert_eq!(result["result"]["isError"], false);
        probe.server.finish().await;
    }
}

#[path = "adapter/official_client.rs"]
mod official_client;
