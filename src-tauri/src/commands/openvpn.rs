use std::{fs, thread, time::Duration};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, State};

use super::{
    adapt_command_for_shell, load_app_config_from_disk, open_pty_internal,
    update_functions_for_session_exit, upsert_function_status,
};
#[cfg(target_os = "windows")]
use crate::models::{HelperStatusFile, OpenVpnLaunchConfig};
use crate::models::{PtyExitPayload, StartFunctionResult};
#[cfg(target_os = "windows")]
use crate::services::build_openvpn_args_windows;
use crate::services::prepare_openvpn_start;
#[cfg(not(target_os = "windows"))]
use crate::services::run_openvpn_automation as run_openvpn_automation_service;
#[cfg(not(target_os = "windows"))]
use crate::services::start_openvpn_pty_session as start_openvpn_pty_session_service;
#[cfg(not(target_os = "windows"))]
use crate::services::stop_openvpn_pty_session as stop_openvpn_pty_session_service;
use crate::services::{
    add_watcher, close_pty_internal, emit_pty_exit, lock_err, now_ms, write_pty_internal,
};
use crate::services::{
    build_openvpn_command_unix, generate_openvpn_password, normalized_openvpn_config,
};
#[cfg(target_os = "windows")]
use crate::services::{
    emit_openvpn_log as emit_openvpn_log_service, escape_ps_single_quoted,
    get_openvpn_admin_session, openvpn_runtime_dir, set_openvpn_admin_session,
    start_openvpn_admin_worker as start_openvpn_admin_worker_service,
    write_openvpn_helper_script as write_openvpn_helper_script_service,
};
#[cfg(target_os = "windows")]
use crate::stores::OpenVpnAdminSession;
use crate::stores::{FunctionStore, OpenVpnAdminStore, OpenVpnReconnectStore, SessionStore};
#[cfg(target_os = "windows")]
use crate::CREATE_NO_WINDOW;
use crate::OPENVPN_FUNCTION_ID;
#[cfg(target_os = "windows")]
use crate::OPENVPN_HELPER_SCRIPT_NAME;

#[cfg(target_os = "windows")]
fn is_windows_pid_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        let _ = CloseHandle(h);
        if ok == 0 {
            return false;
        }
        code == STILL_ACTIVE
    }
}

