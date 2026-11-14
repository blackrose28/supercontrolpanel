//! Cross-platform OpenVPN detection, TOTP generation, and CLI/Unix elevation supervision.
use crate::error::CoreError;
use crate::models::{
    OpenVpnConfig, StartFunctionResult, KEYRING_OPENVPN_OTP_SECRET, KEYRING_OPENVPN_PASSWORD,
    OPENVPN_FUNCTION_ID,
};
use crate::profile::{acquire_exclusive_lock_file, OwnerLease};
use crate::runtime::{ProfileRuntime, StatusUpdate, SuperviseHooks, SupervisedReadyWait};
use crate::secrets::KeyringBackend;
use crate::supervisor::{CancelToken, ChildLaunch, OutputCallback, ProcessSupervisor, SpawnMode};
use parking_lot::Mutex;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use totp_rs::{Algorithm, Secret, TOTP};
use uuid::Uuid;

const DEFAULT_OPENVPN_READY_TIMEOUT: Duration = Duration::from_secs(30);

/// Optional test/runtime overrides for OpenVPN discovery, `sudo` path, and `vpn.lock`.
#[derive(Clone, Default)]
pub struct OpenVpnRuntimeOverrides {
    pub sudo_program: Option<String>,
    pub vpn_lock_path: Option<PathBuf>,
    pub locator_program: Option<String>,
    pub candidate_paths: Option<Vec<String>>,
    pub ready_timeout: Option<Duration>,
}

/// Parses the stdout/exit status of `which` or `where` when probing for OpenVPN.
pub fn openvpn_locator_result(
    status: Option<i32>,
    stdout: &[u8],
) -> Result<Option<String>, String> {
    if status == Some(1) {
        return Ok(None);
    }
    if status != Some(0) {
        return Err("OpenVPN executable detection failed".into());
    }
    std::str::from_utf8(stdout)
        .ok()
        .and_then(|value| value.lines().map(str::trim).find(|line| !line.is_empty()))
        .map(|line| Some(line.to_owned()))
        .ok_or_else(|| "OpenVPN executable detection failed".into())
}

fn common_openvpn_path(candidates: &[&str]) -> Result<Option<String>, String> {
    for candidate in candidates {
        match fs::metadata(candidate) {
            Ok(metadata) if metadata.is_file() => return Ok(Some((*candidate).into())),
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => return Err("OpenVPN executable detection failed".into()),
        }
    }
    Ok(None)
}

/// Probes `locator` on PATH and falls back to `candidates` when `locator` finds nothing.
pub fn detect_openvpn_executable_checked_with(
    locator: &str,
    candidates: &[&str],
) -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    if let Some(path) = common_openvpn_path(candidates)? {
        return Ok(Some(path));
    }
    let located = match Command::new(locator).arg("openvpn").output() {
        Ok(output) => openvpn_locator_result(output.status.code(), &output.stdout),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err("OpenVPN executable detection failed".into()),
    };
    if let Ok(Some(path)) = located {
        return Ok(Some(path));
    }
    if let Some(path) = common_openvpn_path(candidates)? {
        return Ok(Some(path));
    }
    located
}

fn default_platform_locator_and_candidates() -> (&'static str, &'static [&'static str]) {
    #[cfg(target_os = "windows")]
    {
        (
            "where",
            &[
                r"C:\Program Files\OpenVPN\bin\openvpn.exe",
                r"C:\Program Files\OpenVPN Connect\bin\openvpn.exe",
                r"C:\Program Files (x86)\OpenVPN\bin\openvpn.exe",
            ],
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        (
            "which",
            &[
                "/usr/sbin/openvpn",
                "/usr/local/sbin/openvpn",
                "/opt/homebrew/sbin/openvpn",
            ],
        )
    }
}

/// Discovers the OpenVPN executable on the current host, distinguishing absence from failure.
pub fn detect_openvpn_executable_checked() -> Result<Option<String>, String> {
    let (locator, default_cands) = default_platform_locator_and_candidates();
    if let Ok(raw_cands) = std::env::var("SCPCTL_OPENVPN_CANDIDATES") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        let list: Vec<String> = raw_cands
            .split(sep)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        let refs: Vec<&str> = list.iter().map(String::as_str).collect();
        return detect_openvpn_executable_checked_with(locator, &refs);
    }
    detect_openvpn_executable_checked_with(locator, default_cands)
}

/// Best-effort `Option` wrapper used by desktop callers.
pub fn detect_openvpn_executable_path() -> Option<String> {
    detect_openvpn_executable_checked().ok().flatten()
}

/// Quotes an argument safely for POSIX shells.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn quote_posix_arg(raw: &str) -> String {
    if raw.is_empty() {
        "''".to_string()
    } else if raw
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_./:".contains(&b))
    {
        raw.to_string()
    } else {
        format!("'{}'", raw.replace('\'', "'\"'\"'"))
    }
}

