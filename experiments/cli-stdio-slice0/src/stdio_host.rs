use crate::supervisor_spike::{ChildKind, OwnershipOutcome, Supervisor};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    JsonObject, ListResourcesResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, ServerInitializeError};
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::transport::io::stdio_pipes;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::json;
use std::borrow::Cow;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

const DEFAULT_MAX_LINE_BYTES: usize = 1_048_576;
const DEFAULT_MAX_WORKERS: usize = 8;
const DEFAULT_WRITE_TIMEOUT_MS: u64 = 5_000;
const STDIN_EOF_GRACE: Duration = Duration::from_millis(150);
const CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(2);

struct EofWatchReader<R> {
    inner: R,
    eof_hit: Arc<AtomicBool>,
    eof_notify: Arc<Notify>,
}

impl<R: AsyncRead + Unpin + Send + 'static> AsyncRead for EofWatchReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let remaining = buf.remaining();
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) if remaining > 0 && buf.filled().len() == before => {
                if !self.eof_hit.swap(true, Ordering::SeqCst) {
                    self.eof_notify.notify_waiters();
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(err)) => {
                if !self.eof_hit.swap(true, Ordering::SeqCst) {
                    self.eof_notify.notify_waiters();
                }
                Poll::Ready(Err(err))
            }
            other => other,
        }
    }
}

struct TimedAsyncWriter<W> {
    inner: W,
    timeout: Duration,
    deadline: Option<Pin<Box<tokio::time::Sleep>>>,
    abort: CancellationToken,
}

impl<W> TimedAsyncWriter<W> {
    fn new(inner: W, timeout: Duration, abort: CancellationToken) -> Self {
        Self {
            inner,
            timeout,
            deadline: None,
            abort,
        }
    }
}

fn poll_stall_deadline(
    deadline: &mut Option<Pin<Box<tokio::time::Sleep>>>,
    timeout: Duration,
    abort: &CancellationToken,
    cx: &mut Context<'_>,
) -> Option<io::Error> {
    let sleep = deadline.get_or_insert_with(|| Box::pin(tokio::time::sleep(timeout)));
    if sleep.as_mut().poll(cx).is_ready() {
        *deadline = None;
        abort.cancel();
        Some(io::Error::new(
            io::ErrorKind::TimedOut,
            "stdout writer stalled past write timeout",
        ))
    } else {
        None
    }
}

