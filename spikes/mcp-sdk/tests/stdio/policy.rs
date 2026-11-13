//! Deny-all tracing boundary; raw characterization remains independently asserted.
use super::*;

const STARTUP: &str = "scpctl: MCP stdio startup failed\n";
const LOGS: [&str; 2] = [
    "trace",
    "rmcp=trace,rmcp::service=trace,rmcp::transport::async_rw=trace",
];

fn connect(mode: &str, logs: &str) -> (Wire, tokio::task::JoinHandle<String>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_stdio-policy-fixture"))
        .arg(mode)
        .env("RUST_LOG", logs)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    // Drain before sending input, bounded in both size and time.
    let diagnostics = tokio::spawn(async move {
        let mut bytes = Vec::new();
        bounded(stderr.take(65537).read_to_end(&mut bytes))
            .await
            .unwrap();
        assert!(bytes.len() <= 65536, "diagnostic budget exceeded");
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

async fn complete(
    mut wire: Wire,
    diagnostics: tokio::task::JoinHandle<String>,
    success: bool,
) -> String {
    drop(wire.input.take());
    assert!(wire.line().await.is_none(), "unexpected stdout before EOF");
    let status = bounded(wire.child.wait()).await.unwrap();
    assert_eq!(status.success(), success);
    assert_eq!(status.code(), Some(if success { 0 } else { 1 }));
    bounded(diagnostics).await.unwrap()
}

async fn initialize(wire: &mut Wire) {
    let response = wire
        .request(
            "initialize",
            json!({"protocolVersion":LEGACY,
        "capabilities":{},"clientInfo":{"name":"policy-probe","version":"0"}}),
            1,
        )
        .await;
    assert_eq!(response["result"]["protocolVersion"], LEGACY);
    assert_eq!(response["result"]["capabilities"], json!({"tools":{}}));
}

async fn startup_peer_error(mode: &str, logs: &str, expected: &str) {
    let (mut wire, diagnostics) = connect(mode, logs);
    wire.send(super::input_errors::peer_error()).await;
    assert_eq!(complete(wire, diagnostics, false).await, expected);
}

#[tokio::test]
async fn policy_startup_peer_error_has_only_fixed_line_and_failure() {
    for logs in LOGS {
        startup_peer_error("normal", logs, STARTUP).await;
    }
}

#[tokio::test]
async fn policy_preselection_eof_is_failure_but_legacy_initialize_selects_service() {
    for stage in ["empty", "discover", "legacy"] {
        let (mut wire, diagnostics) = connect("normal", "trace");
        if stage == "discover" {
            let response = wire.request("server/discover", json!({}), 1).await;
            assert_eq!(
                response,
                json!({"jsonrpc":"2.0","id":1,"result":{
                    "resultType":"complete","supportedVersions":[MODERN,LEGACY],
                    "capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private",
                    "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"rmcp","version":"3.10.0"}}
                }})
            );
        } else if stage == "legacy" {
            wire.version = LEGACY;
            initialize(&mut wire).await; // No initialized notification needed for SDK selection.
        }
        assert_eq!(
            complete(wire, diagnostics, stage == "legacy").await,
            if stage == "legacy" { "" } else { STARTUP }
        );
    }
}

async fn active_probe(version: &'static str, mode: &str, logs: &str) {
    let (mut wire, diagnostics) = connect(mode, logs);
    wire.version = version;
    if version == LEGACY {
        initialize(&mut wire).await;
        wire.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
    } else {
        wire.request("server/discover", json!({}), 1).await;
    }
    // Correlated catalog proves service selection; discovery alone never does.
    check_catalog(&wire.request("tools/list", json!({}), 2).await["result"]);
    wire.send(json!({"jsonrpc":"2.0","id":77,"method":17,
        "params":{"password":"SYNTHETIC_PARSER_PASSWORD"}}))
        .await;
    bounded(super::parser_errors::read_invalid_request_line(
        &mut wire.output,
    ))
    .await;
    wire.send(super::input_errors::peer_error()).await;
    assert_eq!(
        wire.request(
            "tools/call",
            json!({"name":"app_get_overview","arguments":"SYNTHETIC_REQUEST_PASSWORD"}),
            3
        )
        .await,
        json!({"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"MCP method not supported"}})
    );
    check_catalog(&wire.request("tools/list", json!({}), 4).await["result"]);
    assert_eq!(complete(wire, diagnostics, true).await, "");
}

async fn active_matrix(mode: &str) {
    for version in [LEGACY, MODERN] {
        for logs in LOGS {
            active_probe(version, mode, logs).await;
        }
    }
}

#[tokio::test]
async fn policy_preserves_active_parser_peer_and_custom_request_wire() {
    active_matrix("normal").await;
}

#[tokio::test]
async fn policy_suppresses_all_levels_and_spans_on_main_and_worker() {
    active_matrix("levels").await;
}

#[tokio::test]
async fn policy_subscriber_conflict_fails_before_reading_even_open_stdin() {
    let (mut wire, diagnostics) = connect("conflict", "trace");
    // Retain open stdin through wait: even waiting for input would hit the watchdog.
    // The child's conflict tripwire precedes runtime/transport/service construction.
    let status = bounded(wire.child.wait()).await.unwrap();
    assert!(!status.success());
    assert_eq!(status.code(), Some(1));
    assert!(wire.line().await.is_none());
    assert_eq!(bounded(diagnostics).await.unwrap(), STARTUP);
}

#[tokio::test]
async fn policy_failed_diagnostic_sink_has_no_stdout_fallback_and_exits_one() {
    startup_peer_error("writer-failure", "trace", "").await;
}