/// Resolves legacy `command` fallback into `executable_path`.
pub fn normalized_openvpn_config(config: &OpenVpnConfig) -> OpenVpnConfig {
    let mut cfg = config.clone();
    if cfg.executable_path.trim().is_empty() && !cfg.command.trim().is_empty() {
        cfg.executable_path = cfg.command.trim().to_string();
    }
    cfg
}

/// Builds the desktop Unix `sudo openvpn --config ...` shell command line.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn build_openvpn_command_unix(config: &OpenVpnConfig) -> Result<String, String> {
    if config.executable_path.trim().is_empty() {
        return Err("openvpn executable path cannot be empty".to_string());
    }
    if config.ovpn_path.trim().is_empty() {
        return Err("ovpn file path cannot be empty".to_string());
    }
    let mut command = format!(
        "sudo {} --config {}",
        quote_posix_arg(config.executable_path.trim()),
        quote_posix_arg(config.ovpn_path.trim())
    );
    if !config.extra_args.trim().is_empty() {
        command.push(' ');
        command.push_str(config.extra_args.trim());
    }
    Ok(command)
}

/// Generates a 6-digit SHA-1 TOTP code at an explicit UNIX timestamp.
pub fn generate_totp_code_at_epoch(otp_secret: &str, epoch_secs: u64) -> Result<String, String> {
    let secret = otp_secret.trim();
    if secret.is_empty() {
        return Err("OTP secret is empty".to_string());
    }
    let secret_bytes = Secret::Encoded(secret.to_string())
        .to_bytes()
        .map_err(|e| format!("invalid OTP secret: {e}"))?;
    let totp = TOTP::new(Algorithm::SHA1, 6, 1, 30, secret_bytes)
        .map_err(|e| format!("failed to initialize OTP generator: {e}"))?;
    Ok(totp.generate(epoch_secs))
}

/// Generates a 6-digit SHA-1 TOTP code using current clock plus signed offset seconds.
pub fn generate_totp_code_with_offset(
    otp_secret: &str,
    offset_secs: i64,
) -> Result<String, String> {
    let local_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("system time error: {e}"))?
        .as_secs();
    let corrected = local_epoch.saturating_add_signed(offset_secs);
    generate_totp_code_at_epoch(otp_secret, corrected)
}

/// Prefixes the 6-digit TOTP code before `password` when `otp_secret` is non-empty.
pub fn generate_openvpn_password_with_offset(
    password: &str,
    otp_secret: &str,
    offset_secs: i64,
) -> Result<String, String> {
    let secret = otp_secret.trim();
    if secret.is_empty() {
        return Ok(password.to_string());
    }
    let otp = generate_totp_code_with_offset(secret, offset_secs)?;
    Ok(format!("{otp}{password}"))
}

/// Exponential reconnect backoff schedule preserved from desktop commands.
pub fn compute_openvpn_reconnect_delay_secs(
    failure_count: u32,
    base_seconds: u64,
    max_seconds: u64,
) -> u64 {
    if failure_count <= 1 {
        return 0;
    }
    let effective_base = base_seconds.max(1);
    let effective_max = max_seconds.max(effective_base);
    let exponent = failure_count.saturating_sub(1).min(62);
    let factor = (1_u128 << exponent).saturating_sub(1);
    let delay = (effective_base as u128).saturating_mul(factor);
    delay.min(effective_max as u128) as u64
}

