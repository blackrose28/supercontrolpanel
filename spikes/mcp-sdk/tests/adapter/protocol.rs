//! Protocol adapter hosting is deliberately UNWRAPPED by the HTTP-status experiment.
use super::*;
use rmcp::model::RequestMetaObject;

const SECRET: &str = "SYNTHETIC_SECRET_METHOD";

fn custom_cases() -> Vec<(&'static str, Value)> {
    vec![
        (SECRET, json!({"secret":SECRET})),
        (
            "tools/call",
            json!({"name":"app_get_overview","arguments":SECRET}),
        ),
        ("tools/call", json!({"name": {"secret":SECRET}})),
        (
            "initialize",
            json!({"protocolVersion":"2025-11-25","capabilities":SECRET,"clientInfo":{"name":"probe","version":"0"}}),
        ),
    ]
}

fn check_error(value: &Value) {
    assert_eq!(
        value,
        &json!({"code":-32601,"message":"MCP method not supported"})
    );
    assert!(!value.to_string().contains(SECRET));
}

#[tokio::test]
async fn official_custom_fallback_is_fixed_in_json_and_initialized_sse() {
    for version in [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25] {
        for json_response in [true, false] {
            let mut probe = OfficialProbe::start(
                version.clone(),
                json_response,
                &["read-only"],
                "allow-with-token",
                std::convert::identity,
            )
            .await;
            for (method, params) in custom_cases() {
                let error = bounded(probe.client.send_request(ClientRequest::CustomRequest(
                    CustomRequest::new(method, Some(params)),
                )))
                .await
                .unwrap_err();
                match error {
                    ServiceError::McpError(error) => {
                        check_error(&serde_json::to_value(error).unwrap())
                    }
                    _ => panic!("expected decoded MCP error"),
                }
                assert!(probe.reads.try_recv().is_err());
            }
            // A protocol rejection must leave discovery and the session usable.
            check_catalog(&probe.client).await;
            probe.finish().await;
        }
    }
}

struct RawProbe {
    server: BoundServer,
    version: ProtocolVersion,
    session: Option<String>,
    reads: mpsc::UnboundedReceiver<Read>,
}
impl RawProbe {
    async fn start(version: ProtocolVersion, json_response: bool) -> Self {
        let (tx, reads) = mpsc::unbounded_channel();
        let adapter = OverviewAdapter::new(
            FakeHost(tx),
            OverviewPolicy {
                allowed_categories: vec!["read-only".into()],
                sensitive_action_policy: "allow-with-token".into(),
            },
        );
        let stop = CancellationToken::new();
        let service: StreamableHttpService<OverviewAdapter<FakeHost>, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(adapter.clone()),
                Default::default(),
                StreamableHttpServerConfig::default()
                    .with_json_response(json_response)
                    .with_legacy_session_mode(!json_response)
                    .enforce_origin_validation()
                    .with_cancellation_token(stop.child_token()),
            );
        let server = BoundServer::bind(Router::new().nest_service("/mcp", service), stop).await;
        let mut probe = Self {
            server,
            version,
            session: None,
            reads,
        };
        if probe.version == ProtocolVersion::V_2025_11_25 {
            let response = probe.post("initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"raw-protocol","version":"0"}})).await;
            probe.session = response
                .headers()
                .get("Mcp-Session-Id")
                .map(|v| v.to_str().unwrap().into());
            assert_eq!(probe.session.is_some(), !json_response);
            let init = decode(response).await;
            assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
            let mut notification = reqwest::Client::new()
                .post(&probe.server.url)
                .header("Accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", probe.version.as_str())
                .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
            if let Some(session) = &probe.session {
                notification = notification.header("Mcp-Session-Id", session);
            }
            assert_eq!(bounded(notification.send()).await.unwrap().status(), 202);
        } else {
            let discovery = decode(probe.post("server/discover", json!({})).await).await;
            assert_eq!(discovery["result"]["capabilities"], json!({"tools":{}}));
        }
        probe
    }
    async fn post(&self, method: &str, mut params: Value) -> reqwest::Response {
        if self.version == ProtocolVersion::V_2026_07_28 {
            params["_meta"] = serde_json::to_value(RequestMetaObject::with_client_context(
                self.version.clone(),
                rmcp::model::Implementation::new("raw-protocol", "0"),
                Default::default(),
            ))
            .unwrap();
        }
        let mut request = reqwest::Client::new()
            .post(&self.server.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", self.version.as_str())
            .header("Mcp-Method", method)
            .json(&json!({"jsonrpc":"2.0","id":42,"method":method,"params":params}));
        if let Some(name) = params["name"].as_str() {
            request = request.header("Mcp-Name", name);
        }
        if let Some(session) = &self.session {
            request = request.header("Mcp-Session-Id", session);
        }
        bounded(request.send()).await.unwrap()
    }
}

async fn decode(response: reqwest::Response) -> Value {
    let sse = response.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream");
    if !sse {
        return bounded(response.json()).await.unwrap();
    }
    // Consume only until the request's terminal event; never buffer a live SSE body.
    let mut response = response;
    let mut pending = String::new();
    while let Some(chunk) = bounded(response.chunk()).await.unwrap() {
        pending.push_str(std::str::from_utf8(&chunk).unwrap());
        assert!(pending.len() < 65536, "synthetic SSE fixture bound");
        while let Some(end) = pending.find("\n\n") {
            let event = pending[..end].to_owned();
            pending.drain(..end + 2);
            if let Some(value) = terminal_event(&event) {
                return value;
            }
        }
    }
    panic!("missing terminal SSE event");
}

// The native legacy session stream starts with an empty data priming event.
fn terminal_event(event: &str) -> Option<Value> {
    event
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| !data.trim().is_empty())
        .map(|data| serde_json::from_str::<Value>(data).unwrap())
        .find(|value| value.get("result").is_some() || value.get("error").is_some())
}

#[tokio::test]
async fn raw_custom_fallback_preserves_native_status_code_and_envelope() {
    for version in [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25] {
        for json_response in [true, false] {
            let mut probe = RawProbe::start(version.clone(), json_response).await;
            for (case, (method, params)) in custom_cases().into_iter().enumerate() {
                // Establish the published untagged enum fallback, not call_tool validation.
                assert!(matches!(
                    serde_json::from_value::<ClientRequest>(
                        json!({"method":method,"params":params})
                    )
                    .unwrap(),
                    ClientRequest::CustomRequest(_)
                ));
                let response = probe.post(method, params).await;
                assert_eq!(
                    response.status().as_u16(),
                    if version == ProtocolVersion::V_2026_07_28 {
                        404
                    } else {
                        200
                    },
                    "case {case}, json={json_response}, version={version}"
                );
                let content_type = response.headers()["content-type"].to_str().unwrap();
                assert_eq!(
                    content_type.starts_with("text/event-stream"),
                    !json_response && version == ProtocolVersion::V_2025_11_25
                );
                let value = decode(response).await;
                assert_eq!(value["jsonrpc"], "2.0");
                assert_eq!(value["id"], 42);
                assert!(value.get("result").is_none());
                check_error(&value["error"]);
                assert!(probe.reads.try_recv().is_err());
            }
            let listed = decode(probe.post("tools/list", json!({})).await).await;
            assert_eq!(listed["result"]["tools"][0]["name"], "app_get_overview");
            probe.server.finish().await;
        }
    }
}
