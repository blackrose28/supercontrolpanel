use rmcp::{
    service::{QuitReason, ServerInitializeError},
    ServiceExt,
};
use scp_core::mcp_dto::McpAppOverviewResult;
use scp_mcp_sdk_spike::overview::{OverviewAdapter, OverviewHost, OverviewPolicy};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

pub(super) enum Failure {
    Setup,
    Startup(Box<ServerInitializeError>),
    Service,
}

pub(super) async fn serve<R, W>(input: R, output: W) -> Result<(), Failure>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let adapter = OverviewAdapter::new(
        UnreachableHost,
        OverviewPolicy {
            allowed_categories: vec!["read-only".into()],
            sensitive_action_policy: "require-ui-approval".into(),
        },
    );
    let service = adapter
        .serve((input, output))
        .await
        .map_err(|error| Failure::Startup(Box::new(error)))?;
    // This synthetic host owns no native work. Closed is not native cleanup proof.
    finish(service.waiting().await, false, async { Ok(()) }).await
}

// Private boundary seam; owned cleanup is synthetic in this isolated fixture.
async fn finish(
    waiting: Result<QuitReason, tokio::task::JoinError>,
    host_requested: bool,
    cleanup: impl std::future::Future<Output = Result<(), ()>>,
) -> Result<(), Failure> {
    // Always settle owned cleanup, including on either SDK task-error layer.
    let cleanup = cleanup.await;
    match (waiting, cleanup) {
        (Ok(QuitReason::Closed), Ok(())) => Ok(()),
        (Ok(QuitReason::Cancelled), Ok(())) if host_requested => Ok(()),
        // Default deny includes both JoinError layers, unexpected cancellation,
        // cleanup failure and any future non-exhaustive SDK variant.
        _ => Err(Failure::Service),
    }
}

#[derive(Clone)]
struct UnreachableHost;