pub(crate) fn detect_openvpn_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
) -> Result<Option<String>, CoreError> {
    let ov = rt.openvpn_overrides();
    let res = match (ov.locator_program, ov.candidate_paths) {
        (None, None) => detect_openvpn_executable_checked(),
        (loc, cands) => {
            let (def_loc, def_cands) = default_platform_locator_and_candidates();
            let locator = loc.as_deref().unwrap_or(def_loc);
            let owned_cands =
                cands.unwrap_or_else(|| def_cands.iter().map(|s| (*s).to_string()).collect());
            let refs: Vec<&str> = owned_cands.iter().map(String::as_str).collect();
            detect_openvpn_executable_checked_with(locator, &refs)
        }
    };
    res.map_err(|e| {
        CoreError::operation_failed(
            "openvpn_detect_failed",
            e,
            false,
            "check_openvpn_installation",
        )
    })
}

pub(crate) fn stop_openvpn_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    timeout: Duration,
) -> Result<bool, CoreError> {
    rt.stop_supervised_function(OPENVPN_FUNCTION_ID, timeout)
}

#[cfg(target_os = "windows")]
pub(crate) fn start_openvpn_impl<B: KeyringBackend + 'static>(
    rt: &ProfileRuntime<B>,
    cancel: &CancelToken,
) -> Result<StartFunctionResult, CoreError> {
    start_openvpn_with_overrides_impl(rt, None, None, cancel)
}

#[cfg(target_os = "windows")]
pub(crate) fn start_openvpn_with_overrides_impl<B: KeyringBackend + 'static>(
    _rt: &ProfileRuntime<B>,
    _username: Option<&str>,
    _password: Option<&str>,
    _cancel: &CancelToken,
) -> Result<StartFunctionResult, CoreError> {
    Err(CoreError::capability_unavailable(
        "windows_cli_elevation_unimplemented",
        "Headless CLI OpenVPN elevation is not yet implemented on Windows",
        "use_desktop_app",
    ))
}

#[cfg(not(target_os = "windows"))]
struct ValidatedVpnInputs {
    cfg: OpenVpnConfig,
    password: String,
    otp_secret: String,
    cols: u16,
    rows: u16,
}

#[cfg(not(target_os = "windows"))]
struct VpnLeaseGuard {
    attempt_id: String,
    slot: Arc<Mutex<Option<(String, OwnerLease)>>>,
    armed: bool,
}

#[cfg(not(target_os = "windows"))]
impl Drop for VpnLeaseGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut guard = self.slot.lock();
        if guard.as_ref().is_some_and(|(id, _)| id == &self.attempt_id) {
            *guard = None;
        }
    }
}

#[cfg(not(target_os = "windows"))]
#[derive(Default)]
struct OpenVpnScanState {
    buf: String,
    username_sent: bool,
    password_sent: bool,
    connected: bool,
    auth_failed: bool,
    otp_error: Option<String>,
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn start_openvpn_impl<B: KeyringBackend + 'static>(
    rt: &ProfileRuntime<B>,
    cancel: &CancelToken,
) -> Result<StartFunctionResult, CoreError> {
    start_openvpn_with_overrides_impl(rt, None, None, cancel)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn start_openvpn_with_overrides_impl<B: KeyringBackend + 'static>(
    rt: &ProfileRuntime<B>,
    username_override: Option<&str>,
    password_override: Option<&str>,
    cancel: &CancelToken,
) -> Result<StartFunctionResult, CoreError> {
    let inputs = validate_vpn_inputs(rt, username_override, password_override)?;
    if rt.is_shutting_down() {
        return Err(CoreError::conflict(
            "shutting_down",
            "Runtime is shutting down",
            false,
            "restart_runtime",
        ));
    }
    if rt
        .supervisor()
        .active_session_for_function(OPENVPN_FUNCTION_ID)
        .is_some()
    {
        return Err(CoreError::conflict(
            "function_already_running",
            format!("Function '{OPENVPN_FUNCTION_ID}' is already running"),
            false,
            "stop_existing_session",
        ));
    }
    let ov = rt.openvpn_overrides();
    let mut lease_guard = acquire_vpn_lease(rt, &ov)?;
    let sudo_prog = resolve_sudo_program(&ov);
    run_sudo_preflight(&sudo_prog)?;
    let scan = Arc::new(Mutex::new(OpenVpnScanState::default()));
    let launch = build_openvpn_launch(&sudo_prog, &inputs);
    let hooks = build_openvpn_hooks(rt, &inputs, Arc::clone(&scan), &lease_guard.attempt_id);
    let sid =
        rt.start_supervised_with_hooks(launch, cancel, None, "Connecting OpenVPN...", hooks)?;
    wait_for_openvpn_ready(rt, &sid, &scan, &ov, cancel)?;
    lease_guard.armed = false;
    Ok(StartFunctionResult {
        function_id: OPENVPN_FUNCTION_ID.to_string(),
        session_id: sid,
    })
}

