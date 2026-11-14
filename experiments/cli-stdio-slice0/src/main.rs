use scp_cli_stdio_slice0::SpikeError;
use scp_cli_stdio_slice0::boundary_spike::{BoundaryTokenMode, run_get_token};
use scp_cli_stdio_slice0::keyring_spike::{NamespacedSecretStore, ensure_keyring_ready};
use scp_cli_stdio_slice0::profile_lock::{acquire_owner_lease, resolve_profile_dir};
use scp_cli_stdio_slice0::supervisor_spike::{ChildKind, OwnershipOutcome, Supervisor};
use scp_cli_stdio_slice0::vpn_spike::{VpnPreflightConfig, preflight_and_acquire_vpn};
use serde_json::json;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static DAEMON_SHUTDOWN: AtomicBool = AtomicBool::new(false);

fn fail(err: &SpikeError) -> ExitCode {
    eprintln!("{} ({}): {}", err.code, err.reason, err.message);
    ExitCode::from(err.exit_code)
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).cloned();
        }
        i += 1;
    }
    None
}

fn required_flag(args: &[String], name: &str) -> Result<String, SpikeError> {
    flag_value(args, name).ok_or_else(|| {
        SpikeError::new(
            "INVALID_ARGUMENTS",
            "missing_flag",
            format!("Missing required flag {name}"),
            2,
        )
    })
}

fn write_atomic_file(path: &Path, bytes: &[u8]) -> Result<(), SpikeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SpikeError::new("OPERATION_FAILED", "io_error", e.to_string(), 1))?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp, bytes)
        .map_err(|e| SpikeError::new("OPERATION_FAILED", "io_error", e.to_string(), 1))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        SpikeError::new("OPERATION_FAILED", "io_error", e.to_string(), 1)
    })
}

fn cmd_hold_lock(args: &[String]) -> Result<(), SpikeError> {
    let config_dir = required_flag(args, "--config-dir")?;
    let profile = required_flag(args, "--profile")?;
    let hold_ms: u64 = required_flag(args, "--hold-ms")?
        .parse()
        .map_err(|e| SpikeError::new("INVALID_ARGUMENTS", "invalid_hold_ms", format!("{e}"), 2))?;
    let profile_dir = resolve_profile_dir(
        Some(Path::new(&config_dir)),
        None,
        None,
        None,
        None,
        Some(&profile),
    )?;
    let _lease = acquire_owner_lease(&profile_dir, "cli-hold-lock", "hold-lock")?;
    if let Some(ready_path) = flag_value(args, "--ready-file") {
        write_atomic_file(Path::new(&ready_path), b"ready\n")?;
    }
    std::thread::sleep(Duration::from_millis(hold_ms));
    Ok(())
}

fn cmd_keyring_probe(args: &[String]) -> Result<(), SpikeError> {
    let namespace = required_flag(args, "--namespace")?;
    let action = required_flag(args, "--action")?;
    if action == "preflight" {
        return ensure_keyring_ready();
    }
    let account = flag_value(args, "--account").unwrap_or_else(|| "probe-account".to_string());
    let mut store = NamespacedSecretStore::new("com.chuonglv.scpctl", namespace);
    match action.as_str() {
        "get" => {
            let _ = store.get(&account)?;
            Ok(())
        }
        "set" => {
            let val = required_flag(args, "--value")?;
            store.set(&account, &val)
        }
        "delete" => store.delete(&account),
        other => Err(SpikeError::new(
            "INVALID_ARGUMENTS",
            "unknown_keyring_action",
            format!("Unsupported keyring action {other}"),
            2,
        )),
    }
}

