use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn cancelled_pending_work_never_runs() {
    let control = TransportControl::default();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let called = Arc::new(AtomicUsize::new(0));
    let count = called.clone();
    let result = control
        .run(cancel, move || {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await;
    assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(called.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn capacity_is_held_until_accepted_work_finishes_after_disconnect() {
    let control = TransportControl::default();
    let capacity = Arc::new(tokio::sync::Semaphore::new(1));
    let control = TransportControl {
        workers: capacity.clone(),
        ..control
    };
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker_control = control.clone();
    let request = tokio::spawn(async move {
        worker_control
            .run(CancellationToken::new(), move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
            .await
    });
    started_rx.await.unwrap();
    request.abort();
    assert_eq!(
        control
            .run(CancellationToken::new(), || Ok(()))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while capacity.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn slow_consumers_coalesce_without_blocking_producers() {
    let control = TransportControl::default();
    let mut receiver = control.updates.subscribe();
    for _ in 0..100_000 {
        control.invalidate();
    }
    receiver.changed().await.unwrap();
    assert_eq!(*receiver.borrow_and_update(), 100_000);
    assert!(!receiver.has_changed().unwrap());
}

#[tokio::test]
async fn revoked_listener_rejects_pending_work() {
    let control = TransportControl::default();
    control.stop.cancel();
    assert_eq!(
        control
            .run(CancellationToken::new(), || Ok(()))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
}

#[test]
fn dropped_request_cannot_run_later_from_blocking_queue() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let blocker = runtime.spawn_blocking(move || {
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    started_rx.recv().unwrap();
    let called = Arc::new(AtomicUsize::new(0));
    let count = called.clone();
    runtime.block_on(async {
        let control = TransportControl::default();
        let request = tokio::spawn(async move {
            control
                .run(CancellationToken::new(), move || {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .await
        });
        tokio::task::yield_now().await;
        request.abort();
        let _ = request.await;
        release_tx.send(()).unwrap();
        blocker.await.unwrap();
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    assert_eq!(called.load(Ordering::SeqCst), 0);
}

#[test]
fn backup_maintenance_and_recovery_reject_worker_mutations() {
    // Recovery's process-wide latch is deliberately irreversible; isolate it from other tests.
    if std::env::var_os("SLICE4_GATE_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "mcp::lifecycle::tests::backup_maintenance_and_recovery_reject_worker_mutations",
                "--nocapture",
            ])
            .env("SLICE4_GATE_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let control = TransportControl::default();
    let mutations = Arc::new(AtomicUsize::new(0));
    let mut maintenance = Some(crate::settings_backup::gate::maintain().unwrap());
    for recovery in [false, true] {
        if recovery {
            drop(maintenance.take());
            crate::settings_backup::gate::block();
        }
        let count = mutations.clone();
        let result = runtime.block_on(control.run(CancellationToken::new(), move || {
            let _lease = crate::settings_backup::gate::enter()
                .map_err(|_| OperationError::new(ErrorCode::Conflict))?;
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
    }
    control.stop.cancel();
    drop(maintenance);
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
}
