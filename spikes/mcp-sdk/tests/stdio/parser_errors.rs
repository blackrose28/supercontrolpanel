//! Complete syntax errors and typed envelope failures never reach a host handler.
use super::*;

const SYNTAX: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":77,\"password\":\"SYNTHETIC_STDIO_SECRET\",}\n";

fn invalid_shape() -> Value {
    // Numeric method cannot become CustomRequest or an ignored compatibility notification.
    json!({"jsonrpc":"2.0","id":77,"method":17,
        "params":{"password":"SYNTHETIC_STDIO_SECRET"}})
}

async fn read_invalid_request_line(output: &mut (impl tokio::io::AsyncBufRead + Unpin)) {
    // This is a separate, exact SDK-error exception; correlated replies still require id.
    let mut bytes = Vec::new();
    let read = output
        .take(65537)
        .read_until(b'\n', &mut bytes)
        .await
        .unwrap();
    assert!(
        read > 0 && read <= 65536 && bytes.ends_with(b"\n"),
        "invalid protocol line"
    );
    let value: Value = serde_json::from_slice(&bytes).expect("stdout must be protocol JSON");
    assert_eq!(
        value,
        json!({"jsonrpc":"2.0",
        "error":{"code":-32600,"message":"Invalid request"}})
    );
}

async fn inject(wire: &mut Wire, structural: bool) {
    if structural {
        wire.send(invalid_shape()).await;
        bounded(read_invalid_request_line(&mut wire.output)).await;
    } else {
        bounded(wire.input.as_mut().unwrap().write_all(SYNTAX))
            .await
            .unwrap();
        bounded(wire.input.as_mut().unwrap().flush()).await.unwrap();
    }
}

async fn startup(structural: bool) {
    let mut child = spawn("startup-eof");
    let mut wire = Wire {
        input: child.stdin.take(),
        output: BufReader::new(child.stdout.take().unwrap()),
        child,
        version: MODERN,
    };
    // Probe before selection, then after complete discovery (still bootstrap).
    for id in [1, 2] {
        inject(&mut wire, structural).await;
        let discovery = wire.request("server/discover", json!({}), id).await;
        assert_eq!(discovery["result"]["capabilities"], json!({"tools":{}}));
        assert_eq!(
            discovery["result"]["supportedVersions"],
            json!([MODERN, LEGACY])
        );
    }
    drop(wire.input.take());
    assert!(wire.line().await.is_none());
    assert!(!bounded(wire.child.wait()).await.unwrap().success());
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
    assert_eq!(
        diagnostics,
        concat!(
            "fixture started\n",
            "fixture startup connection closed external active\n",
            "Error: ConnectionClosed(\"initialize request\")\n"
        )
    );
}

async fn active(structural: bool) {
    for version in [LEGACY, MODERN] {
        let mut wire = Wire::start(version, "input-errors").await;
        // Discovery is insufficient: this result proves actual service dispatch.
        check_catalog(&wire.request("tools/list", json!({}), 2).await["result"]);
        inject(&mut wire, structural).await;
        // No extra syntax-error reply may precede this exact correlated result.
        check_catalog(&wire.request("tools/list", json!({}), 3).await["result"]);
        drop(wire.input.take());
        assert!(wire.line().await.is_none());
        assert!(bounded(wire.child.wait()).await.unwrap().success());
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
        assert_eq!(
            diagnostics,
            concat!(
                "fixture started\n",
                "fixture input service created\n",
                "fixture input waiting closed external cancelled\n",
                "fixture stopped\n"
            )
        );
    }
}

#[tokio::test]
async fn startup_skips_complete_syntax_invalid_lines_without_selecting_service() {
    startup(false).await;
}

#[tokio::test]
async fn startup_replies_to_invalid_typed_envelopes_without_selecting_service() {
    startup(true).await;
}

#[tokio::test]
async fn active_services_skip_complete_syntax_invalid_lines_and_recover() {
    active(false).await;
}

#[tokio::test]
async fn active_services_reply_to_invalid_typed_envelopes_and_recover() {
    active(true).await;
}

#[tokio::test]
async fn invalid_request_checker_accepts_only_exact_bounded_idless_error() {
    let expected =
        b"{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Invalid request\"}}\n";
    bounded(read_invalid_request_line(&mut expected.as_slice())).await;
    let mut malformed = vec![
        Vec::new(),
        b"diagnostic\n".to_vec(),
        expected[..expected.len() - 1].to_vec(),
        vec![b' '; 65537],
    ];
    for value in [
        json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid request"}}),
        json!({"jsonrpc":"2.0","id":77,"error":{"code":-32600,"message":"Invalid request"}}),
        json!({"jsonrpc":"2.0","error":{"code":-32700,"message":"Invalid request"}}),
        json!({"jsonrpc":"2.0","error":{"code":-32600,"message":"SYNTHETIC_STDIO_SECRET"}}),
        json!({"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid request","data":null}}),
        json!({"jsonrpc":"2.0","result":{},"error":{"code":-32600,"message":"Invalid request"}}),
        json!({"jsonrpc":"1.0","error":{"code":-32600,"message":"Invalid request"}}),
        json!({"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid request"},"extra":true}),
    ] {
        malformed.push(format!("{value}\n").into_bytes());
    }
    for bytes in malformed {
        let result = bounded(tokio::spawn(async move {
            read_invalid_request_line(&mut bytes.as_slice()).await;
        }))
        .await;
        assert!(result.expect_err("invalid stdout accepted").is_panic());
    }
}