#[cfg(unix)]
extern "C" fn handle_daemon_signal(_sig: libc::c_int) {
    DAEMON_SHUTDOWN.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
fn install_daemon_signals() {
    // SAFETY: Signal handler only writes an atomic bool, which is async-signal-safe.
    unsafe {
        libc::signal(
            libc::SIGTERM,
            handle_daemon_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            handle_daemon_signal as *const () as libc::sighandler_t,
        );
    }
}

fn cmd_supervisor_daemon(args: &[String]) -> Result<(), SpikeError> {
    let state_file = required_flag(args, "--state-file")?;
    DAEMON_SHUTDOWN.store(false, Ordering::SeqCst);
    #[cfg(unix)]
    install_daemon_signals();

    let sup = Supervisor::new();
    let OwnershipOutcome::Accepted(pty_id) =
        sup.spawn_and_register(ChildKind::Pty, "sleep", &["30"], false)?
    else {
        return Err(SpikeError::new(
            "OPERATION_FAILED",
            "spawn_cancelled",
            "unexpected cancel",
            1,
        ));
    };
    let OwnershipOutcome::Accepted(pipe_id) =
        sup.spawn_and_register(ChildKind::Pipe, "sleep", &["30"], false)?
    else {
        let _ = sup.shutdown(Duration::from_secs(1));
        return Err(SpikeError::new(
            "OPERATION_FAILED",
            "spawn_cancelled",
            "unexpected cancel",
            1,
        ));
    };

    let sessions = sup.active_sessions();
    let pty_pid = sessions
        .iter()
        .find(|(id, _)| id == &pty_id)
        .map(|(_, p)| *p)
        .unwrap_or(0);
    let pipe_pid = sessions
        .iter()
        .find(|(id, _)| id == &pipe_id)
        .map(|(_, p)| *p)
        .unwrap_or(0);

    let payload = json!({
        "ptyPid": pty_pid,
        "pipePid": pipe_pid,
    })
    .to_string();
    write_atomic_file(Path::new(&state_file), payload.as_bytes())?;

    std::thread::spawn(|| {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 64];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 {
                break;
            }
        }
        DAEMON_SHUTDOWN.store(true, Ordering::SeqCst);
    });

    while !DAEMON_SHUTDOWN.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = sup.shutdown(Duration::from_secs(2));
    Ok(())
}

fn cmd_vpn_lock(args: &[String]) -> Result<(), SpikeError> {
    let cfg = VpnPreflightConfig {
        sudo_path: PathBuf::from(required_flag(args, "--sudo-path")?),
        openvpn_path: PathBuf::from(required_flag(args, "--openvpn-path")?),
        ovpn_path: PathBuf::from(required_flag(args, "--ovpn-path")?),
        vpn_lock_path: PathBuf::from(required_flag(args, "--lock-file")?),
        terminate_when_helper_exits: false,
    };
    let hold_ms: u64 = required_flag(args, "--hold-ms")?
        .parse()
        .map_err(|e| SpikeError::new("INVALID_ARGUMENTS", "invalid_hold_ms", format!("{e}"), 2))?;
    let _lease = preflight_and_acquire_vpn(&cfg)?;
    if let Some(ready_path) = flag_value(args, "--ready-file") {
        write_atomic_file(Path::new(&ready_path), b"ready\n")?;
    }
    std::thread::sleep(Duration::from_millis(hold_ms));
    Ok(())
}

fn cmd_boundary_probe(args: &[String]) -> Result<(), SpikeError> {
    let boundary_path = required_flag(args, "--boundary-path")?;
    let namespace = required_flag(args, "--namespace")?;
    let _token = run_get_token(
        Path::new(&boundary_path),
        None,
        &BoundaryTokenMode::NamespacedKeyring { namespace },
    )?;
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let res = match args.first().map(String::as_str) {
        Some("serve-stdio") => return scp_cli_stdio_slice0::stdio_host::run_stdio_host(),
        Some("hold-lock") => cmd_hold_lock(&args[1..]),
        Some("keyring-probe") => cmd_keyring_probe(&args[1..]),
        Some("supervisor-daemon") => cmd_supervisor_daemon(&args[1..]),
        Some("vpn-lock") => cmd_vpn_lock(&args[1..]),
        Some("boundary-probe") => cmd_boundary_probe(&args[1..]),
        other => {
            eprintln!("Unknown subcommand: {:?}", other);
            return ExitCode::from(2);
        }
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&err),
    }
}
