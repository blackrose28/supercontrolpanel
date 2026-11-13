//! Native evidence and narrowly mapped negotiation errors; no HTTP-status wrapper.
use super::*;
use scp_mcp_sdk_spike::negotiation_errors::with_fixed_negotiation_errors;

const MARKER: &str = "SYNTHETIC_SECRET_NEGOTIATION";
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";

fn metadata(version: &str) -> Value {
    serde_json::to_value(RequestMetaObject::with_client_context(
        serde_json::from_value(json!(version)).unwrap(),
        rmcp::model::Implementation::new("negotiation-probe", "0"),
        Default::default(),
    ))
    .unwrap()
}

// Expected codes are native observations, not a proposed reclassification.
type Case = (&'static str, &'static str, &'static str, Value, i64, bool);

fn cases() -> Vec<Case> {
    vec![
        (
            "legacy version mismatch",
            "2025-11-25",
            "tools/list",
            json!({"_meta":metadata(MARKER)}),
            -32020,
            true,
        ),
        (
            "version mismatch",
            MARKER,
            "tools/list",
            json!({"_meta":metadata("2026-07-28")}),
            -32020,
            true,
        ),
        (
            "unsupported",
            MARKER,
            "tools/list",
            json!({"_meta":metadata(MARKER)}),
            -32022,
            true,
        ),
        (
            "method mismatch",
            "2026-07-28",
            MARKER,
            json!({"_meta":metadata("2026-07-28")}),
            -32020,
            true,
        ),
        (
            "name mismatch",
            "2026-07-28",
            "tools/call",
            json!({"name":MARKER,"arguments":{},"_meta":metadata("2026-07-28")}),
            -32020,
            true,
        ),
        (
            "missing metadata",
            "2026-07-28",
            "tools/list",
            json!({"secret":MARKER}),
            -32602,
            false,
        ),
        (
            "incomplete metadata",
            "2026-07-28",
            "tools/list",
            json!({"_meta":{META_VERSION:"2026-07-28"},"secret":MARKER}),
            -32602,
            false,
        ),
        (
            "initialize mismatch",
            "2025-11-25",
            "initialize",
            json!({"protocolVersion":MARKER,"capabilities":{},"clientInfo":{"name":"probe","version":"0"}}),
            -32600,
            true,
        ),
    ]
}

async fn check_case(probe: &mut RawProbe, case: Case, mapped: bool) {
    let (label, header, method, params, code, reflects) = case;
    let body_method = if method == MARKER {
        "tools/list"
    } else {
        method
    };
    let body = json!({"jsonrpc":"2.0","id":42,"method":body_method,"params":params});
    let mut request = reqwest::Client::new()
        .post(&probe.server.url)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", header)
        .header("Mcp-Method", method)
        .header("Mcp-Name", "app_get_overview")
        .json(&body);
    if let Some(session) = &probe.session {
        request = request.header("Mcp-Session-Id", session);
    }
    let response = bounded(request.send()).await.unwrap();
    assert_eq!(response.status(), 400, "{label}");
    assert_eq!(response.headers()["content-type"], "application/json");
    let value: Value = bounded(response.json()).await.unwrap();
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["id"], 42);
    assert_eq!(value["error"]["code"], code, "{label}");
    if mapped {
        assert_eq!(
            value,
            json!({"jsonrpc":"2.0","id":42,"error":{"code":code,"message":"MCP request negotiation failed"}})
        );
    } else {
        assert_eq!(
            value.to_string().contains(MARKER),
            reflects,
            "{label}: {value}"
        );
    }
    assert!(probe.reads.try_recv().is_err());
}

async fn check_negotiation(mapped: bool) {
    for version in [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25] {
        for json_response in [true, false] {
            let mut probe = RawProbe::start_hosted(
                version.clone(),
                json_response,
                if mapped {
                    with_fixed_negotiation_errors
                } else {
                    std::convert::identity
                },
            )
            .await;
            for case in cases() {
                check_case(&mut probe, case, mapped).await;
            }
            // An initialized session remains usable, including a streamed catalog.
            let response = probe.post("tools/list", json!({})).await;
            assert_eq!(
                response.headers()["content-type"]
                    .to_str()
                    .unwrap()
                    .starts_with("text/event-stream"),
                !json_response
            );
            assert_eq!(
                decode(response).await["result"]["tools"][0]["name"],
                "app_get_overview"
            );
            probe.server.finish().await;
        }
    }
}

#[tokio::test]
async fn native_pre_dispatch_negotiation_evidence() {
    check_negotiation(false).await;
}

#[tokio::test]
async fn fixed_negotiation_preserves_native_status_code_and_session() {
    check_negotiation(true).await;
}

fn corrupt_list_headers(router: Router) -> Router {
    with_fixed_negotiation_errors(mismatch_list_headers(router))
}

