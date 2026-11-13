//! Startup EOF is not necessarily a RunningService service-loop EOF.
use super::*;

async fn startup_eof(stage: &str) {
    let mut child = spawn("startup-eof");
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut diagnostics = String::new();
    bounded(stderr.read_line(&mut diagnostics)).await.unwrap();
    assert_eq!(diagnostics, "fixture started\n");
    let mut wire = Wire {
        input: child.stdin.take(),
        output: BufReader::new(child.stdout.take().unwrap()),
        child,
        version: if stage == "legacy" { LEGACY } else { MODERN },
    };
    observe_stage(&mut wire, stage).await;
    if stage == "legacy" {
        let mut marker = String::new();
        bounded(stderr.read_line(&mut marker)).await.unwrap();
        assert_eq!(marker, "fixture running service created\n");
        diagnostics.push_str(&marker);
    }
    drop(wire.input.take()); // Actual stdin EOF; no service cancellation or sleep.
    assert!(
        wire.line().await.is_none(),
        "no remaining protocol response"
    );
    let status = bounded(wire.child.wait()).await.unwrap();
    bounded(stderr.read_to_string(&mut diagnostics))
        .await
        .unwrap();
    if stage == "legacy" {
        assert!(status.success());
        assert_eq!(
            diagnostics,
            concat!(
                "fixture started\n",
                "fixture running service created\n",
                "fixture sdk input eof\n",
                "fixture startup waiting closed external cancelled\n",
                "fixture stopped\n"
            )
        );
    } else {
        assert!(!status.success());
        // Preserve the SDK's typed error and Rust's raw Debug termination diagnostic.
        assert_eq!(
            diagnostics,
            concat!(
                "fixture started\n",
                "fixture startup connection closed external active\n",
                "Error: ConnectionClosed(\"initialize request\")\n"
            )
        );
    }
}

async fn observe_stage(wire: &mut Wire, stage: &str) {
    match stage {
        "legacy" => {
            let response = wire
                .request(
                    "initialize",
                    json!({"protocolVersion":LEGACY,
                "capabilities":{},"clientInfo":{"name":"startup-probe","version":"0"}}),
                    1,
                )
                .await;
            assert_eq!(response["result"]["protocolVersion"], LEGACY);
            assert_eq!(response["result"]["capabilities"], json!({"tools":{}}));
            // Deliberately never send notifications/initialized.
        }
        "discover" => {
            let response = wire.request("server/discover", json!({}), 1).await;
            assert_eq!(response["result"]["capabilities"], json!({"tools":{}}));
            assert_eq!(
                response["result"]["supportedVersions"],
                json!([MODERN, LEGACY])
            );
            // Complete inline metadata is sent by request(), but no non-discover request.
        }
        "empty" => {} // No bytes or messages written to stdin.
        _ => panic!("unknown test stage"),
    }
}

#[tokio::test]
async fn eof_without_a_first_message_returns_typed_startup_error() {
    startup_eof("empty").await;
}

#[tokio::test]
async fn eof_after_discover_still_returns_typed_startup_error() {
    startup_eof("discover").await;
}

#[tokio::test]
async fn legacy_eof_before_initialized_closes_running_service() {
    startup_eof("legacy").await;
}
