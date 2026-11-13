//! Observe raw DEBUG data, independently of fixed EOF markers and HTTP policy.
use super::*;

const PARSER: &str = "SYNTHETIC_PARSER_PASSWORD";
const PEER: &str = "SYNTHETIC_STDIO_SECRET";
const REQUEST: &str = "SYNTHETIC_REQUEST_PASSWORD";

fn connect() -> (Wire, tokio::task::JoinHandle<String>) {
    let mut child = spawn("raw-debug");
    let stderr = child.stderr.take().unwrap();
    // Drain concurrently: diagnostics cannot fill the pipe while child.wait runs.
    let diagnostics = tokio::spawn(async move {
        let mut bytes = Vec::new();
        bounded(stderr.take(65537).read_to_end(&mut bytes))
            .await
            .unwrap();
        assert!(bytes.len() <= 65536, "fixture diagnostic budget exceeded");
        String::from_utf8(bytes).unwrap()
    });
    (
        Wire {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            version: MODERN,
        },
        diagnostics,
    )
}

async fn parser(wire: &mut Wire) {
    wire.send(json!({"jsonrpc":"2.0","id":77,"method":17,
        "params":{"password":PARSER}}))
        .await;
    bounded(super::parser_errors::read_invalid_request_line(
        &mut wire.output,
    ))
    .await;
}

fn records(diagnostics: &str) -> Vec<Value> {
    diagnostics
        .lines()
        .filter_map(|line| line.strip_prefix("fixture raw "))
        .map(|line| serde_json::from_str(line).expect("raw record JSON"))
        .collect()
}

fn check_parser(records: &[Value]) {
    let record = records
        .iter()
        .find(|r| {
            r["fields"]["message"]
                .as_str()
                .is_some_and(|m| m.starts_with("Failed to parse message receive:"))
        })
        .expect("raw SDK parser DEBUG event");
    assert_eq!(record["target"], "rmcp::transport::async_rw");
    assert_eq!(record["level"], "DEBUG");
    let message = record["fields"]["message"].as_str().unwrap();
    assert!(message.contains(PARSER));
    assert!(message.contains("\"method\":17"));
    assert!(message.contains(" | Error: "));
}

fn check_dispatch(raw: &[Value]) {
    let peer = raw
        .iter()
        .find(|r| r["fields"]["message"] == "received id-less peer error")
        .expect("raw peer error event");
    assert_eq!(peer["target"], "rmcp::service");
    assert_eq!(peer["level"], "DEBUG");
    let error = peer["fields"]["error"].as_str().unwrap();
    assert_eq!(
        error.matches(PEER).count(),
        2,
        "both message and data retained"
    );
    assert!(error.contains("password"));
    let request = raw
        .iter()
        .find(|r| r["fields"]["message"] == "received request" && r["fields"]["id"] == "3")
        .expect("raw request event");
    assert_eq!(request["target"], "rmcp::service");
    assert_eq!(request["level"], "DEBUG");
    assert!(request["fields"]["request"]
        .as_str()
        .unwrap()
        .contains(REQUEST));
}

async fn complete(
    mut wire: Wire,
    diagnostics: tokio::task::JoinHandle<String>,
    success: bool,
) -> String {
    drop(wire.input.take());
    assert!(
        wire.line().await.is_none(),
        "stdout remains exclusively protocol"
    );
    assert_eq!(bounded(wire.child.wait()).await.unwrap().success(), success);
    bounded(diagnostics).await.unwrap()
}

#[tokio::test]
async fn raw_debug_bootstrap_parser_and_original_typed_error() {
    let (mut wire, diagnostics) = connect();
    parser(&mut wire).await;
    assert_eq!(
        wire.request("server/discover", json!({}), 1).await["result"]["capabilities"],
        json!({"tools":{}})
    );
    wire.send(super::input_errors::peer_error()).await;
    let diagnostics = complete(wire, diagnostics, false).await;
    check_parser(&records(&diagnostics));
    let fixed: Vec<_> = diagnostics
        .lines()
        .filter(|line| !line.starts_with("fixture raw "))
        .collect();
    assert_eq!(fixed, vec![
        "fixture started",
        "fixture unexpected peer error external active",
        concat!("Error: ExpectedInitializeRequest(Some(Error(JsonRpcError { jsonrpc: JsonRpcVersion2_0, ",
            "id: None, error: ErrorData { code: ErrorCode(-32600), ",
            "message: \"SYNTHETIC_STDIO_SECRET\", data: Some(Object {\"password\": String(\"SYNTHETIC_STDIO_SECRET\")}) } })))")
    ]);
}

#[tokio::test]
async fn raw_debug_active_parser_peer_error_and_custom_request() {
    let (mut wire, diagnostics) = connect();
    wire.request("server/discover", json!({}), 1).await;
    // Correlated tools/list establishes actual service selection, unlike discovery.
    check_catalog(&wire.request("tools/list", json!({}), 2).await["result"]);
    parser(&mut wire).await;
    wire.send(super::input_errors::peer_error()).await;
    assert_eq!(
        wire.request(
            "tools/call",
            json!({"name":"app_get_overview","arguments":REQUEST}),
            3
        )
        .await,
        json!({"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"MCP method not supported"}})
    );
    check_catalog(&wire.request("tools/list", json!({}), 4).await["result"]);
    let diagnostics = complete(wire, diagnostics, true).await;
    let raw = records(&diagnostics);
    check_parser(&raw);
    check_dispatch(&raw);
    assert!(
        diagnostics.find("fixture input service created\n").unwrap()
            < diagnostics.find(PARSER).unwrap()
    );
    assert!(
        diagnostics.ends_with("fixture input waiting closed external cancelled\nfixture stopped\n")
    );
    let fixed: Vec<_> = diagnostics
        .lines()
        .filter(|line| !line.starts_with("fixture raw "))
        .collect();
    assert_eq!(
        fixed,
        vec![
            "fixture started",
            "fixture input service created",
            "fixture input waiting closed external cancelled",
            "fixture stopped"
        ]
    );
}
