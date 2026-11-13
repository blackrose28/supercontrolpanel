//! Release synthetic work only after the SDK service loop reports input EOF.
use super::{cancellation::Gate, *};
use tokio::net::TcpListener;

#[tokio::test]
async fn active_overview_completes_after_observed_sdk_eof() {
    for version in [LEGACY, MODERN] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_stdio-fixture"))
            .arg("gated-eof")
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
