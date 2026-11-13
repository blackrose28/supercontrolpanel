//! Hold synthetic work across observed SDK EOF, then qualify completion or expiry.
use super::{cancellation::Gate, *};
use tokio::net::TcpListener;

/// Shared raw handshake, held host entry and actual SDK EOF evidence.
async fn held_eof(
    version: &'static str,
    mode: &str,
) -> (Wire, Gate, BufReader<tokio::process::ChildStderr>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_stdio-fixture"))
        .arg(mode)
        .arg(listener.local_addr().unwrap().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut wire = Wire::connect(version, child).await;
    let mut params = json!({"name":"app_get_overview","arguments":{}});
    if version == MODERN {
        params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":MODERN,
            "io.modelcontextprotocol/clientInfo":{"name":"stdio-probe","version":"0"},
            "io.modelcontextprotocol/clientCapabilities":{}});
    }
    wire.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":params}))
        .await;
    let mut gate = Gate::entered(&listener).await;
    gate.active().await;
    drop(wire.input.take());
    let mut stderr = BufReader::new(wire.child.stderr.take().unwrap());
    let mut diagnostics = String::new();
    bounded(async {
        loop {
            let mut line = String::new();
            assert!(stderr.read_line(&mut line).await.unwrap() > 0);
            diagnostics.push_str(&line);
            if line == "fixture sdk input eof\n" {
                break;
            }
        }
    })
    .await;
    assert!(!diagnostics.contains("fixture stopped\n"));
    assert!(wire.child.try_wait().unwrap().is_none());
    gate.active().await;
    (wire, gate, stderr, diagnostics)
}

#[tokio::test]
async fn active_overview_completes_after_observed_sdk_eof() {
    for version in [LEGACY, MODERN] {
        let (mut wire, mut gate, mut stderr, mut diagnostics) =
            held_eof(version, "gated-eof").await;
        gate.send("release").await;
        gate.finished("released").await;
        let response = wire.line().await.expect("drained overview response");
        assert_eq!(response["id"], 2);
        check_result(&response["result"], version, false);
        assert!(
            wire.line().await.is_none(),
            "no extra stdout after response"
        );
        assert!(bounded(wire.child.wait()).await.unwrap().success());
        bounded(stderr.read_to_string(&mut diagnostics))
            .await
            .unwrap();
        assert_eq!(diagnostics.matches("fixture host read\n").count(), 1);
        assert_eq!(diagnostics.matches("fixture sdk input eof\n").count(), 1);
        assert!(diagnostics.ends_with("fixture stopped\n"));
        assert!(!diagnostics.contains("SECRET"));
    }
}

/// The 12-second observation bound exceeds the SDK's five-second EOF drain.
/// No release or cancellation command is sent by this harness.
#[tokio::test]
async fn eof_drain_expiry_cancels_internal_work_before_wrapper_drop() {
    for version in [LEGACY, MODERN] {
        let (mut wire, mut gate, mut stderr, mut diagnostics) =
            held_eof(version, "gated-eof-expiry").await;
        // Expiry is an actual SDK event, never an arbitrary sleep.
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let mut line = String::new();
                assert!(stderr.read_line(&mut line).await.unwrap() > 0);
                diagnostics.push_str(&line);
                if line == "fixture sdk drain expired\n" {
                    break;
                }
            }
        })
        .await
        .expect("SDK expiry evidence watchdog exceeds drain window");
        gate.finished("cancelled").await;
        // Every remaining stdout line is checked; expired request has no response.
        assert!(wire.line().await.is_none());
        assert!(bounded(wire.child.wait()).await.unwrap().success());
        bounded(stderr.read_to_string(&mut diagnostics))
            .await
            .unwrap();
        assert_eq!(diagnostics.matches("fixture host read\n").count(), 1);
        assert_eq!(diagnostics.matches("fixture sdk input eof\n").count(), 1);
        assert_eq!(
            diagnostics.matches("fixture sdk drain expired\n").count(),
            1
        );
        assert!(diagnostics.contains("fixture host completion observed external active\n"));
        assert!(diagnostics.contains("fixture waiting closed external cancelled\n"));
        assert!(diagnostics.ends_with("fixture stopped\n"));
        assert!(!diagnostics.contains("SECRET"));
    }
}
