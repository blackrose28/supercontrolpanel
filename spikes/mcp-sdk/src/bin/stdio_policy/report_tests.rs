use super::*;
use std::io;

const STARTUP: &[u8] = b"scpctl: MCP stdio startup failed\n";
const SERVICE: &[u8] = b"scpctl: MCP stdio service failed\n";

#[test]
fn single_literal_line_selected_without_formatting_typed_startup_error() {
    for (outcome, expected) in [
        (Err(Failure::Setup), STARTUP),
        (
            Err(Failure::Startup(Box::new(
                rmcp::service::ServerInitializeError::ConnectionClosed(
                    "SYNTHETIC_STARTUP_SECRET".into(),
                ),
            ))),
            STARTUP,
        ),
        (Err(Failure::Service), SERVICE),
    ] {
        let mut sink = Vec::new();
        assert_eq!(report_to(outcome, &mut sink), ExitCode::FAILURE);
        assert_eq!(sink, expected);
    }
    let mut sink = Vec::new();
    assert_eq!(report_to(Ok(()), &mut sink), ExitCode::SUCCESS);
    assert!(sink.is_empty());
}

struct FaultSink {
    prefix: Vec<u8>,
    allowance: usize,
    calls: usize,
    zero: bool,
}

impl Write for FaultSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.allowance == 0 {
            if self.zero {
                return Ok(0);
            }
            return Err(io::Error::other("SYNTHETIC_SINK_SECRET"));
        }
        let count = bytes.len().min(self.allowance);
        self.prefix.extend_from_slice(&bytes[..count]);
        self.allowance -= count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("report must not add a flush/report loop");
    }
}

#[test]
fn writer_failure_keeps_failure_without_panic_recursive_report_or_extra_bytes() {
    for zero in [false, true] {
        for allowance in [0, 7] {
            for (outcome, expected) in [
                (Err(Failure::Setup), STARTUP),
                (Err(Failure::Service), SERVICE),
            ] {
                let mut sink = FaultSink {
                    prefix: vec![],
                    allowance,
                    calls: 0,
                    zero,
                };
                assert_eq!(report_to(outcome, &mut sink), ExitCode::FAILURE);
                assert_eq!(sink.prefix, expected[..allowance]);
                assert_eq!(sink.calls, if allowance == 0 { 1 } else { 2 });
            }
        }
    }
}

#[test]
fn successful_outcome_never_touches_even_a_failed_sink() {
    let mut sink = FaultSink {
        prefix: vec![],
        allowance: 0,
        calls: 0,
        zero: false,
    };
    assert_eq!(report_to(Ok(()), &mut sink), ExitCode::SUCCESS);
    assert_eq!(sink.calls, 0);
}