impl OverviewHost for UnreachableHost {
    async fn read_overview(&self, _: CancellationToken) -> Result<McpAppOverviewResult, String> {
        // Fixture tripwire: none of these probes may enter a host operation.
        panic!("policy fixture unexpectedly entered host");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ClientJsonRpcMessage;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn policy_runner_retains_original_typed_startup_peer_error() {
        let (mut peer, input) = tokio::io::duplex(4096);
        peer.write_all(concat!("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,",
            "\"message\":\"SYNTHETIC_STDIO_SECRET\",\"data\":{\"password\":\"SYNTHETIC_STDIO_SECRET\"}}}\n")
            .as_bytes()).await.unwrap();
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            serve(input, tokio::io::sink()),
        )
        .await
        .expect("typed startup fixture watchdog");
        // No Debug, Display, source traversal or error-to-string conversion.
        let Err(Failure::Startup(error)) = outcome else {
            panic!("typed startup failure required");
        };
        let ServerInitializeError::ExpectedInitializeRequest(Some(ClientJsonRpcMessage::Error(
            message,
        ))) = *error
        else {
            panic!("original typed startup peer error required");
        };
        assert!(message.id.is_none());
        assert_eq!(message.error.code.0, -32600);
        assert_eq!(message.error.message, "SYNTHETIC_STDIO_SECRET");
        assert_eq!(
            message.error.data,
            Some(serde_json::json!({"password":"SYNTHETIC_STDIO_SECRET"}))
        );
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::*;
    use std::{cell::Cell, time::Duration};

    fn assert_service_line(result: Result<(), Failure>) {
        let mut sink = Vec::new();
        assert_eq!(
            super::super::report_to(result, &mut sink),
            std::process::ExitCode::FAILURE
        );
        assert_eq!(sink, b"scpctl: MCP stdio service failed\n");
    }

    async fn aborted() -> tokio::task::JoinError {
        let task = tokio::spawn(std::future::pending::<()>());
        task.abort();
        let error = task.await.expect_err("aborted task must fail");
        assert!(error.is_cancelled());
        assert!(!error.is_panic());
        error
    }

    #[tokio::test]
    async fn outer_join_error_fails_after_cleanup_even_if_host_requested() {
        for requested in [false, true] {
            let settled = Cell::new(false);
            let result = finish(Err(aborted().await), requested, async {
                settled.set(true);
                Ok(())
            })
            .await;
            assert!(matches!(result, Err(Failure::Service)));
            assert!(settled.get(), "outer error bypassed owned cleanup");
            assert_service_line(result);
        }
    }

    #[tokio::test]
    async fn inner_join_error_fails_after_cleanup_even_if_host_requested() {
        for requested in [false, true] {
            let settled = Cell::new(false);
            let result = finish(
                Ok(QuitReason::JoinError(aborted().await)),
                requested,
                async {
                    settled.set(true);
                    Ok(())
                },
            )
            .await;
            assert!(matches!(result, Err(Failure::Service)));
            assert!(settled.get(), "inner error bypassed owned cleanup");
            assert_service_line(result);
        }
    }

    #[tokio::test]
    async fn cancellation_requires_explicit_owner_request() {
        for requested in [false, true] {
            let result = finish(Ok(QuitReason::Cancelled), requested, async { Ok(()) }).await;
            assert_eq!(result.is_ok(), requested);
        }
    }

    #[tokio::test]
    async fn success_waits_for_owned_cleanup_and_cleanup_failure_wins() {
        for cancelled in [false, true] {
            let reason = if cancelled {
                QuitReason::Cancelled
            } else {
                QuitReason::Closed
            };
            let (release, gate) = tokio::sync::oneshot::channel();
            let (entered, ready) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(finish(Ok(reason), true, async {
                entered.send(()).unwrap();
                gate.await.unwrap();
                Ok(())
            }));
            tokio::time::timeout(Duration::from_secs(5), ready)
                .await
                .unwrap()
                .unwrap();
            assert!(
                !task.is_finished(),
                "success escaped before owned cleanup settled"
            );
            release.send(()).unwrap();
            assert!(task.await.unwrap().is_ok());
            let reason = if cancelled {
                QuitReason::Cancelled
            } else {
                QuitReason::Closed
            };
            assert_service_line(finish(Ok(reason), true, async { Err(()) }).await);
        }
    }
}

#[cfg(test)]
mod sdk_cancellation_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn actual_sdk_cancellation_preserves_explicit_request_origin() {
        for requested in [false, true] {
            let (mut peer, input) = tokio::io::duplex(4096);
            peer.write_all(
                concat!(
                    "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",",
                    "\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},",
                    "\"clientInfo\":{\"name\":\"policy-cancellation\",\"version\":\"0\"}}}\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
            let external = CancellationToken::new();
            let adapter = OverviewAdapter::new(
                UnreachableHost,
                OverviewPolicy {
                    allowed_categories: vec!["read-only".into()],
                    sensitive_action_policy: "require-ui-approval".into(),
                },
            );
            let service = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                adapter.serve_with_ct((input, tokio::io::sink()), external.clone()),
            )
            .await
            .expect("SDK selection watchdog")
            .unwrap_or_else(|_| panic!("SDK must select service"));
            if requested {
                // Explicit action of this synthetic owner; never infer from token state.
                service.cancellation_token().cancel();
            } else {
                // Cancellation from outside the owner is not an expected host shutdown.
                external.cancel();
            }
            let waiting =
                tokio::time::timeout(std::time::Duration::from_secs(5), service.waiting())
                    .await
                    .expect("SDK cancellation watchdog");
            assert!(matches!(waiting, Ok(QuitReason::Cancelled)));
            let settled = std::cell::Cell::new(false);
            let outcome = finish(waiting, requested, async {
                settled.set(true);
                Ok(())
            })
            .await;
            assert!(settled.get());
            let mut sink = Vec::new();
            let status = super::super::report_to(outcome, &mut sink);
            assert_eq!(
                status,
                if requested {
                    std::process::ExitCode::SUCCESS
                } else {
                    std::process::ExitCode::FAILURE
                }
            );
            assert_eq!(
                sink,
                if requested {
                    &b""[..]
                } else {
                    &b"scpctl: MCP stdio service failed\n"[..]
                }
            );
        }
    }
}