fn mismatch_list_headers(router: Router) -> Router {
    router.layer(axum::middleware::from_fn(
        |mut request: axum::extract::Request, next: axum::middleware::Next| async move {
            // Only corrupt tools/list; real discovery/initialize and tool streaming survive.
            if request
                .headers()
                .get("Mcp-Method")
                .is_some_and(|v| v == "tools/list")
            {
                request
                    .headers_mut()
                    .insert("Mcp-Method", MARKER.parse().unwrap());
            }
            next.run(request).await
        },
    ))
}

async fn check_official_error(probe: &OfficialProbe, version: &ProtocolVersion) {
    let error = if *version == ProtocolVersion::V_2026_07_28 {
        bounded(probe.client.list_tools(None)).await.unwrap_err()
    } else {
        let params = json!({"protocolVersion":MARKER,"capabilities":{},
            "clientInfo":{"name":"probe","version":"0"}});
        let request = ClientRequest::CustomRequest(CustomRequest::new("initialize", Some(params)));
        bounded(probe.client.send_request(request))
            .await
            .unwrap_err()
    };
    let code = if *version == ProtocolVersion::V_2026_07_28 {
        -32020
    } else {
        -32600
    };
    match error {
        ServiceError::McpError(error) => assert_eq!(
            serde_json::to_value(error).unwrap(),
            json!({"code":code,"message":"MCP request negotiation failed"})
        ),
        _ => panic!("expected decoded negotiation error"),
    }
}

#[tokio::test]
async fn official_negotiation_error_is_decoded_and_stream_stays_live() {
    check_official_negotiation(corrupt_list_headers).await;
}

async fn check_official_negotiation(hosting: fn(Router) -> Router) {
    for version in [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25] {
        for json_response in [true, false] {
            let mut probe = OfficialProbe::start(
                version.clone(),
                json_response,
                &["read-only"],
                "allow-with-token",
                hosting,
            )
            .await;
            check_official_error(&probe, &version).await;
            assert!(probe.reads.try_recv().is_err());
            let call = probe
                .call(CallToolRequestParams::new("app_get_overview"))
                .await;
            let read = bounded(probe.reads.recv()).await.unwrap();
            if !json_response {
                // Progress must reach the official client while the real host is still gated.
                let progress = bounded(probe.progress.recv()).await.unwrap();
                assert_eq!(progress.progress_token, call.progress_token);
                assert_eq!(progress.progress, 1.0);
            }
            read.release.send(Ok(overview())).ok().unwrap();
            bounded(read.finished).await.unwrap();
            let result =
                serde_json::to_value(bounded(call.await_response()).await.unwrap()).unwrap();
            check_overview_result(&json!({"result":result}), version.as_str());
            probe.finish().await;
        }
    }
}

#[tokio::test]
async fn shipping_policy_raw_negotiation_and_protocol_errors_preserve_live_sessions() {
    use scp_mcp_sdk_spike::http_error_policy::with_http_error_policy;
    for version in [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25] {
        for json_response in [true, false] {
            let mut probe =
                RawProbe::start_hosted(version.clone(), json_response, with_http_error_policy)
                    .await;
            for case in cases() {
                check_case(&mut probe, case, true).await;
            }
            for (method, params) in custom_cases() {
                let response = probe.post(method, params).await;
                assert_eq!(
                    response.status().as_u16(),
                    if version == ProtocolVersion::V_2026_07_28 {
                        404
                    } else {
                        200
                    }
                );
                assert_eq!(
                    response.headers()["content-type"]
                        .to_str()
                        .unwrap()
                        .starts_with("text/event-stream"),
                    !json_response && version == ProtocolVersion::V_2025_11_25
                );
                let value = decode(response).await;
                assert_eq!(
                    value,
                    json!({"jsonrpc":"2.0","id":42,
                    "error":{"code":-32601,"message":"MCP method not supported"}})
                );
                assert!(probe.reads.try_recv().is_err());
            }
            let listed = decode(probe.post("tools/list", json!({})).await).await;
            assert_eq!(listed["result"]["tools"][0]["name"], "app_get_overview");
            probe.server.finish().await;
        }
    }
}

#[tokio::test]
async fn shipping_policy_official_client_decodes_protocol_errors_and_recovers() {
    check_official_custom_fallback(scp_mcp_sdk_spike::http_error_policy::with_http_error_policy)
        .await;
}

fn policy_with_mismatched_headers(router: Router) -> Router {
    scp_mcp_sdk_spike::http_error_policy::with_http_error_policy(mismatch_list_headers(router))
}

#[tokio::test]
async fn shipping_policy_official_negotiation_decodes_and_stream_stays_live() {
    check_official_negotiation(policy_with_mismatched_headers).await;
}