#[cfg(not(target_os = "windows"))]
fn resolve_vpn_secrets<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    cfg: &OpenVpnConfig,
    password_override: Option<&str>,
) -> Result<(String, String), CoreError> {
    let password = password_override
        .map(ToOwned::to_owned)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            rt.read_optional_secret(KEYRING_OPENVPN_PASSWORD)
                .filter(|s| !s.is_empty())
        })
        .or_else(|| (!cfg.password.is_empty()).then(|| cfg.password.clone()))
        .ok_or_else(|| {
            CoreError::capability_unavailable(
                "credentials_missing",
                "OpenVPN password is not configured in profile keyring",
                "configure_credentials",
            )
        })?;
    let otp_secret = rt
        .read_optional_secret(KEYRING_OPENVPN_OTP_SECRET)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| cfg.otp_secret.trim().to_string());
    if !otp_secret.is_empty() {
        generate_totp_code_with_offset(&otp_secret, 0).map_err(|e| {
            CoreError::invalid_arguments("invalid_otp_secret", format!("Invalid OTP secret: {e}"))
        })?;
    }
    Ok((password, otp_secret))
}

#[cfg(not(target_os = "windows"))]
fn validate_vpn_inputs<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    username_override: Option<&str>,
    password_override: Option<&str>,
) -> Result<ValidatedVpnInputs, CoreError> {
    let app = rt.get_app_config()?;
    let mut cfg = normalized_openvpn_config(&app.openvpn);
    if let Some(user) = username_override.map(str::trim).filter(|s| !s.is_empty()) {
        cfg.username = user.to_string();
    }
    if cfg.terminate_open_vpn_when_helper_exits {
        return Err(CoreError::capability_unavailable(
            "incompatible_vpn_ownership",
            "terminateOpenVpnWhenHelperExits=true is incompatible with standalone foreground OpenVPN",
            "disable_terminate_on_helper_exit",
        ));
    }
    if cfg.executable_path.trim().is_empty() || cfg.ovpn_path.trim().is_empty() {
        return Err(CoreError::invalid_arguments(
            "openvpn_config_incomplete",
            "OpenVPN executablePath and ovpnPath are required",
        ));
    }
    if !Path::new(cfg.ovpn_path.trim()).is_file() {
        return Err(CoreError::invalid_arguments(
            "ovpn_file_missing",
            format!("OpenVPN profile file not found: {}", cfg.ovpn_path.trim()),
        ));
    }
    if cfg.username.trim().is_empty() {
        return Err(CoreError::invalid_arguments(
            "openvpn_username_required",
            "OpenVPN username is required",
        ));
    }
    let (password, otp_secret) = resolve_vpn_secrets(rt, &cfg, password_override)?;
    Ok(ValidatedVpnInputs {
        cfg,
        password,
        otp_secret,
        cols: app.app.default_terminal_cols.max(80),
        rows: app.app.default_terminal_rows.max(24),
    })
}