impl<W: AsyncWrite + Unpin + Send + 'static> AsyncWrite for TimedAsyncWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(res) => {
                this.deadline = None;
                Poll::Ready(res)
            }
            Poll::Pending => {
                if let Some(err) =
                    poll_stall_deadline(&mut this.deadline, this.timeout, &this.abort, cx)
                {
                    Poll::Ready(Err(err))
                } else {
                    Poll::Pending
                }
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_flush(cx) {
            Poll::Ready(res) => {
                this.deadline = None;
                Poll::Ready(res)
            }
            Poll::Pending => {
                if let Some(err) =
                    poll_stall_deadline(&mut this.deadline, this.timeout, &this.abort, cx)
                {
                    Poll::Ready(Err(err))
                } else {
                    Poll::Pending
                }
            }
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[derive(Clone)]
struct StdioProbeHost {
    supervisor: Arc<Supervisor>,
    workers: Arc<Semaphore>,
    max_workers: usize,
    active_workers: Arc<AtomicUsize>,
    shutdown_ct: CancellationToken,
    resources: Arc<Vec<Resource>>,
}

struct ActiveHoldGuard {
    active_workers: Arc<AtomicUsize>,
    supervisor: Arc<Supervisor>,
    session_id: Option<String>,
}

impl Drop for ActiveHoldGuard {
    fn drop(&mut self) {
        self.active_workers.fetch_sub(1, Ordering::SeqCst);
        if let Some(id) = self.session_id.take() {
            let sup = Arc::clone(&self.supervisor);
            std::thread::spawn(move || {
                let _ = sup.stop_session(&id);
            });
        }
    }
}

fn value_object_schema(include_reaped: bool) -> JsonObject {
    let mut properties = serde_json::Map::new();
    properties.insert("value".to_string(), json!({ "type": "string" }));
    let mut required = vec![serde_json::Value::String("value".to_string())];
    if include_reaped {
        properties.insert("noisyChildReaped".to_string(), json!({ "type": "boolean" }));
        required.push(serde_json::Value::String("noisyChildReaped".to_string()));
    }
    let mut root = serde_json::Map::new();
    root.insert("type".to_string(), json!("object"));
    root.insert(
        "properties".to_string(),
        serde_json::Value::Object(properties),
    );
    root.insert("required".to_string(), serde_json::Value::Array(required));
    root.insert("additionalProperties".to_string(), json!(false));
    root
}

fn build_tool_catalog() -> Vec<Tool> {
    let input_schema = Arc::new(value_object_schema(false));
    let echo_output = Arc::new(value_object_schema(true));
    vec![
        Tool::new(
            "probe_echo",
            "Isolated stdio probe echo supporting deny and noisy pipe/PTY children",
            Arc::clone(&input_schema),
        )
        .with_raw_output_schema(echo_output),
        Tool::new(
            "probe_hold",
            "Isolated stdio probe hold testing worker saturation, abort signals, and child reaping",
            input_schema,
        ),
    ]
}

fn extract_required_value(params: &CallToolRequestParams) -> Result<String, ErrorData> {
    params
        .arguments
        .as_ref()
        .and_then(|map| map.get("value"))
        .and_then(|val| val.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| ErrorData::invalid_params("`value` argument must be a string", None))
}

async fn run_noisy_child_probe(supervisor: Arc<Supervisor>) -> Result<(), ErrorData> {
    tokio::task::spawn_blocking(move || {
        let pipe_out = supervisor
            .spawn_and_register(
                ChildKind::Pipe,
                "sh",
                &[
                    "-c",
                    "printf 'pipe-stdout-noise\\n'; printf 'pipe-stderr-noise\\n' >&2",
                ],
                false,
            )
            .map_err(|e| ErrorData::internal_error(e.message, None))?;
        let pty_out = supervisor
            .spawn_and_register(
                ChildKind::Pty,
                "sh",
                &["-c", "printf 'pty-noise\\n'"],
                false,
            )
            .map_err(|e| ErrorData::internal_error(e.message, None))?;

        let (OwnershipOutcome::Accepted(pipe_id), OwnershipOutcome::Accepted(pty_id)) =
            (pipe_out, pty_out)
        else {
            return Err(ErrorData::internal_error(
                "unexpected pre-accept cancellation during noisy probe",
                None,
            ));
        };

        let deadline = Instant::now() + Duration::from_millis(600);
        while Instant::now() < deadline {
            let pipe_ready = supervisor
                .output_snapshot(&pipe_id)
                .is_some_and(|s| s.contains("pipe-stdout-noise"));
            let pty_ready = supervisor
                .output_snapshot(&pty_id)
                .is_some_and(|s| s.contains("pty-noise"));
            if pipe_ready && pty_ready {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }

        let _ = supervisor.stop_session(&pipe_id);
        let _ = supervisor.stop_session(&pty_id);
        Ok(())
    })
    .await
    .map_err(|e| ErrorData::internal_error(format!("noisy probe task join failed: {e}"), None))?
}

impl StdioProbeHost {
    async fn handle_echo(&self, value: String) -> Result<CallToolResponse, ErrorData> {
        if value == "deny" {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "POLICY_DENIED: stdio spike denial",
            )])
            .into());
        }
        let noisy_reaped = if value == "noisy-child" {
            run_noisy_child_probe(Arc::clone(&self.supervisor)).await?;
            true
        } else {
            false
        };
        Ok(CallToolResult::structured(json!({
            "value": value,
            "noisyChildReaped": noisy_reaped,
        }))
        .into())
    }

    async fn handle_hold(
        &self,
        value: String,
        context: &RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.active_workers.fetch_add(1, Ordering::SeqCst);
        let mut guard = ActiveHoldGuard {
            active_workers: Arc::clone(&self.active_workers),
            supervisor: Arc::clone(&self.supervisor),
            session_id: None,
        };

        if value == "hold-child" {
            let sup = Arc::clone(&self.supervisor);
            let outcome = tokio::task::spawn_blocking(move || {
                sup.spawn_and_register(ChildKind::Pipe, "sleep", &["30"], false)
            })
            .await
            .map_err(|e| ErrorData::internal_error(format!("hold spawn join failed: {e}"), None))?
            .map_err(|e| ErrorData::internal_error(e.message, None))?;
            if let OwnershipOutcome::Accepted(session_id) = outcome {
                guard.session_id = Some(session_id);
            }
        }

        tokio::select! {
            _ = context.ct.cancelled() => {
                stop_held_session(&mut guard).await;
                Ok(CallToolResult::error(vec![ContentBlock::text("CANCELLED")]).into())
            }
            _ = self.shutdown_ct.cancelled() => {
                stop_held_session(&mut guard).await;
                Ok(CallToolResult::error(vec![ContentBlock::text("CANCELLED: shutdown")]).into())
            }
            _ = tokio::time::sleep(Duration::from_secs(30)) => {
                stop_held_session(&mut guard).await;
                Ok(CallToolResult::structured(json!({
                    "value": value,
                    "noisyChildReaped": false,
                })).into())
            }
        }
    }
}

