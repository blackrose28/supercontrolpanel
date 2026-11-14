use crate::args::{CommandRequest, Subcommand};
use scp_core::error::CoreError;
use scp_core::models::{StartFunctionResult, BOUNDARY_AUTH_FUNCTION_ID};
use scp_core::openvpn_ops::detect_openvpn_executable_checked;
use scp_core::profile::OwnedProfileLocation;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::OsKeyringBackend;
use scp_core::supervisor::CancelToken;
use scp_core::tty::{
    is_interactive_tty, terminate_child, SignalWatcher, TerminalModeGuard, TerminationSignal,
};
use scp_mcp::public_dto::{SafeFunctionStatus, SafeTransfer};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const BOUNDARY_AUTH_TIMEOUT: Duration = Duration::from_secs(30);

struct CancelPump {
    done: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl CancelPump {
    fn spawn(watcher: &Arc<SignalWatcher>, cancel: &CancelToken) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let done_bg = Arc::clone(&done);
        let w = Arc::clone(watcher);
        let c = cancel.clone();
        let handle = std::thread::spawn(move || {
            while !done_bg.load(Ordering::Relaxed) {
                if w.signal().is_some() {
                    c.cancel();
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        });
        Self {
            done,
            handle: Some(handle),
        }
    }
}

impl Drop for CancelPump {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn install_signal_watcher() -> Result<Arc<SignalWatcher>, CoreError> {
    SignalWatcher::install().map(Arc::new).map_err(|e| {
        CoreError::operation_failed("signal_setup_failed", e.to_string(), false, "retry_later")
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn stopped_status_json(function_id: &str) -> Value {
    json!(SafeFunctionStatus {
        function_id: function_id.to_string(),
        state: "stopped".to_string(),
        message: String::new(),
        boundary_token_source: None,
        session_id: None,
        updated_at_ms: now_ms(),
    })
}

pub fn openvpn_detect() -> Result<Value, CoreError> {
    match detect_openvpn_executable_checked() {
        Ok(Some(path)) => Ok(json!({ "executablePath": path })),
        Ok(None) => Err(CoreError::not_found(
            "openvpn_not_found",
            "OpenVPN executable not found on this host",
            "install_or_configure_openvpn",
        )),
        Err(msg) => Err(CoreError::operation_failed(
            "openvpn_detect_failed",
            msg,
            false,
            "check_openvpn_installation",
        )),
    }
}

pub fn run_foreground(
    rt: &ProfileRuntime<OsKeyringBackend>,
    cmd: Subcommand,
) -> Result<Value, CoreError> {
    let watcher = install_signal_watcher()?;
    let cancel = CancelToken::new();
    let pump = CancelPump::spawn(&watcher, &cancel);
    let started = match cmd {
        Subcommand::OpenvpnStart => rt.start_openvpn(&cancel),
        Subcommand::CaddyStart { folder } => rt.start_caddy(folder.as_deref(), &cancel),
        Subcommand::ServerTunnelStart { server_id } => rt.start_server_tunnel(&server_id, &cancel),
        _ => unreachable!(),
    };
    let started = match started {
        Ok(s) => s,
        Err(err) => {
            drop(pump);
            return Err(map_start_err(&watcher, err));
        }
    };
    eprintln!(
        "Started {} (session {})",
        started.function_id, started.session_id
    );
    let outcome = wait_foreground_loop(rt, &watcher, &started);
    drop(pump);
    rt.shutdown(SHUTDOWN_TIMEOUT);
    outcome
}

fn map_start_err(watcher: &SignalWatcher, err: CoreError) -> CoreError {
    if watcher.signal() == Some(TerminationSignal::Interrupt) {
        CoreError::interrupted("Operation interrupted by user")
    } else {
        err
    }
}

fn wait_foreground_loop(
    rt: &ProfileRuntime<OsKeyringBackend>,
    watcher: &SignalWatcher,
    started: &StartFunctionResult,
) -> Result<Value, CoreError> {
    loop {
        let exited = rt
            .supervisor()
            .wait_for_session_exit(&started.session_id, Duration::from_millis(40));
        if let Some(sig) = watcher.signal() {
            let _ = rt.stop_supervised_function(&started.function_id, SHUTDOWN_TIMEOUT);
            return match sig {
                TerminationSignal::Terminate => Ok(stopped_status_json(&started.function_id)),
                TerminationSignal::Interrupt => {
                    Err(CoreError::interrupted("Foreground session interrupted"))
                }
            };
        }
        if exited {
            return resolve_self_exited_status(rt, &started.function_id);
        }
    }
}

fn resolve_self_exited_status(
    rt: &ProfileRuntime<OsKeyringBackend>,
    function_id: &str,
) -> Result<Value, CoreError> {
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match rt.get_function_status(function_id) {
            Err(_) => return Ok(stopped_status_json(function_id)),
            Ok(st) if st.session_id.is_none() => {
                if st.state == "error" {
                    return Err(CoreError::operation_failed(
                        "process_exited",
                        "Foreground session exited unexpectedly",
                        true,
                        "inspect_status",
                    ));
                }
                return Ok(stopped_status_json(function_id));
            }
            Ok(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            Ok(_) => return Ok(stopped_status_json(function_id)),
        }
    }
}

pub fn run_boundary_auth(rt: &ProfileRuntime<OsKeyringBackend>) -> Result<Value, CoreError> {
    let watcher = install_signal_watcher()?;
    let cancel = CancelToken::new();
    let pump = CancelPump::spawn(&watcher, &cancel);
    let started = match rt.start_boundary_auth(&cancel) {
        Ok(s) => s,
        Err(err) => {
            drop(pump);
            return Err(map_start_err(&watcher, err));
        }
    };
    let outcome = wait_boundary_auth_loop(rt, &watcher, &started);
    drop(pump);
    outcome
}

fn wait_boundary_auth_loop(
    rt: &ProfileRuntime<OsKeyringBackend>,
    watcher: &SignalWatcher,
    started: &StartFunctionResult,
) -> Result<Value, CoreError> {
    let deadline = Instant::now() + BOUNDARY_AUTH_TIMEOUT;
    loop {
        let exited = rt
            .supervisor()
            .wait_for_session_exit(&started.session_id, Duration::from_millis(25));
        if let Some(sig) = watcher.signal() {
            let _ = rt.stop_supervised_function(BOUNDARY_AUTH_FUNCTION_ID, SHUTDOWN_TIMEOUT);
            return match sig {
                TerminationSignal::Terminate => Ok(stopped_status_json(BOUNDARY_AUTH_FUNCTION_ID)),
                TerminationSignal::Interrupt => Err(CoreError::interrupted(
                    "Boundary authentication interrupted",
                )),
            };
        }
        if exited {
            return finalize_boundary_auth_status(rt);
        }
        if Instant::now() >= deadline {
            let _ = rt.stop_supervised_function(BOUNDARY_AUTH_FUNCTION_ID, SHUTDOWN_TIMEOUT);
            return Err(CoreError::timeout(
                "boundary_auth_timeout",
                "Boundary authentication timed out",
            ));
        }
    }
}

fn finalize_boundary_auth_status(
    rt: &ProfileRuntime<OsKeyringBackend>,
) -> Result<Value, CoreError> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let st = rt.get_function_status(BOUNDARY_AUTH_FUNCTION_ID)?;
        if st.session_id.is_none() || Instant::now() >= deadline {
            if st.state == "authenticated" {
                return Ok(json!(SafeFunctionStatus::from(&st)));
            }
            return Err(CoreError::operation_failed(
                "boundary_auth_failed",
                "Boundary authentication failed",
                false,
                "check_credentials",
            ));
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

pub fn run_scp(
    rt: &ProfileRuntime<OsKeyringBackend>,
    cmd: &Subcommand,
) -> Result<Value, CoreError> {
    let watcher = install_signal_watcher()?;
    let cancel = CancelToken::new();
    let pump = CancelPump::spawn(&watcher, &cancel);
    let res = match cmd {
        Subcommand::ScpUpload {
            server_id,
            local,
            remote,
            recursive,
        } => rt.scp_upload(server_id, local, remote, *recursive, &cancel),
        Subcommand::ScpDownload {
            server_id,
            remote,
            local,
            recursive,
            overwrite,
        } => rt.scp_download(server_id, remote, local, *recursive, *overwrite, &cancel),
        _ => unreachable!(),
    };
    drop(pump);
    rt.shutdown(SHUTDOWN_TIMEOUT);
    match res {
        Ok(entry) => Ok(json!(SafeTransfer::from(&entry))),
        Err(err) => Err(map_start_err(&watcher, err)),
    }
}

pub fn execute_interactive_ssh(req: CommandRequest) -> i32 {
    let Subcommand::ServerSsh { server_id } = req.subcommand else {
        unreachable!()
    };
    if !is_interactive_tty() {
        let err = CoreError::capability_unavailable(
            "interactive_tty_required",
            "server ssh requires an interactive terminal",
            "run_in_terminal",
        );
        eprintln!("{err}");
        return err.exit_code();
    }
    execute_interactive_ssh_tty(req.config_dir, req.profile, &server_id)
}

fn execute_interactive_ssh_tty(
    config_dir: Option<PathBuf>,
    profile: Option<String>,
    server_id: &str,
) -> i32 {
    let loc = OwnedProfileLocation::from_env(config_dir, profile);
    let rt = match ProfileRuntime::open(&loc.as_input(), "cli") {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("{err}");
            return err.exit_code();
        }
    };
    let watcher = match install_signal_watcher() {
        Ok(w) => w,
        Err(err) => {
            eprintln!("{err}");
            return err.exit_code();
        }
    };
    let mut term_guard = TerminalModeGuard::capture();
    let cancel = CancelToken::new();
    let pump = CancelPump::spawn(&watcher, &cancel);
    let prepared = rt.prepare_interactive_ssh(server_id, &cancel);
    drop(pump);
    let exit_code = match prepared {
        Ok((program, args, _temp_srv)) => spawn_and_wait_ssh(&program, &args, &watcher),
        Err(err) => {
            let mapped = map_start_err(&watcher, err);
            eprintln!("{mapped}");
            mapped.exit_code()
        }
    };
    term_guard.restore();
    rt.shutdown(SHUTDOWN_TIMEOUT);
    exit_code
}

fn spawn_and_wait_ssh(program: &str, args: &[String], watcher: &SignalWatcher) -> i32 {
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            eprintln!("Failed to launch SSH client ({program}): {err}");
            return 1;
        }
    };
    poll_ssh_child(&mut child, watcher)
}

fn poll_ssh_child(child: &mut Child, watcher: &SignalWatcher) -> i32 {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if watcher.signal() == Some(TerminationSignal::Interrupt) {
                    130
                } else {
                    status.code().unwrap_or(1)
                };
            }
            Ok(None) => {
                if let Some(sig) = watcher.signal() {
                    terminate_child(child);
                    return match sig {
                        TerminationSignal::Interrupt => 130,
                        TerminationSignal::Terminate => 0,
                    };
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => {
                eprintln!("SSH wait failed: {err}");
                return 1;
            }
        }
    }
}