#[cfg(not(target_os = "windows"))]
fn resolve_sudo_program(ov: &OpenVpnRuntimeOverrides) -> String {
    ov.sudo_program
        .clone()
        .or_else(|| {
            std::env::var("SCPCTL_SUDO_PATH")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "sudo".to_string())
}

#[cfg(not(target_os = "windows"))]
fn resolve_vpn_lock_path<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    ov: &OpenVpnRuntimeOverrides,
) -> PathBuf {
    ov.vpn_lock_path
        .clone()
        .or_else(|| {
            std::env::var_os("SCPCTL_VPN_LOCK_PATH")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| rt.profile_paths().root.join("vpn.lock"))
}

#[cfg(not(target_os = "windows"))]
fn acquire_vpn_lease<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    ov: &OpenVpnRuntimeOverrides,
) -> Result<VpnLeaseGuard, CoreError> {
    let lock_path = resolve_vpn_lock_path(rt, ov);
    let slot = rt.vpn_lease_arc();
    if slot.lock().is_some() {
        return Err(CoreError::conflict(
            "vpn_owner_conflict",
            "OpenVPN lease already active in this runtime",
            false,
            "stop_existing_session",
        ));
    }
    let lease =
        acquire_exclusive_lock_file(&lock_path, rt.runtime_id(), "openvpn", "vpn_owner_conflict")?;
    let attempt_id = Uuid::new_v4().to_string();
    *slot.lock() = Some((attempt_id.clone(), lease));
    Ok(VpnLeaseGuard {
        attempt_id,
        slot: Arc::clone(slot),
        armed: true,
    })
}