pub(crate) fn compute_openvpn_reconnect_delay_secs(
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

pub(crate) fn mark_openvpn_connected(app: &AppHandle) {
    if let Ok(mut state) = app.state::<OpenVpnReconnectStore>().state.lock() {
        state.consecutive_failures = 0;
        state.next_retry_at_ms = None;
    }
}

pub(crate) fn reset_openvpn_reconnect(app: &AppHandle, user_requested_stop: bool) {
    if let Ok(mut state) = app.state::<OpenVpnReconnectStore>().state.lock() {
        state.consecutive_failures = 0;
        state.next_retry_at_ms = None;
        state.retry_generation = state.retry_generation.saturating_add(1);
        state.user_requested_stop = user_requested_stop;
    }
}

pub(crate) fn prepare_openvpn_manual_start(app: &AppHandle) {
    reset_openvpn_reconnect(app, false);
}

pub(crate) fn handle_openvpn_terminal_state(app: &AppHandle, state: &str, reason: &str) {
    let reconnect_store = app.state::<OpenVpnReconnectStore>();
    let cfg = match load_app_config_from_disk(app) {
        Ok(cfg) => cfg,
        Err(_) => return,
    };
    let now = now_ms();
    let (generation, delay_secs) = {
        let mut reconnect = match reconnect_store.state.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if reconnect.user_requested_stop || !cfg.openvpn.auto_reconnect_enabled {
            reconnect.next_retry_at_ms = None;
            return;
        }
        reconnect.consecutive_failures = reconnect.consecutive_failures.saturating_add(1);
        // Safety cap: stop retrying after too many consecutive failures to avoid
        // unbounded retry chains when config is permanently broken.
        const MAX_RECONNECT_RETRIES: u32 = 50;
        if reconnect.consecutive_failures > MAX_RECONNECT_RETRIES {
            reconnect.next_retry_at_ms = None;
            drop(reconnect);
            let _ = upsert_function_status(
                app,
                &app.state::<FunctionStore>(),
                OPENVPN_FUNCTION_ID,
                "error",
                &format!(
                    "Auto-reconnect gave up after {MAX_RECONNECT_RETRIES} consecutive failures. {reason}"
                ),
                None,
            );
            return;
        }
        let delay_secs = compute_openvpn_reconnect_delay_secs(
            reconnect.consecutive_failures,
            cfg.openvpn.auto_reconnect_base_seconds,
            cfg.openvpn.auto_reconnect_max_seconds,
        );
        reconnect.next_retry_at_ms = Some(now.saturating_add(delay_secs.saturating_mul(1000)));
        (reconnect.retry_generation, delay_secs)
    };

    let message = if delay_secs == 0 {
        format!("OpenVPN exited unexpectedly ({state}); reconnecting now. {reason}")
    } else {
        format!("OpenVPN exited unexpectedly ({state}); retrying in {delay_secs}s. {reason}")
    };
    let _ = upsert_function_status(
        app,
        &app.state::<FunctionStore>(),
        OPENVPN_FUNCTION_ID,
        "reconnecting",
        &message,
        None,
    );

    let app_clone = app.clone();
    thread::spawn(move || {
        if delay_secs > 0 {
            thread::sleep(Duration::from_secs(delay_secs));
        }
        {
            let reconnect_store = app_clone.state::<OpenVpnReconnectStore>();
            let mut reconnect = match reconnect_store.state.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            if reconnect.retry_generation != generation || reconnect.user_requested_stop {
                return;
            }
            reconnect.next_retry_at_ms = None;
        }
        // Empty command/username/password: start_openvpn_action loads stored
        // config from disk when these are empty, so the reconnect uses the
        // same credentials as the original manual start.
        if let Err(err) = start_openvpn_action(
            String::new(),
            String::new(),
            String::new(),
            app_clone.clone(),
            app_clone.state::<SessionStore>(),
            app_clone.state::<FunctionStore>(),
            app_clone.state::<OpenVpnAdminStore>(),
        ) {
            handle_openvpn_terminal_state(
                &app_clone,
                "error",
                &format!("Reconnect start failed: {err}"),
            );
        }
    });
}

#[cfg(target_os = "windows")]
fn start_openvpn_admin_worker(
    app: &AppHandle,
    function_store: &FunctionStore,
    admin_store: &OpenVpnAdminStore,
    executable_path: String,
    args: String,
    username: String,
    password: String,
    terminate_open_vpn_when_helper_exits: bool,
    auto_reconnect_enabled: bool,
    auto_reconnect_base_seconds: u64,
    auto_reconnect_max_seconds: u64,
) -> Result<String, String> {
    start_openvpn_admin_worker_service(
        app,
        function_store,
        admin_store,
        executable_path,
        args,
        username,
        password,
        terminate_open_vpn_when_helper_exits,
        auto_reconnect_enabled,
        auto_reconnect_base_seconds,
        auto_reconnect_max_seconds,
        OPENVPN_HELPER_SCRIPT_NAME,
        OPENVPN_FUNCTION_ID,
        now_ms,
        is_windows_pid_running,
        mark_openvpn_connected,
        upsert_function_status,
        handle_openvpn_terminal_state,
    )
}

#[cfg(not(target_os = "windows"))]
fn run_openvpn_automation(
    app: AppHandle,
    session_id: String,
    command: String,
    username: String,
    password: String,
    otp_secret: String,
    close_pty: fn(&str, &SessionStore) -> Result<Option<PtyExitPayload>, String>,
    emit_exit: fn(&AppHandle, PtyExitPayload),
    on_session_exit: fn(&AppHandle, &FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
) {
    run_openvpn_automation_service(
        app,
        session_id,
        command,
        username,
        password,
        otp_secret,
        OPENVPN_FUNCTION_ID,
        add_watcher,
        write_pty_internal,
        upsert_function_status,
        generate_openvpn_password,
        mark_openvpn_connected,
        adapt_command_for_shell,
        lock_err,
        close_pty,
        emit_exit,
        on_session_exit,
    )
}

pub(crate) fn start_openvpn_action(
    _command: String,
    username: String,
    password: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    _admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<StartFunctionResult, String> {
    scp_debug!("start_openvpn_function: invoked");
    let saved_config = load_app_config_from_disk(&app)?;
    let (resolved_openvpn, resolved_username, combined_password) = match prepare_openvpn_start(
        &saved_config,
        username,
        password,
        normalized_openvpn_config,
        generate_openvpn_password,
    ) {
        Ok(values) => values,
        Err(e) => {
            scp_debug!("start_openvpn_function: ERR prepare_openvpn_start: {e}");
            return Err(e);
        }
    };

    scp_debug!(
        "start_openvpn_function: after load — exe_nonempty={} ovpn_nonempty={} user_nonempty={} password_nonempty={}",
        !resolved_openvpn.executable_path.trim().is_empty(),
        !resolved_openvpn.ovpn_path.trim().is_empty(),
        !resolved_username.trim().is_empty(),
        !combined_password.is_empty()
    );

    #[cfg(target_os = "windows")]
    {
        let _ = &session_store;
        let openvpn_args = match build_openvpn_args_windows(&resolved_openvpn) {
            Ok(a) => a,
            Err(e) => {
                scp_debug!("start_openvpn_function: ERR build_openvpn_args_windows: {e}");
                return Err(e);
            }
        };
        scp_debug!(
            "start_openvpn_function: Windows path — launching elevated helper; exe={}",
            resolved_openvpn.executable_path
        );
        if let Some(active) = get_openvpn_admin_session(&_admin_store)? {
            let _ = fs::write(&active.stop_path, "stop");
        }
        let session_id = start_openvpn_admin_worker(
            &app,
            &function_store,
            &_admin_store,
            resolved_openvpn.executable_path.clone(),
            openvpn_args,
            resolved_username,
            combined_password,
            resolved_openvpn.terminate_open_vpn_when_helper_exits,
            saved_config.openvpn.auto_reconnect_enabled,
            saved_config.openvpn.auto_reconnect_base_seconds,
            saved_config.openvpn.auto_reconnect_max_seconds,
        )?;
        scp_debug!("start_openvpn_function: Windows helper session_id={session_id}");

        return Ok(StartFunctionResult {
            function_id: OPENVPN_FUNCTION_ID.to_string(),
            session_id,
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let session_id = start_openvpn_pty_session_service(
            &app,
            &session_store,
            &function_store,
            &saved_config,
            resolved_openvpn,
            resolved_username,
            combined_password,
            OPENVPN_FUNCTION_ID,
            lock_err,
            close_pty_internal,
            upsert_function_status,
            open_pty_internal,
            build_openvpn_command_unix,
            run_openvpn_automation,
            emit_pty_exit,
            update_functions_for_session_exit,
        )?;

        return Ok(StartFunctionResult {
            function_id: OPENVPN_FUNCTION_ID.to_string(),
            session_id,
        });
    }
}

pub(crate) fn start_openvpn_manual_action(
    command: String,
    username: String,
    password: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<StartFunctionResult, String> {
    prepare_openvpn_manual_start(&app);
    start_openvpn_action(
        command,
        username,
        password,
        app,
        session_store,
        function_store,
        admin_store,
    )
}

#[tauri::command]
pub fn start_openvpn_function(
    command: String,
    username: String,
    password: String,
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<StartFunctionResult, String> {
    start_openvpn_manual_action(
        command,
        username,
        password,
        app,
        session_store,
        function_store,
        admin_store,
    )
}

pub(crate) fn stop_openvpn_action(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    _admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<(), String> {
    reset_openvpn_reconnect(&app, true);
    #[cfg(target_os = "windows")]
    {
        let _ = &session_store;
        let active = get_openvpn_admin_session(&_admin_store)?;
        if let Some(active) = active {
            let helper_pid_path =
                openvpn_runtime_dir(&app)?.join(format!("{}.helper.pid", active.id));
            if let Ok(s) = fs::read_to_string(&helper_pid_path) {
                if let Ok(pid) = s.trim().parse::<u32>() {
                    if pid > 0 && !is_windows_pid_running(pid) {
                        let _ = fs::remove_file(&helper_pid_path);
                        upsert_function_status(
                            &app,
                            &function_store,
                            OPENVPN_FUNCTION_ID,
                            "stopped",
                            "Helper already exited",
                            None,
                        )?;
                        set_openvpn_admin_session(&_admin_store, None)?;
                        return Ok(());
                    }
                }
            }
            upsert_function_status(
                &app,
                &function_store,
                OPENVPN_FUNCTION_ID,
                "stopping",
                "Requesting elevated OpenVPN stop",
                Some(active.id.clone()),
            )?;
            fs::write(&active.stop_path, "stop")
                .map_err(|e| format!("failed to signal openvpn stop: {e}"))?;
        } else {
            upsert_function_status(
                &app,
                &function_store,
                OPENVPN_FUNCTION_ID,
                "idle",
                "No running admin session",
                None,
            )?;
        }

        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        return stop_openvpn_pty_session_service(
            &app,
            &session_store,
            &function_store,
            OPENVPN_FUNCTION_ID,
            close_pty_internal,
            emit_pty_exit,
            update_functions_for_session_exit,
            upsert_function_status,
            lock_err,
        );
    }
}

#[tauri::command]
pub fn stop_openvpn_function(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    function_store: State<'_, FunctionStore>,
    admin_store: State<'_, OpenVpnAdminStore>,
) -> Result<(), String> {
    stop_openvpn_action(app, session_store, function_store, admin_store)
}

pub(crate) fn restart_openvpn_action(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let admin_store = app.state::<OpenVpnAdminStore>();
        if let Some(active) = get_openvpn_admin_session(&admin_store)? {
            reset_openvpn_reconnect(&app, false);
            let _ = fs::remove_file(&active.stop_path);
            match fs::write(&active.control_path, "restart") {
                Ok(_) => {
                    let function_store = app.state::<FunctionStore>();
                    upsert_function_status(
                        &app,
                        &function_store,
                        OPENVPN_FUNCTION_ID,
                        "restarting",
                        "Requesting elevated OpenVPN child restart",
                        Some(active.id.clone()),
                    )?;
                    return Ok(());
                }
                Err(err) => {
                    eprintln!(
                        "CC OpenVPN restart: helper control signal failed for session {}: {}",
                        active.id, err
                    );
                }
            }
        }
    }

    let is_running = {
        let function_store = app.state::<FunctionStore>();
        let running = lock_err(&function_store.statuses)?
            .get(OPENVPN_FUNCTION_ID)
            .and_then(|status| status.session_id.clone())
            .is_some();
        running
    };

    if is_running {
        let session_store = app.state::<SessionStore>();
        let function_store = app.state::<FunctionStore>();
        let admin_store = app.state::<OpenVpnAdminStore>();

        stop_openvpn_action(app.clone(), session_store, function_store, admin_store)?;
    } else {
        let function_store = app.state::<FunctionStore>();
        upsert_function_status(
            &app,
            &function_store,
            OPENVPN_FUNCTION_ID,
            "starting",
            "CC restart requested; OpenVPN is idle, starting directly",
            None,
        )?;

        let session_store = app.state::<SessionStore>();
        let function_store = app.state::<FunctionStore>();
        let admin_store = app.state::<OpenVpnAdminStore>();

        return start_openvpn_manual_action(
            String::new(),
            String::new(),
            String::new(),
            app.clone(),
            session_store,
            function_store,
            admin_store,
        )
        .map(|_| ());
    }

    let restart_app = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(2));

        let session_store = restart_app.state::<SessionStore>();
        let function_store = restart_app.state::<FunctionStore>();
        let admin_store = restart_app.state::<OpenVpnAdminStore>();

        if let Err(err) = start_openvpn_manual_action(
            String::new(),
            String::new(),
            String::new(),
            restart_app.clone(),
            session_store,
            function_store,
            admin_store,
        ) {
            let function_store = restart_app.state::<FunctionStore>();
            let _ = upsert_function_status(
                &restart_app,
                &function_store,
                OPENVPN_FUNCTION_ID,
                "error",
                &format!("CC restart failed: {err}"),
                None,
            );
        }
    });

    Ok(())
}
