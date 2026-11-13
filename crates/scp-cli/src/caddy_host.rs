//! One-invocation Caddy host. Signal handlers are process-global; never embed this
//! adapter in desktop/MCP or reuse it as a general cancellable library runtime.

use std::path::Path;

pub(super) struct ShutdownPolicy {
    pub stdin_eof: bool,
    pub require_bounded: bool,
}

#[cfg(target_os = "linux")]
mod stdin_eof;

#[cfg(not(target_os = "linux"))]
pub(super) fn run(
    _: &Path,
    _: &str,
    _: &Path,
    _: bool,
    policy: ShutdownPolicy,
) -> Result<(), String> {
    let _ = (policy.stdin_eof, policy.require_bounded);
    Err("Caddy foreground host is available only in the Linux preview; Windows/macOS supervision is not qualified".into())
}

#[cfg(target_os = "linux")]
use {
    portable_pty::CommandBuilder,
    scp_core::host::{ForegroundOutcome, ProfileRuntime},
    std::{
        fs::{File, OpenOptions},
        io::{self, Write},
        os::unix::fs::OpenOptionsExt,
        sync::{atomic::AtomicBool, Arc},
    },
};

#[cfg(target_os = "linux")]
pub(super) fn run(
    profile: &Path,
    folder: &str,
    output: &Path,
    json: bool,
    policy: ShutdownPolicy,
) -> Result<(), String> {
    // No fallback to cooperative cleanup when the caller requires containment.
    // Delegation alone would not qualify workload confinement or crash retention.
    if policy.require_bounded {
        return Err("bounded shutdown is unavailable: Linux operation containment and guardian recovery are not qualified; no child was launched".into());
    }
    // Validate without creating a profile or log. Keep the desktop's path checks.
    let command = scp_core::caddy::build_caddy_command(folder)?;
    // Install before starting PTYs/worker threads, and retain through output reporting.
    let shutdown = Shutdown::install()?;
    let mut runtime = ProfileRuntime::acquire(profile)?;
    // Declared after runtime: every exit/unwind joins this observer before the
    // profile can be released. It neither reads stdin nor owns any PTY handles.
    let mut eof = if policy.stdin_eof {
        Some(stdin_eof::EofWatch::start(Arc::clone(&shutdown.cancelled))?)
    } else {
        None
    };
    let mut output = create_output(output)?;
    let result = run_prepared(&mut runtime, command, &mut output, &shutdown);
    let observed = eof.as_mut().map_or(Ok(()), stdin_eof::EofWatch::finish);
    // Preserve a PTY/output/cleanup failure over an observer failure.
    let outcome = result.and_then(|outcome| observed.map(|()| outcome))?;
    report(outcome, json, &mut io::stdout().lock())
}

#[cfg(target_os = "linux")]
fn create_output(path: &Path) -> Result<File, String> {
    // create_new rejects existing files, symlinks and special nodes. No stdout/stderr
    // locks, memory queue or detached output workers; ordinary file I/O may still block.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("create Caddy output file: {error}"))
}

#[cfg(target_os = "linux")]
fn run_prepared(
    runtime: &mut ProfileRuntime,
    command: CommandBuilder,
    output: &mut (dyn Write + Send),
    shutdown: &Shutdown,
) -> Result<ForegroundOutcome, String> {
    runtime.run_foreground_cancellable(
        &*portable_pty::native_pty_system(),
        (80, 24),
        command,
        output,
        &shutdown.cancelled,
    )
}

#[cfg(target_os = "linux")]
fn report(outcome: ForegroundOutcome, json: bool, output: &mut dyn Write) -> Result<(), String> {
    let (value, message, success) = match outcome {
        ForegroundOutcome::Completed(exit) => (
            serde_json::json!({"outcome":"completed", "exitCode":exit.exit_code}),
            format!("Caddy exited with code {:?}", exit.exit_code),
            exit.exit_code == Some(0),
        ),
        ForegroundOutcome::Cancelled => (
            serde_json::json!({"outcome":"cancelled"}),
            "Caddy cancelled".to_owned(),
            false,
        ),
    };
    if json {
        writeln!(output, "{value}")
    } else {
        writeln!(output, "{message}")
    }
    .map_err(|error| format!("write Caddy result: {error}"))?;
    if success {
        Ok(())
    } else {
        Err(message)
    }
}

#[cfg(target_os = "linux")]
struct Shutdown {
    // Never reset this flag. Registration and cancellation belong to one invocation.
    cancelled: Arc<AtomicBool>,
    registrations: Vec<signal_hook::SigId>,
}

#[cfg(target_os = "linux")]
impl Shutdown {
    fn install() -> Result<Self, String> {
        let mut shutdown = Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            registrations: Vec::new(),
        };
        for signal in [
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
        ] {
            let id = signal_hook::flag::register(signal, Arc::clone(&shutdown.cancelled))
                .map_err(|error| format!("register Caddy shutdown signal: {error}"))?;
            shutdown.registrations.push(id);
        }
        Ok(shutdown)
    }
}

#[cfg(target_os = "linux")]
impl Drop for Shutdown {
    fn drop(&mut self) {
        for id in self.registrations.drain(..) {
            signal_hook::low_level::unregister(id);
        }
        // signal-hook removes actions, not the previous/default signal disposition.
        // This adapter is only for a CLI invocation that exits immediately afterwards.
        // Repeated signals request cancellation again; they never force-exit past guards.
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