async fn stop_held_session(guard: &mut ActiveHoldGuard) {
    if let Some(session_id) = guard.session_id.take() {
        let sup = Arc::clone(&guard.supervisor);
        let _ = tokio::task::spawn_blocking(move || sup.stop_session(&session_id)).await;
    }
}

const STDIO_PROTOCOL_VERSIONS: [ProtocolVersion; 2] =
    [ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25];

const PROBE_RESOURCE_DEFS: [(&str, &str); 2] = [
    ("probe://status", "stdio-probe-status"),
    ("probe://active", "stdio-probe-active"),
];

impl ServerHandler for StdioProbeHost {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();
        ServerConfig::new(capabilities)
            .with_server_info(Implementation::new("scpctl-stdio-spike", "0.1.0"))
            .with_instructions("Isolated stdio slice0 probe server")
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&STDIO_PROTOCOL_VERSIONS)
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(build_tool_catalog()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool_name = request.name.as_ref();
        if tool_name != "probe_echo" && tool_name != "probe_hold" {
            return Err(ErrorData::invalid_params("Unknown stdio probe tool", None));
        }
        let value = extract_required_value(&request)?;

        let _permit = tokio::select! {
            acquired = Arc::clone(&self.workers).acquire_owned() => {
                acquired.map_err(|_| ErrorData::internal_error("worker semaphore closed", None))?
            }
            _ = context.ct.cancelled() => {
                return Ok(CallToolResult::error(vec![ContentBlock::text("CANCELLED")]).into());
            }
            _ = self.shutdown_ct.cancelled() => {
                return Ok(CallToolResult::error(vec![ContentBlock::text("CANCELLED: shutdown")]).into());
            }
        };

        if tool_name == "probe_hold" {
            self.handle_hold(value, &context).await
        } else {
            self.handle_echo(value).await
        }
    }

    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(
            (*self.resources).clone(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let body = match request.uri.as_str() {
            "probe://status" => json!({
                "transport": "stdio",
                "server": "scpctl-stdio-spike",
            })
            .to_string(),
            "probe://active" => {
                let child_pids: Vec<u32> = self
                    .supervisor
                    .active_sessions()
                    .into_iter()
                    .map(|(_, pid)| pid)
                    .collect();
                json!({
                    "active": self.active_workers.load(Ordering::SeqCst),
                    "maxWorkers": self.max_workers,
                    "childPids": child_pids,
                })
                .to_string()
            }
            _ => {
                return Err(ErrorData::invalid_params(
                    "Unknown stdio probe resource",
                    None,
                ));
            }
        };
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(body, request.uri).with_mime_type("application/json"),
        ])
        .into())
    }
}

fn env_usize_min(var: &str, default: usize, min_val: usize) -> usize {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .map(|n| n.max(min_val))
        .unwrap_or(default)
}

async fn wait_for_termination_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

