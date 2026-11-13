use axum::Router;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Test-owned listener; cancellation and abort fallback never promise handler cleanup.
pub struct BoundServer {
    pub url: String,
    pub stop: CancellationToken,
    task: JoinHandle<std::io::Result<()>>,
}

impl BoundServer {
    pub async fn bind(router: Router, stop: CancellationToken) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let shutdown = stop.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
        });
        Self { url, stop, task }
    }

    pub async fn finish(mut self) {
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), &mut self.task)
            .await
            .expect("listener fixture cleanup expired")
            .unwrap()
            .unwrap();
    }
}

impl Drop for BoundServer {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}
