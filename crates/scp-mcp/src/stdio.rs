//! Bounded newline-delimited JSON-RPC stdio host wrapping `SdkHost` and `rmcp 3.9.0`.
use crate::protocol::SdkHost;
use rmcp::service::ServerInitializeError;
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::transport::io::stdio_pipes;
use rmcp::ServiceExt;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::KeyringBackend;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const DEFAULT_MAX_LINE_BYTES: usize = 1_048_576;
const DEFAULT_WRITE_TIMEOUT_MS: u64 = 5_000;
const STDIN_EOF_GRACE: Duration = Duration::from_millis(150);

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

fn handle_timed_poll<T>(
    deadline: &mut Option<Pin<Box<tokio::time::Sleep>>>,
    timeout: Duration,
    abort: &CancellationToken,
    cx: &mut Context<'_>,
    res: Poll<io::Result<T>>,
) -> Poll<io::Result<T>> {
    match res {
        Poll::Ready(val) => {
            *deadline = None;
            Poll::Ready(val)
        }
        Poll::Pending => {
            let sleep = deadline.get_or_insert_with(|| Box::pin(tokio::time::sleep(timeout)));
            if sleep.as_mut().poll(cx).is_ready() {
                *deadline = None;
                abort.cancel();
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "stdout writer stalled past write timeout",
                )))
            } else {
                Poll::Pending
            }
        }
    }
}

impl<W: AsyncWrite + Unpin + Send + 'static> AsyncWrite for TimedAsyncWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        let res = Pin::new(&mut this.inner).poll_write(cx, buf);
        handle_timed_poll(&mut this.deadline, this.timeout, &this.abort, cx, res)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        let res = Pin::new(&mut this.inner).poll_flush(cx);
        handle_timed_poll(&mut this.deadline, this.timeout, &this.abort, cx, res)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

enum TerminationSignal {
    Interrupt,
    Terminate,
}

async fn wait_for_termination_signal() -> TerminationSignal {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            return tokio::select! {
                _ = tokio::signal::ctrl_c() => TerminationSignal::Interrupt,
                _ = sigterm.recv() => TerminationSignal::Terminate,
            };
        }
    }
    let _ = tokio::signal::ctrl_c().await;
    TerminationSignal::Interrupt
}

fn env_usize_min(key: &str, default: usize, min: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|v| v.max(min))
        .unwrap_or(default)
}

pub fn serve_stdio_runtime<B: KeyringBackend + 'static>(
    runtime: Arc<ProfileRuntime<B>>,
    version: &'static str,
) -> i32 {
    let tokio_rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start tokio runtime for stdio host: {err}");
            return 1;
        }
    };
    let code = tokio_rt.block_on(async move {
        let (stdin, stdout) = stdio_pipes();
        serve_stdio_io(runtime, version, stdin, stdout).await
    });
    tokio_rt.shutdown_timeout(Duration::from_millis(200));
    code
}

pub async fn serve_stdio_io<B, R, W>(
    runtime: Arc<ProfileRuntime<B>>,
    version: &'static str,
    raw_reader: R,
    raw_writer: W,
) -> i32
where
    B: KeyringBackend + 'static,
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let max_line = env_usize_min("SCP_STDIO_MAX_LINE_BYTES", DEFAULT_MAX_LINE_BYTES, 256);
    let write_timeout_ms = env_usize_min(
        "SCP_STDIO_WRITE_TIMEOUT_MS",
        DEFAULT_WRITE_TIMEOUT_MS as usize,
        50,
    ) as u64;

    let host = SdkHost::standalone(runtime, version);
    let stop = host.control.stop.clone();
    let writer_abort = CancellationToken::new();
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
        AsyncRwTransport::new_server(reader, timed_writer).with_max_line_length(max_line);

    let running = match host.serve(transport).await {
        Ok(running) => running,
        Err(ServerInitializeError::ConnectionClosed(_)) => {
            stop.cancel();
            return if writer_abort.is_cancelled() { 1 } else { 0 };
        }
        Err(err) => {
            stop.cancel();
            eprintln!("stdio serve initialization failed: {err}");
            return 1;
        }
    };

    drive_stdio_session(running, stop, writer_abort, eof_hit, eof_notify).await
}

async fn drive_stdio_session(
    running: rmcp::service::RunningService<rmcp::RoleServer, SdkHost>,
    stop: CancellationToken,
    writer_abort: CancellationToken,
    eof_hit: Arc<AtomicBool>,
    eof_notify: Arc<Notify>,
) -> i32 {
    let cancel_handle = running.cancellation_token();
    let wait_fut = running.waiting();
    tokio::pin!(wait_fut);
    let eof_grace = async {
        if !eof_hit.load(Ordering::SeqCst) {
            eof_notify.notified().await;
        }
        tokio::time::sleep(STDIN_EOF_GRACE).await;
    };

    tokio::select! {
        res = &mut wait_fut => {
            stop.cancel();
            if writer_abort.is_cancelled() {
                1
            } else if let Err(err) = res {
                eprintln!("stdio service task failed: {err}");
                1
            } else {
                0
            }
        }
        _ = eof_grace => {
            stop.cancel();
            cancel_handle.cancel();
            let _ = tokio::time::timeout(Duration::from_millis(300), &mut wait_fut).await;
            0
        }
        _ = writer_abort.cancelled() => {
            stop.cancel();
            cancel_handle.cancel();
            eprintln!("stdio host aborted after stdout writer stall");
            1
        }
        sig = wait_for_termination_signal() => {
            stop.cancel();
            cancel_handle.cancel();
            let _ = tokio::time::timeout(Duration::from_millis(300), &mut wait_fut).await;
            match sig {
                TerminationSignal::Interrupt => 130,
                TerminationSignal::Terminate => 0,
            }
        }
    }
}
