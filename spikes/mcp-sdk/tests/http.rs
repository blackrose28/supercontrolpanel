use std::time::Duration;

use rmcp::{
    model::{CallToolRequestParams, ClientConfig, ProtocolVersion, ReadResourceRequestParams},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig,
        streamable_http_server::StreamableHttpServerConfig, StreamableHttpClientTransport,
    },
    ClientLifecycleMode, ClientServiceExt, ServiceError,
};
use scp_mcp_sdk_spike::fixture_router;
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

struct Host {
    url: String,
    stop: CancellationToken,
    task: JoinHandle<std::io::Result<()>>,
}

impl Host {
    async fn start(json_response: bool) -> Self {
        let stop = CancellationToken::new();
        let router = fixture_router(
            StreamableHttpServerConfig::default()
                .with_max_request_body_bytes(4096)
                .with_json_response(json_response)
                .with_legacy_session_mode(!json_response)
                .enforce_origin_validation()
                .with_cancellation_token(stop.child_token()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let shutdown = stop.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
        });
        Self { url, stop, task }
    }

    async fn finish(mut self) {
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), &mut self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    fn post(&self, version: &str, method: &str, params: Value) -> reqwest::RequestBuilder {
        reqwest::Client::new()
            .post(&self.url)
            .bearer_auth("spike-only-token")
            .timeout(Duration::from_secs(5))
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", version)
            .header("Mcp-Method", method)
            .json(&json!({"jsonrpc":"2.0", "id":7, "method":method, "params":params}))
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}

async fn exercise_client(version: ProtocolVersion, json_response: bool) {
    let host = Host::start(json_response).await;
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(host.url.clone())
            .auth_header("spike-only-token"),
    );
    let lifecycle = if version == ProtocolVersion::V_2026_07_28 {
        ClientLifecycleMode::Discover {
            preferred_versions: vec![version.clone()],
        }
    } else {
        ClientLifecycleMode::Initialize
    };
    let client = ClientConfig::default()
        .with_protocol_version(version.clone())
        .serve_with_lifecycle(transport, lifecycle)
        .await
        .unwrap();
    let info = serde_json::to_value(client.peer_info().unwrap()).unwrap();
    assert_eq!(info["protocolVersion"], version.as_str());
    assert_eq!(info["capabilities"], json!({"tools":{}, "resources":{}}));
    check_tools(client.peer(), &version).await;
    check_resources(client.peer(), &version).await;
    client.cancel().await.unwrap();
    host.finish().await;
}