async fn run_stdio_event_loop() -> ExitCode {
    let max_line_bytes = env_usize_min("SCP_STDIO_MAX_LINE_BYTES", DEFAULT_MAX_LINE_BYTES, 256);
    let max_workers = env_usize_min("SCP_STDIO_MAX_WORKERS", DEFAULT_MAX_WORKERS, 1);
    let write_timeout_ms = env_usize_min(
        "SCP_STDIO_WRITE_TIMEOUT_MS",
        DEFAULT_WRITE_TIMEOUT_MS as usize,
        50,
    ) as u64;

    let supervisor = Arc::new(Supervisor::new());
    let shutdown_ct = CancellationToken::new();
    let writer_abort = CancellationToken::new();
    let resources = Arc::new(
        PROBE_RESOURCE_DEFS
            .iter()
            .map(|(uri, name)| Resource::new(*uri, *name).with_mime_type("application/json"))
            .collect(),
    );
    let host = StdioProbeHost {
        supervisor: Arc::clone(&supervisor),
        workers: Arc::new(Semaphore::new(max_workers)),
        max_workers,
        active_workers: Arc::new(AtomicUsize::new(0)),
        shutdown_ct: shutdown_ct.clone(),
        resources,
    };

    let (raw_reader, raw_writer) = stdio_pipes();
    let eof_hit = Arc::new(AtomicBool::new(false));
    let eof_notify = Arc::new(Notify::new());
    let reader = EofWatchReader {
        inner: raw_reader,
        eof_hit: Arc::clone(&eof_hit),
        eof_notify: Arc::clone(&eof_notify),
    };
    let timed_writer = TimedAsyncWriter::new(
        raw_writer,
        Duration::from_millis(write_timeout_ms),
        writer_abort.clone(),
    );
    let transport =
        AsyncRwTransport::new_server(reader, timed_writer).with_max_line_length(max_line_bytes);

    let running = match host.serve(transport).await {
        Ok(running) => running,
        Err(ServerInitializeError::ConnectionClosed(_)) => {
            supervisor.shutdown(CHILD_REAP_TIMEOUT);
            return if writer_abort.is_cancelled() {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            };
        }
        Err(err) => {
            eprintln!("stdio serve initialization failed: {err}");
            supervisor.shutdown(CHILD_REAP_TIMEOUT);
            return ExitCode::from(1);
        }
    };

    let cancel_handle = running.cancellation_token();
    let wait_fut = running.waiting();
    tokio::pin!(wait_fut);
    let eof_grace_fut = async {
        if !eof_hit.load(Ordering::SeqCst) {
            eof_notify.notified().await;
        }
        tokio::time::sleep(STDIN_EOF_GRACE).await;
    };
    let exit_code = tokio::select! {
        wait_res = &mut wait_fut => {
            shutdown_ct.cancel();
            if writer_abort.is_cancelled() {
                ExitCode::from(1)
            } else {
                match wait_res {
                    Ok(_) => ExitCode::SUCCESS,
                    Err(err) => {
                        eprintln!("stdio service task failed: {err}");
                        ExitCode::from(1)
                    }
                }
            }
        }
        _ = eof_grace_fut => {
            shutdown_ct.cancel();
            cancel_handle.cancel();
            let _ = tokio::time::timeout(Duration::from_millis(300), &mut wait_fut).await;
            ExitCode::SUCCESS
        }
        _ = writer_abort.cancelled() => {
            shutdown_ct.cancel();
            cancel_handle.cancel();
            eprintln!("stdio host aborted after stdout writer stall");
            ExitCode::from(1)
        }
        _ = wait_for_termination_signal() => {
            shutdown_ct.cancel();
            cancel_handle.cancel();
            ExitCode::SUCCESS
        }
    };

    supervisor.shutdown(CHILD_REAP_TIMEOUT);
    exit_code
}

pub fn run_stdio_host() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start tokio runtime for stdio host: {err}");
            return ExitCode::from(1);
        }
    };

    let exit_code = runtime.block_on(run_stdio_event_loop());
    runtime.shutdown_timeout(Duration::from_millis(200));
    exit_code
}
