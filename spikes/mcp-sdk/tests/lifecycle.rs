mod support;

use serde_json::json;
use std::time::Duration;
use support::{bounded, LifecycleHost};

#[tokio::test]
async fn disconnect_and_explicit_cancellation_are_revision_specific() {
    for version in ["2026-07-28", "2025-11-25"] {
        let mut host = LifecycleHost::start(version, false).await;
        let first = host.call(10, true);
        let held = host.entered().await;
        let mut stream = bounded(first).await.unwrap().unwrap();
        assert_eq!(stream.status(), 200);
        // Consume a real progress event before closing the response stream.
        bounded(async {
            let mut wire = Vec::new();
            while !String::from_utf8_lossy(&wire).contains("notifications/progress") {
                wire.extend(
                    stream
                        .chunk()
                        .await
                        .unwrap()
                        .expect("stream ended before progress"),
                );
                assert!(wire.len() <= 4096, "unexpected fixture stream size");
            }
        })
        .await;
        let other = host.call(11, false);
        let survivor = host.entered().await;
        drop(stream);
        if version == "2026-07-28" {
            bounded(held.cancelled.cancelled()).await;
        } else {
            // A bounded observation, not proof that cancellation can never occur.
            assert!(
                tokio::time::timeout(Duration::from_millis(200), held.cancelled.cancelled())
                    .await
                    .is_err()
            );
            assert!(!held.dropped.is_cancelled());
            host.cancel(10).await;
            bounded(held.cancelled.cancelled()).await;
        }
        held.release_after_cancellation().await;
        assert!(!survivor.cancelled.is_cancelled());
        assert!(!survivor.dropped.is_cancelled());
        survivor.release.send(()).unwrap();
        let response = bounded(other).await.unwrap().unwrap();
        assert_eq!(response.status(), 200);
        assert!(bounded(response.text())
            .await
            .unwrap()
            .contains("completed"));
        bounded(survivor.dropped.cancelled()).await;
        host.finish().await;
    }
}

#[tokio::test]
async fn disconnect_before_first_response_cancels_stateless_requests() {
    for version in ["2026-07-28", "2025-11-25"] {
        let mut host = LifecycleHost::start(version, true).await;
        let request = host.call(20, false);
        let held = host.entered().await;
        request.abort();
        assert!(bounded(request).await.unwrap_err().is_cancelled());
        bounded(held.cancelled.cancelled()).await;
        held.release_after_cancellation().await;
        host.finish().await;
    }
}

#[tokio::test]
async fn listener_shutdown_does_not_join_noncooperative_handlers() {
    for version in ["2026-07-28", "2025-11-25"] {
        for json_response in [true, false] {
            let mut host = LifecycleHost::start(version, json_response).await;
            let first = host.call(30, false);
            let a = host.entered().await;
            let second = host.call(31, false);
            let b = host.entered().await;
            host.server.stop.cancel();
            bounded(a.cancelled.cancelled()).await;
            bounded(b.cancelled.cancelled()).await;
            // Listener completion alone is not handler cleanup. Both synthetic
            // handlers remain gated even after the server task has joined.
            host.finish().await;
            a.release_after_cancellation().await;
            b.release_after_cancellation().await;
            // Shutdown is not a successful tool completion; transport error or HTTP
            // error is acceptable. This does not assert a portable error envelope.
            for task in [first, second] {
                if let Ok(response) = bounded(task).await.unwrap() {
                    assert!(!bounded(response.text())
                        .await
                        .unwrap_or_default()
                        .contains("completed"));
                }
            }
        }
    }
}

#[tokio::test]
async fn fixture_gate_completes_only_after_release() {
    let mut host = LifecycleHost::start("2026-07-28", true).await;
    let request = host.call(40, false);
    let held = host.entered().await;
    assert!(!request.is_finished());
    assert!(!held.dropped.is_cancelled());
    held.release.send(()).unwrap();
    let response = bounded(request).await.unwrap().unwrap();
    let body: serde_json::Value = bounded(response.json()).await.unwrap();
    assert_eq!(
        body["result"]["structuredContent"],
        json!({"state":"completed"})
    );
    bounded(held.dropped.cancelled()).await;
    host.finish().await;
}