#[cfg(not(target_os = "windows"))]
fn run_sudo_preflight(sudo_prog: &str) -> Result<(), CoreError> {
    let status = Command::new(sudo_prog)
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                CoreError::capability_unavailable(
                    "elevation_unavailable",
                    format!("Elevation helper not found: {sudo_prog}"),
                    "install_or_configure_sudo",
                )
            } else {
                CoreError::capability_unavailable(
                    "elevation_unavailable",
                    format!("Elevation check failed: {err}"),
                    "check_sudo_permissions",
                )
            }
        })?;
    if !status.success() {
        return Err(CoreError::capability_unavailable(
            "elevation_interaction_required",
            "Non-interactive sudo -n preflight failed; configure passwordless sudo or use desktop app",
            "configure_passwordless_sudo",
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn build_openvpn_launch(sudo_prog: &str, inputs: &ValidatedVpnInputs) -> ChildLaunch {
    let mut args = vec![
        "-n".to_string(),
        inputs.cfg.executable_path.trim().to_string(),
        "--config".to_string(),
        inputs.cfg.ovpn_path.trim().to_string(),
    ];
    for extra in inputs.cfg.extra_args.split_whitespace() {
        args.push(extra.to_string());
    }
    ChildLaunch {
        function_id: OPENVPN_FUNCTION_ID.to_string(),
        program: sudo_prog.to_string(),
        args,
        cwd: None,
        env: Vec::new(),
        env_remove: Vec::new(),
        mode: SpawnMode::Pty {
            cols: inputs.cols,
            rows: inputs.rows,
        },
        kill_tree: true,
        kind: "openvpn",
    }
}

#[cfg(not(target_os = "windows"))]
fn build_openvpn_hooks<B: KeyringBackend + 'static>(
    rt: &ProfileRuntime<B>,
    inputs: &ValidatedVpnInputs,
    scan: Arc<Mutex<OpenVpnScanState>>,
    attempt_id: &str,
) -> SuperviseHooks {
    let sup = Arc::clone(rt.supervisor());
    let st_arc = Arc::clone(rt.runtime_state_arc());
    let sink = Arc::clone(rt.event_sink());
    let username = inputs.cfg.username.trim().to_string();
    let password = inputs.password.clone();
    let otp = inputs.otp_secret.clone();

    let on_output = Some(Arc::new(move |sid: &str, chunk: &str, _is_pty: bool| {
        let (send_user, send_pw, connected, auth_fail) = {
            let mut st = scan.lock();
            inspect_openvpn_chunk(&mut st, chunk)
        };
        if send_user {
            spawn_pty_line(&sup, sid, &username);
            rt_status(&st_arc, &sink, sid, "authenticating", "Username sent");
        }
        if let Some(()) = send_pw {
            match generate_openvpn_password_with_offset(&password, &otp, 0) {
                Ok(combined) => {
                    spawn_pty_line(&sup, sid, &combined);
                    rt_status(&st_arc, &sink, sid, "authenticating", "Password sent");
                }
                Err(e) => scan.lock().otp_error = Some(e),
            }
        }
        if auth_fail {
            rt_status(&st_arc, &sink, sid, "error", "Authentication failed");
        } else if connected {
            rt_status(&st_arc, &sink, sid, "running", "VPN tunnel is up");
        }
    }) as OutputCallback);

    let lease_slot = Arc::clone(rt.vpn_lease_arc());
    let aid = attempt_id.to_string();
    let on_exit = Some(
        Arc::new(move |_sid: &str, _code: Option<i32>, _reason: &str| {
            let mut guard = lease_slot.lock();
            if guard.as_ref().is_some_and(|(id, _)| id == &aid) {
                *guard = None;
            }
        }) as crate::supervisor::ExitCallback,
    );

    SuperviseHooks { on_output, on_exit }
}

#[cfg(not(target_os = "windows"))]
fn inspect_openvpn_chunk(st: &mut OpenVpnScanState, chunk: &str) -> (bool, Option<()>, bool, bool) {
    st.buf.push_str(&chunk.to_lowercase());
    if st.buf.len() > 5000 {
        let drain_to = st.buf.len() - 3000;
        st.buf.drain(..drain_to);
    }
    let want_user = !st.username_sent
        && (st.buf.contains("username")
            || st.buf.contains("user name")
            || st.buf.contains("enter auth username"));
    if want_user {
        st.username_sent = true;
    }
    let want_pw = (!st.password_sent && st.buf.contains("password")).then(|| {
        st.password_sent = true;
    });
    let auth_fail = st.buf.contains("authentication failed")
        || st.buf.contains("auth failed")
        || st.buf.contains("auth-failure")
        || st.buf.contains("auth_failed");
    if auth_fail {
        st.auth_failed = true;
    }
    let connected = !st.connected
        && (st.buf.contains("initialization sequence completed")
            || st.buf.contains("peer connection initiated"));
    if connected {
        st.connected = true;
    }
    (want_user, want_pw, connected, auth_fail)
}

#[cfg(not(target_os = "windows"))]
fn spawn_pty_line(sup: &Arc<ProcessSupervisor>, sid: &str, text: &str) {
    let sup_w = Arc::clone(sup);
    let sid_w = sid.to_string();
    let line = format!("{text}\r");
    std::thread::spawn(move || {
        let _ = sup_w.write(&sid_w, &line);
    });
}

#[cfg(not(target_os = "windows"))]
fn rt_status(
    state: &Arc<Mutex<crate::runtime::RuntimeState>>,
    sink: &Arc<dyn crate::events::EventSink>,
    sid: &str,
    status_state: &str,
    msg: &str,
) {
    crate::runtime::update_active_session_status(
        state,
        sink,
        StatusUpdate {
            function_id: OPENVPN_FUNCTION_ID,
            state: status_state,
            message: msg.to_string(),
            boundary_token_source: None,
            session_id: Some(sid.to_string()),
        },
        sid,
    );
}

#[cfg(not(target_os = "windows"))]
fn check_scan_outcome(scan: &Arc<Mutex<OpenVpnScanState>>) -> Result<bool, CoreError> {
    let st = scan.lock();
    if let Some(otp_err) = st.otp_error.clone() {
        return Err(CoreError::invalid_arguments("invalid_otp_secret", otp_err));
    }
    if st.auth_failed {
        return Err(CoreError::operation_failed(
            "openvpn_auth_failed",
            "OpenVPN authentication failed",
            false,
            "check_openvpn_credentials",
        ));
    }
    Ok(st.connected)
}

#[cfg(not(target_os = "windows"))]
fn wait_for_openvpn_ready<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    session_id: &str,
    scan: &Arc<Mutex<OpenVpnScanState>>,
    ov: &OpenVpnRuntimeOverrides,
    cancel: &CancelToken,
) -> Result<(), CoreError> {
    let timeout = ov.ready_timeout.unwrap_or(DEFAULT_OPENVPN_READY_TIMEOUT);
    rt.wait_for_supervised_ready(
        SupervisedReadyWait {
            function_id: OPENVPN_FUNCTION_ID,
            session_id,
            timeout,
            cancel_msg: "OpenVPN start cancelled",
            early_exit: (
                "openvpn_exited_early",
                "OpenVPN exited before completing initialization",
                "inspect_openvpn_config",
            ),
            timeout_err: (
                "openvpn_ready_timeout",
                format!(
                    "Timed out after {}s waiting for OpenVPN tunnel readiness",
                    timeout.as_secs()
                ),
            ),
        },
        cancel,
        || check_scan_outcome(scan),
    )
}
