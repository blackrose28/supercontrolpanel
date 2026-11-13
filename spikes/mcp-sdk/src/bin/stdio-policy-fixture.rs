//! Isolated executable policy fixture; no production scpctl entry point.
#[path = "stdio_policy/probes.rs"]
mod probes;
#[path = "stdio_policy/runner.rs"]
mod runner;

use runner::Failure;
use std::{io::Write, process::ExitCode};
use tracing::subscriber::{set_global_default, NoSubscriber};

// Synchronous entry: no Tokio runtime, SDK transport, input or service exists yet.
fn main() -> ExitCode {
    let mode = std::env::args().nth(1).unwrap_or_default();
    // Fixture-only conflict injection; the real policy still performs its own install.
    if mode == "conflict" && set_global_default(NoSubscriber::new()).is_err() {
        return report(Err(Failure::Setup));
    }
    if set_global_default(NoSubscriber::new()).is_err() {
        return report(Err(Failure::Setup));
    }
    // Test tripwire BEFORE runtime construction: exact stderr catches any crossing.
    assert!(
        mode != "conflict",
        "fixture crossed rejected subscriber installation"
    );
    if mode == "levels" {
        probes::all_levels();
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return report(Err(Failure::Setup)),
    };
    let outcome = runtime.block_on(async {
        if mode == "levels" && tokio::spawn(async { probes::all_levels() }).await.is_err() {
            return Err(Failure::Setup);
        }
        let (input, output) = rmcp::transport::stdio_pipes();
        runner::serve(input, output).await
    });
    // No detached logging worker/flush guard. Runtime settles before reporting.
    drop(runtime);
    if mode == "writer-failure" {
        report_to(outcome, &mut RefusingDiagnostic)
    } else {
        report(outcome)
    }
}

fn report(outcome: Result<(), Failure>) -> ExitCode {
    report_to(outcome, &mut std::io::stderr().lock())
}

fn report_to(outcome: Result<(), Failure>, sink: &mut impl Write) -> ExitCode {
    let diagnostic: &[u8] = match outcome {
        Ok(()) => return ExitCode::SUCCESS,
        Err(Failure::Setup) => b"scpctl: MCP stdio startup failed\n",
        Err(Failure::Startup(error)) => {
            // Preserve the typed error until classification; never format it.
            drop(error);
            b"scpctl: MCP stdio startup failed\n"
        }
        Err(Failure::Service) => b"scpctl: MCP stdio service failed\n",
    };
    // A blocked sink may block. A failed sink never recurses, panics or uses stdout.
    let _ = sink.write_all(diagnostic);
    ExitCode::FAILURE
}

// Fixture-only process injection; no new production sink selection or authority.
struct RefusingDiagnostic;
impl Write for RefusingDiagnostic {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

#[cfg(test)]
#[path = "stdio_policy/report_tests.rs"]
mod report_tests;
