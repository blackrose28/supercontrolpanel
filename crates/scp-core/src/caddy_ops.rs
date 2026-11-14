//! Headless Caddy static/reverse-proxy supervision and readiness checks.
use crate::error::CoreError;
use crate::models::{StartFunctionResult, CADDY_FUNCTION_ID};
use crate::runtime::{ProfileRuntime, SuperviseHooks};
use crate::secrets::KeyringBackend;
use crate::supervisor::{CancelToken, ChildLaunch, OutputCallback, SpawnMode};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CADDY_READY_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn start_caddy_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    folder_override: Option<&str>,
    cancel: &CancelToken,
) -> Result<StartFunctionResult, CoreError> {
    let folder = resolve_caddy_folder(rt, folder_override)?;
    let exe = resolve_caddy_executable(rt)?;
    let cfg = rt.get_app_config()?;
    let launch = ChildLaunch {
        function_id: CADDY_FUNCTION_ID.to_string(),
        program: exe,
        args: vec![
            "run".to_string(),
            "--config".to_string(),
            "Caddyfile".to_string(),
        ],
        cwd: Some(folder.clone()),
        env: Vec::new(),
        env_remove: Vec::new(),
        mode: SpawnMode::Pty {
            cols: cfg.app.default_terminal_cols.max(80),
            rows: cfg.app.default_terminal_rows.max(24),
        },
        kill_tree: true,
        kind: "caddy",
    };

    let ready = Arc::new(AtomicBool::new(false));
    let ready_out = Arc::clone(&ready);
    let hooks = SuperviseHooks {
        on_output: Some(Arc::new(move |_sid: &str, chunk: &str, _is_pty: bool| {
            let lower = chunk.to_lowercase();
            if lower.contains("serving initial configuration") || lower.contains("caddy serving") {
                ready_out.store(true, Ordering::SeqCst);
            }
        }) as OutputCallback),
        on_exit: None,
    };

    let initial_msg = format!("Caddy running in {}", folder.display());
    let sid = rt.start_supervised_with_hooks(launch, cancel, None, &initial_msg, hooks)?;
    wait_for_caddy_ready(rt, &sid, &ready, cancel)?;
    Ok(StartFunctionResult {
        function_id: CADDY_FUNCTION_ID.to_string(),
        session_id: sid,
    })
}

pub(crate) fn stop_caddy_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    timeout: Duration,
) -> Result<bool, CoreError> {
    rt.stop_supervised_function(CADDY_FUNCTION_ID, timeout)
}

fn resolve_caddy_folder<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    folder_override: Option<&str>,
) -> Result<PathBuf, CoreError> {
    let raw = match folder_override {
        Some(explicit) => explicit.trim().to_string(),
        None => rt.get_sync_settings()?.caddy_folder.trim().to_string(),
    };
    if raw.is_empty() {
        return Err(CoreError::invalid_arguments(
            "caddy_folder_empty",
            "Caddy folder cannot be empty",
        ));
    }
    let path = PathBuf::from(&raw);
    if !path.is_dir() {
        return Err(CoreError::invalid_arguments(
            "caddy_folder_missing",
            format!("Caddy folder does not exist: {}", path.display()),
        ));
    }
    let caddyfile = path.join("Caddyfile");
    if !caddyfile.is_file() {
        return Err(CoreError::invalid_arguments(
            "caddyfile_missing",
            format!("Caddyfile not found in folder: {}", path.display()),
        ));
    }
    Ok(path)
}

fn resolve_caddy_executable<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
) -> Result<String, CoreError> {
    let candidate = rt
        .caddy_executable_override()
        .or_else(|| {
            std::env::var("SCPCTL_CADDY_PATH")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "caddy".to_string());
    let is_path_like = candidate.contains('/') || candidate.contains('\\');
    if is_path_like {
        if Path::new(&candidate).is_file() {
            return Ok(candidate);
        }
        return Err(caddy_missing_err(&candidate));
    }
    if let Some(found) = find_on_path(&candidate) {
        return Ok(found);
    }
    Err(caddy_missing_err(&candidate))
}

fn find_on_path(bin: &str) -> Option<String> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let cand = dir.join(bin);
        if cand.is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
        #[cfg(target_os = "windows")]
        {
            let exe = dir.join(format!("{bin}.exe"));
            if exe.is_file() {
                return Some(exe.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn caddy_missing_err(candidate: &str) -> CoreError {
    CoreError::capability_unavailable(
        "executable_not_found",
        format!("Caddy executable not found: {candidate}"),
        "install_or_configure_caddy",
    )
}

fn wait_for_caddy_ready<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    session_id: &str,
    ready: &AtomicBool,
    cancel: &CancelToken,
) -> Result<(), CoreError> {
    let deadline = Instant::now() + CADDY_READY_TIMEOUT;
    loop {
        if cancel.is_cancelled() {
            let _ = rt.stop_supervised_function(CADDY_FUNCTION_ID, Duration::from_secs(2));
            return Err(CoreError::interrupted("Caddy start cancelled"));
        }
        if ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        if rt
            .supervisor()
            .active_session_for_function(CADDY_FUNCTION_ID)
            .as_deref()
            != Some(session_id)
        {
            return Err(CoreError::operation_failed(
                "caddy_exited_early",
                "Caddy exited before serving configuration",
                false,
                "inspect_caddyfile",
            ));
        }
        if Instant::now() >= deadline {
            let _ = rt.stop_supervised_function(CADDY_FUNCTION_ID, Duration::from_secs(2));
            return Err(CoreError::timeout(
                "caddy_ready_timeout",
                "Timed out waiting for Caddy readiness",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
