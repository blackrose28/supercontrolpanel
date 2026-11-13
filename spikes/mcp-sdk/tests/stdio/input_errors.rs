//! Raw bootstrap diagnostics and dispatched errors are different boundaries.
use super::*;

const SENTINEL: &str = "SYNTHETIC_STDIO_SECRET";

fn peer_error() -> Value {
    json!({"jsonrpc":"2.0","error":{"code":-32600,"message":SENTINEL,
        "data":{"password":SENTINEL}}})
}

async fn complete(mut wire: Wire, expected_success: bool) -> String {
    drop(wire.input.take()); // Actual EOF; no cancel command or sleep.
    assert!(wire.line().await.is_none(), "no remaining stdout response");
    assert_eq!(
        bounded(wire.child.wait()).await.unwrap().success(),
        expected_success
    );
    let mut diagnostics = String::new();
    bounded(
        wire.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut diagnostics),
    )
    .await
    .unwrap();
    diagnostics
}

#[tokio::test]
async fn unsolicited_peer_error_before_selection_preserves_typed_raw_startup_error() {
    let mut child = spawn("input-errors");
    let mut wire = Wire {
        input: child.stdin.take(),
        output: BufReader::new(child.stdout.take().unwrap()),
        child,
        version: MODERN,
    };
    wire.send(peer_error()).await;
    let diagnostics = complete(wire, false).await;
    assert_eq!(
        diagnostics,
        concat!(
            "fixture started\n",
            "fixture unexpected peer error external active\n",
            "Error: ExpectedInitializeRequest(Some(Error(JsonRpcError { jsonrpc: JsonRpcVersion2_0, ",
            "id: None, error: ErrorData { code: ErrorCode(-32600), ",
            "message: \"SYNTHETIC_STDIO_SECRET\", data: Some(Object {\"password\": String(\"SYNTHETIC_STDIO_SECRET\")}) } })))\n"
        )
    );
}

async fn dispatched_errors(version: &'static str) {
    let mut wire = Wire::start(version, "input-errors").await;
    // Modern discover alone is still bootstrap. A correlated non-discover result
    // establishes the service loop before the unsolicited peer error is sent.
    let catalog = wire.request("tools/list", json!({}), 2).await;
    check_catalog(&catalog["result"]);
    wire.send(peer_error()).await;
    let response = wire
        .request(
            "tools/call",
            json!({"name":"app_get_overview","arguments":SENTINEL}),
            3,
        )
        .await;
    assert_eq!(
        response,
        json!({"jsonrpc":"2.0","id":3,
        "error":{"code":-32601,"message":"MCP method not supported"}})
    );
    let catalog = wire.request("tools/list", json!({}), 4).await;
    check_catalog(&catalog["result"]);
    assert_eq!(
        complete(wire, true).await,
        concat!(
            "fixture started\n",
            "fixture input service created\n",
            "fixture input waiting closed external cancelled\n",
            "fixture stopped\n"
        )
    );
}

#[tokio::test]
async fn legacy_service_drops_idless_peer_error_and_fixes_malformed_method_response() {
    dispatched_errors(LEGACY).await;
}

#[tokio::test]
async fn modern_service_drops_idless_peer_error_and_fixes_malformed_method_response() {
    dispatched_errors(MODERN).await;
}