async fn check_tools(client: &rmcp::Peer<rmcp::RoleClient>, version: &ProtocolVersion) {
    let tools = client.list_tools(None).await.unwrap();
    if *version == ProtocolVersion::V_2026_07_28 {
        assert_eq!(tools.ttl_ms, Some(0));
        assert_eq!(
            serde_json::to_value(&tools).unwrap()["cacheScope"],
            "private"
        );
    }
    assert_eq!(tools.tools.len(), 1);
    assert_eq!(tools.tools[0].name, "spike_echo");
    assert_eq!(tools.tools[0].input_schema["required"], json!(["message"]));
    assert_eq!(
        tools.tools[0].output_schema.as_ref().unwrap()["type"],
        "object"
    );
    let result = client
        .call_tool(
            CallToolRequestParams::new("spike_echo")
                .with_arguments(json!({"message":"hello"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let result = serde_json::to_value(result).unwrap();
    assert_eq!(result["structuredContent"], json!({"message":"hello"}));
    assert_eq!(result["isError"], false);
    if *version == ProtocolVersion::V_2026_07_28 {
        assert_eq!(result["resultType"], "complete");
    } else {
        assert!(result.get("resultType").is_none());
    }
    assert_eq!(result["content"][0]["type"], "text");
    let failed = client
        .call_tool(
            CallToolRequestParams::new("spike_echo")
                .with_arguments(json!({"message":"fail"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(failed).unwrap()["isError"], true);
    for request in [
        CallToolRequestParams::new("spike_echo"),
        CallToolRequestParams::new("spike_echo")
            .with_arguments(json!({"message":42}).as_object().unwrap().clone()),
        CallToolRequestParams::new("unknown"),
    ] {
        match client.call_tool(request).await.unwrap_err() {
            ServiceError::McpError(error) => assert_eq!(error.code.0, -32602),
            other => panic!("expected protocol error: {other:?}"),
        }
    }
}

async fn check_resources(client: &rmcp::Peer<rmcp::RoleClient>, version: &ProtocolVersion) {
    let resources = client.list_resources(None).await.unwrap();
    if *version == ProtocolVersion::V_2026_07_28 {
        assert_eq!(resources.ttl_ms, Some(0));
        assert_eq!(
            serde_json::to_value(&resources).unwrap()["cacheScope"],
            "private"
        );
    }
    assert_eq!(resources.resources.len(), 1);
    assert_eq!(resources.resources[0].uri, "spike://snapshot");
    let resource = client
        .read_resource(ReadResourceRequestParams::new("spike://snapshot"))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(resource).unwrap()["contents"][0]["text"],
        "fixture"
    );
    assert!(client
        .read_resource(ReadResourceRequestParams::new("spike://absent"))
        .await
        .is_err());
}

async fn exercise_protocol(version: ProtocolVersion) {
    tokio::time::timeout(Duration::from_secs(20), async {
        for json_response in [true, false] {
            exercise_client(version.clone(), json_response).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn official_client_modern_json_and_sse() {
    exercise_protocol(ProtocolVersion::V_2026_07_28).await;
}

#[tokio::test]
async fn official_client_legacy_json_and_sse() {
    exercise_protocol(ProtocolVersion::V_2025_11_25).await;
}

fn meta(version: &str) -> Value {
    json!({"_meta": {
        "io.modelcontextprotocol/protocolVersion":version,
        "io.modelcontextprotocol/clientInfo":{"name":"spike-probe","version":"0"},
        "io.modelcontextprotocol/clientCapabilities":{}
    }})
}

#[tokio::test]
async fn modern_wire_validation_and_no_session_lifecycle() {
    let host = Host::start(true).await;
    let response = host
        .post("2026-07-28", "server/discover", meta("2026-07-28"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.headers().get("Mcp-Session-Id").is_none());
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["result"]["supportedVersions"],
        json!(["2026-07-28", "2025-11-25"])
    );
    for (version, method, params, status, code) in [
        (
            "2099-01-01",
            "server/discover",
            meta("2099-01-01"),
            400,
            -32022,
        ),
        ("2026-07-28", "tools/list", meta("2025-11-25"), 400, -32020),
        (
            "2026-07-28",
            "unknown/method",
            meta("2026-07-28"),
            404,
            -32601,
        ),
    ] {
        let response = host.post(version, method, params).send().await.unwrap();
        assert_eq!(response.status(), status, "{method}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], code, "{body}");
    }
    for method in [reqwest::Method::GET, reqwest::Method::DELETE] {
        let response = reqwest::Client::new()
            .request(method, &host.url)
            .bearer_auth("spike-only-token")
            .header("MCP-Protocol-Version", "2026-07-28")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 405);
    }
    host.finish().await;
}

#[tokio::test]
async fn bearer_origin_host_and_malformed_requests() {
    let host = Host::start(true).await;
    for method in [
        reqwest::Method::POST,
        reqwest::Method::GET,
        reqwest::Method::DELETE,
    ] {
        for token in [None, Some("wrong")] {
            let mut request = reqwest::Client::new().request(method.clone(), &host.url);
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            assert_eq!(request.send().await.unwrap().status(), 401);
        }
    }
    for (header, value) in [("Origin", "https://evil.example"), ("Host", "evil.example")] {
        let response = host
            .post("2026-07-28", "server/discover", meta("2026-07-28"))
            .header(header, value)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
    }
    let response = host
        .post("2026-07-28", "server/discover", meta("2026-07-28"))
        .body("{")
        .send()
        .await
        .unwrap();
    // Published 3.10.0 returns 415/plain text here, not a JSON-RPC parse error.
    assert_eq!(response.status(), 415);
    assert!(response
        .text()
        .await
        .unwrap()
        .starts_with("fail to deserialize request body"));
    let response = host
        .post("2026-07-28", "server/discover", meta("2026-07-28"))
        .body("x".repeat(4097))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    host.finish().await;
}

#[tokio::test]
async fn wire_response_formats_and_legacy_session_selection() {
    for json_response in [true, false] {
        let host = Host::start(json_response).await;
        for (version, method, params) in [
            ("2026-07-28", "server/discover", meta("2026-07-28")),
            (
                "2025-11-25",
                "initialize",
                json!({
                    "protocolVersion":"2025-11-25", "capabilities":{},
                    "clientInfo":{"name":"spike-probe","version":"0"}
                }),
            ),
        ] {
            let response = host.post(version, method, params).send().await.unwrap();
            assert_eq!(response.status(), 200);
            let expected_type = if json_response {
                "application/json"
            } else {
                "text/event-stream"
            };
            assert_eq!(response.headers()["content-type"], expected_type);
            assert_eq!(
                response.headers().get("Mcp-Session-Id").is_some(),
                version == "2025-11-25" && !json_response
            );
            let body = response.text().await.unwrap();
            let payload = if json_response {
                body.as_str()
            } else {
                body.lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .find(|line| !line.is_empty())
                    .unwrap()
            };
            let payload: Value = serde_json::from_str(payload).unwrap();
            assert!(payload["result"].is_object(), "{payload}");
            assert_eq!(payload["id"], 7);
        }
        host.finish().await;
    }
}
