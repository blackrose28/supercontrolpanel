//! Boundary CLI target search, token status checks, and capability-gated authentication.
use crate::error::CoreError;
use crate::models::{
    now_ms, AppConfig, BoundaryAuthKind, BoundaryOidcLoginMode, BoundaryTargetSummary,
    FunctionStatus, StartFunctionResult, BOUNDARY_AUTH_FUNCTION_ID, KEYRING_BOUNDARY_APP_TOKEN,
    KEYRING_BOUNDARY_PASSWORD,
};
use crate::runtime::{ProfileRuntime, StatusUpdate, SuperviseHooks};
use crate::secrets::{KeyringBackend, NamespacedSecretStore};
use crate::ssh_ops::{build_boundary_env, resolve_boundary_executable};
use crate::supervisor::{CancelToken, ChildLaunch, SpawnMode};
use parking_lot::Mutex;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const DEFAULT_TARGET_SEARCH_LIMIT: usize = 50;
pub const MAX_TARGET_SEARCH_LIMIT: usize = 100;

/// Parses Boundary `targets list -format json` output with optional case-insensitive filtering.
pub fn parse_boundary_targets(
    output: &[u8],
    filter: Option<&str>,
) -> Result<Vec<BoundaryTargetSummary>, String> {
    let val: serde_json::Value =
        serde_json::from_slice(output).map_err(|e| format!("invalid JSON: {e}"))?;
    let items = val
        .get("items")
        .and_then(|a| a.as_array())
        .ok_or_else(|| "expected items array in boundary output".to_string())?;
    let needle = filter
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let mut results = Vec::new();
    for item in items {
        let Some(entry) = parse_target_item(item, needle.as_deref()) else {
            continue;
        };
        results.push(entry);
    }
    Ok(results)
}

fn parse_target_item(
    item: &serde_json::Value,
    needle: Option<&str>,
) -> Option<BoundaryTargetSummary> {
    let obj = item.as_object()?;
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if id.is_empty() {
        return None;
    }
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let target_type = obj
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let attrs = obj.get("attributes");
    let description = obj
        .get("description")
        .or_else(|| attrs.and_then(|a| a.get("description")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let address = obj
        .get("address")
        .or_else(|| attrs.and_then(|a| a.get("address")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(f) = needle {
        let matched = name.to_lowercase().contains(f)
            || id.to_lowercase().contains(f)
            || description.to_lowercase().contains(f);
        if !matched {
            return None;
        }
    }
    Some(BoundaryTargetSummary {
        id,
        name,
        target_type,
        description,
        address,
    })
}

pub(crate) fn search_boundary_targets_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    filter: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<BoundaryTargetSummary>, CoreError> {
    let cap = limit.unwrap_or(DEFAULT_TARGET_SEARCH_LIMIT);
    if cap == 0 || cap > MAX_TARGET_SEARCH_LIMIT {
        return Err(CoreError::invalid_arguments(
            "invalid_limit",
            format!("limit must be between 1 and {MAX_TARGET_SEARCH_LIMIT}"),
        ));
    }
    let _guard = rt.gate().enter()?;
    let sync = rt.get_sync_settings()?;
    let cfg = rt.get_app_config()?;
    let exe = resolve_checked_boundary(&sync.boundary_cli_path)?;
    let app_token = rt.read_optional_secret(KEYRING_BOUNDARY_APP_TOKEN);
    let secrets = collect_redacted_secrets(rt, &cfg, app_token.as_deref());
    let mut args = vec!["targets".to_string(), "list".to_string()];
    if has_non_empty(app_token.as_deref()) {
        args.extend(["-token".to_string(), "env://BOUNDARY_TOKEN".to_string()]);
    }
    args.extend([
        "-recursive".to_string(),
        "-format".to_string(),
        "json".to_string(),
    ]);
    let out = run_boundary_cmd(
        &exe,
        &args,
        &cfg,
        rt.credential_namespace(),
        app_token.as_deref(),
    )?;
    if !out.status.success() {
        let raw_err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if raw_err.is_empty() {
            "Boundary targets list failed (check Boundary authentication)".to_string()
        } else {
            redact_values(&raw_err, &secrets)
        };
        return Err(CoreError::operation_failed(
            "boundary_search_failed",
            msg,
            false,
            "check_boundary_auth",
        ));
    }
    let mut rows = parse_boundary_targets(&out.stdout, filter).map_err(|e| {
        CoreError::operation_failed("boundary_parse_failed", e, false, "inspect_boundary_output")
    })?;
    rows.truncate(cap);
    Ok(rows)
}

pub(crate) fn refresh_boundary_status_impl<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
) -> Result<FunctionStatus, CoreError> {
    let _guard = rt.gate().enter()?;
    let sync = rt.get_sync_settings()?;
    let cfg = rt.get_app_config()?;
    let exe = resolve_checked_boundary(&sync.boundary_cli_path)?;
    let app_token = rt.read_optional_secret(KEYRING_BOUNDARY_APP_TOKEN);
    let has_app_token = has_non_empty(app_token.as_deref());
    let args = vec!["config".to_string(), "get-token".to_string()];
    let out = run_boundary_cmd(
        &exe,
        &args,
        &cfg,
        rt.credential_namespace(),
        app_token.as_deref(),
    )?;
    let has_cli_output =
        out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty();
    let (state, source, msg) = if has_app_token || has_cli_output {
        let src = if has_app_token { "app" } else { "cli" };
        let detail = if src == "app" {
            "Authenticated via SuperControlPanel. Token is stored in the profile keyring and injected into Boundary commands."
        } else {
            "Authenticated via Boundary CLI. Token is stored in the CLI keyring."
        };
        ("authenticated", Some(src.to_string()), detail.to_string())
    } else {
        ("idle", None, "Not authenticated".to_string())
    };
    rt.update_status_snapshot(StatusUpdate {
        function_id: BOUNDARY_AUTH_FUNCTION_ID,
        state,
        message: msg,
        boundary_token_source: Some(source),
        session_id: None,
    });
    rt.get_function_status(BOUNDARY_AUTH_FUNCTION_ID)
}

/// Options for starting supervised Boundary authentication.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoundaryAuthOptions {
    /// Allow OIDC through the stock Boundary CLI browser login. Only set when an interactive
    /// user is present to complete the login; headless/MCP callers keep the default `false`.
    pub allow_browser_oidc: bool,
}

/// How a started Boundary login authenticates.
enum BoundaryAuthPlan {
    Password(String),
    BrowserOidc,
}

pub(crate) fn start_boundary_auth_impl<B: KeyringBackend + 'static>(
    rt: &ProfileRuntime<B>,
    cancel: &CancelToken,
) -> Result<StartFunctionResult, CoreError> {
    start_boundary_auth_with_size_impl(rt, None, None, cancel)
}

pub(crate) fn start_boundary_auth_with_size_impl<B: KeyringBackend + 'static>(
    rt: &ProfileRuntime<B>,
    cols: Option<u16>,
    rows: Option<u16>,
    cancel: &CancelToken,
) -> Result<StartFunctionResult, CoreError> {
    start_boundary_auth_full_impl(rt, cols, rows, cancel, BoundaryAuthOptions::default())
}

pub(crate) fn start_boundary_auth_with_options_impl<B: KeyringBackend + 'static>(
    rt: &ProfileRuntime<B>,
    cancel: &CancelToken,
    options: BoundaryAuthOptions,
) -> Result<StartFunctionResult, CoreError> {
    start_boundary_auth_full_impl(rt, None, None, cancel, options)
}

fn start_boundary_auth_full_impl<B: KeyringBackend + 'static>(
    rt: &ProfileRuntime<B>,
    cols: Option<u16>,
    rows: Option<u16>,
    cancel: &CancelToken,
    options: BoundaryAuthOptions,
) -> Result<StartFunctionResult, CoreError> {
    let cfg = rt.get_app_config()?;
    let plan = validate_boundary_auth_mode(rt, &cfg, options)?;
    let sync = rt.get_sync_settings()?;
    let exe = resolve_checked_boundary(&sync.boundary_cli_path)?;
    let mut launch = build_boundary_auth_launch(rt, &cfg, exe, &plan);
    if let SpawnMode::Pty { cols: c, rows: r } = &mut launch.mode {
        if let Some(cols) = cols {
            *c = cols.max(80);
        }
        if let Some(rows) = rows {
            *r = rows.max(24);
        }
    }
    let out_buf = Arc::new(Mutex::new(String::new()));
    let (password, label) = match plan {
        BoundaryAuthPlan::Password(pw) => (Some(pw), "Boundary password login"),
        BoundaryAuthPlan::BrowserOidc => (None, "Boundary OIDC login"),
    };
    let hooks = build_boundary_auth_hooks(rt, password, Arc::clone(&out_buf));
    let session_id = rt.start_supervised_with_hooks(launch, cancel, None, label, hooks)?;
    Ok(StartFunctionResult {
        function_id: BOUNDARY_AUTH_FUNCTION_ID.to_string(),
        session_id,
    })
}

const OIDC_NEEDS_INTERACTIVE_AUTOMATED: &str = "Automated Boundary OIDC requires the desktop hidden webview. Run `scpctl boundary auth --interactive` to log in through your browser instead";
const OIDC_NEEDS_INTERACTIVE_BROWSER: &str = "Boundary OIDC login needs an interactive user. Run `scpctl boundary auth --interactive` to log in through your browser";

fn validate_boundary_auth_mode<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    cfg: &AppConfig,
    options: BoundaryAuthOptions,
) -> Result<BoundaryAuthPlan, CoreError> {
    match cfg.boundary.auth_kind {
        BoundaryAuthKind::Oidc if options.allow_browser_oidc => {
            validate_oidc_inputs(cfg)?;
            Ok(BoundaryAuthPlan::BrowserOidc)
        }
        BoundaryAuthKind::Oidc => match cfg.boundary.oidc_login_mode {
            BoundaryOidcLoginMode::Automated => Err(CoreError::capability_unavailable(
                "webview_oidc_desktop_only",
                OIDC_NEEDS_INTERACTIVE_AUTOMATED,
                "use_password_auth_or_desktop",
            )),
            BoundaryOidcLoginMode::Browser => Err(CoreError::capability_unavailable(
                "browser_oidc_unverified",
                OIDC_NEEDS_INTERACTIVE_BROWSER,
                "use_password_auth_or_desktop",
            )),
        },
        BoundaryAuthKind::Password => {
            validate_password_auth_inputs(rt, cfg).map(BoundaryAuthPlan::Password)
        }
    }
}

fn validate_oidc_inputs(cfg: &AppConfig) -> Result<(), CoreError> {
    if cfg.boundary.addr.trim().is_empty() || cfg.boundary.auth_method_id.trim().is_empty() {
        return Err(CoreError::invalid_arguments(
            "boundary_config_incomplete",
            "Boundary addr and authMethodId are required for OIDC auth",
        ));
    }
    Ok(())
}

fn validate_password_auth_inputs<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    cfg: &AppConfig,
) -> Result<String, CoreError> {
    if cfg.boundary.addr.trim().is_empty()
        || cfg.boundary.auth_method_id.trim().is_empty()
        || cfg.boundary.login_name.trim().is_empty()
    {
        return Err(CoreError::invalid_arguments(
            "boundary_config_incomplete",
            "Boundary addr, authMethodId, and loginName are required for password auth",
        ));
    }
    let pw = rt
        .read_optional_secret(KEYRING_BOUNDARY_PASSWORD)
        .filter(|s| !s.is_empty())
        .or_else(|| (!cfg.boundary.password.is_empty()).then(|| cfg.boundary.password.clone()));
    pw.ok_or_else(|| {
        CoreError::capability_unavailable(
            "credentials_missing",
            "Boundary password is not configured in profile keyring",
            "configure_credentials",
        )
    })
}

fn build_boundary_auth_launch<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    cfg: &AppConfig,
    exe: String,
    plan: &BoundaryAuthPlan,
) -> ChildLaunch {
    let (mut env, mut env_remove) = build_boundary_env(cfg, rt.credential_namespace(), None);
    env_remove.push("BOUNDARY_TOKEN".to_string());
    let method_id = cfg.boundary.auth_method_id.trim().to_string();
    let args = match plan {
        BoundaryAuthPlan::Password(password) => {
            env.push(("BOUNDARY_AUTH_PASSWORD".to_string(), password.clone()));
            vec![
                "authenticate".to_string(),
                "password".to_string(),
                "-auth-method-id".to_string(),
                method_id,
                "-login-name".to_string(),
                cfg.boundary.login_name.trim().to_string(),
                "-password".to_string(),
                "env://BOUNDARY_AUTH_PASSWORD".to_string(),
                "-format".to_string(),
                "json".to_string(),
            ]
        }
        BoundaryAuthPlan::BrowserOidc => vec![
            "authenticate".to_string(),
            "oidc".to_string(),
            "-auth-method-id".to_string(),
            method_id,
            "-format".to_string(),
            "json".to_string(),
        ],
    };
    ChildLaunch {
        function_id: BOUNDARY_AUTH_FUNCTION_ID.to_string(),
        program: exe,
        args,
        cwd: None,
        env,
        env_remove,
        mode: SpawnMode::Pty {
            cols: cfg.app.default_terminal_cols.max(80),
            rows: cfg.app.default_terminal_rows.max(24),
        },
        kill_tree: true,
        kind: "boundary-auth",
    }
}

fn build_boundary_auth_hooks<B: KeyringBackend + 'static>(
    rt: &ProfileRuntime<B>,
    password: Option<String>,
    out_buf: Arc<Mutex<String>>,
) -> SuperviseHooks {
    let out_capture = Arc::clone(&out_buf);
    let sent_pw = Arc::new(AtomicBool::new(false));
    let sup = Arc::clone(rt.supervisor());
    let on_output = Some(Arc::new(move |sid: &str, chunk: &str, _is_pty: bool| {
        {
            let mut buf = out_capture.lock();
            buf.push_str(chunk);
            if buf.len() > 64 * 1024 {
                let trim = buf.len() - 64 * 1024;
                buf.drain(..trim);
            }
        }
        let Some(password) = password.as_deref() else {
            return;
        };
        if chunk.to_lowercase().contains("password") && !sent_pw.swap(true, Ordering::SeqCst) {
            let sup_w = Arc::clone(&sup);
            let sid_w = sid.to_string();
            let pw_w = format!("{password}\r");
            std::thread::spawn(move || {
                let _ = sup_w.write(&sid_w, &pw_w);
            });
        }
    }) as crate::supervisor::OutputCallback);

    let secrets = Arc::clone(rt.secrets_arc());
    let state = Arc::clone(rt.runtime_state_arc());
    let on_exit = Some(
        Arc::new(move |sid: &str, code: Option<i32>, _reason: &str| {
            if code != Some(0) {
                return;
            }
            let raw = out_buf.lock().clone();
            finalize_boundary_auth_success(sid, &raw, &secrets, &state);
        }) as crate::supervisor::ExitCallback,
    );

    SuperviseHooks { on_output, on_exit }
}

fn finalize_boundary_auth_success<B: KeyringBackend>(
    session_id: &str,
    raw_output: &str,
    secrets: &Arc<Mutex<NamespacedSecretStore<B>>>,
    state: &Arc<Mutex<crate::runtime::RuntimeState>>,
) {
    let mut source = "cli".to_string();
    if let Some(token) = extract_token_from_output(raw_output) {
        if secrets
            .lock()
            .set(KEYRING_BOUNDARY_APP_TOKEN, &token)
            .is_ok()
        {
            source = "app".to_string();
        }
    }
    let mut st = state.lock();
    if st
        .function_statuses
        .get(BOUNDARY_AUTH_FUNCTION_ID)
        .and_then(|s| s.session_id.as_deref())
        == Some(session_id)
    {
        st.function_statuses.insert(
            BOUNDARY_AUTH_FUNCTION_ID.to_string(),
            FunctionStatus {
                function_id: BOUNDARY_AUTH_FUNCTION_ID.to_string(),
                state: "authenticated".to_string(),
                message: if source == "app" {
                    "Authenticated via SuperControlPanel. Token stored in profile keyring.".into()
                } else {
                    "Authenticated via Boundary CLI.".into()
                },
                boundary_token_source: Some(source),
                session_id: Some(session_id.to_string()),
                updated_at_ms: now_ms(),
            },
        );
    }
}

fn extract_token_from_output(raw: &str) -> Option<String> {
    let start = raw.find('{')?;
    let val: serde_json::Value = serde_json::from_str(raw[start..].trim()).ok()?;
    val.pointer("/attributes/token")
        .or_else(|| val.pointer("/item/attributes/token"))
        .or_else(|| val.get("token"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn resolve_checked_boundary(configured_path: &str) -> Result<String, CoreError> {
    let sync = crate::models::SyncSettings {
        boundary_cli_path: configured_path.to_string(),
        ..Default::default()
    };
    let exe = resolve_boundary_executable(&sync)?;
    if exe != "boundary" && !Path::new(&exe).is_file() {
        return Err(CoreError::capability_unavailable(
            "executable_not_found",
            format!("Boundary CLI not found: {exe}"),
            "install_or_configure_boundary",
        ));
    }
    Ok(exe)
}

fn run_boundary_cmd(
    exe: &str,
    args: &[String],
    cfg: &AppConfig,
    namespace: &str,
    token: Option<&str>,
) -> Result<Output, CoreError> {
    let (env, env_remove) = build_boundary_env(cfg, namespace, token);
    let mut cmd = Command::new(exe);
    cmd.args(args);
    for key in &env_remove {
        cmd.env_remove(key);
    }
    for (k, v) in &env {
        cmd.env(k, v);
    }
    cmd.output().map_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            CoreError::capability_unavailable(
                "executable_not_found",
                format!("Boundary CLI not found: {exe}"),
                "install_or_configure_boundary",
            )
        } else {
            CoreError::operation_failed(
                "boundary_exec_failed",
                format!("Failed to execute Boundary CLI: {err}"),
                false,
                "check_boundary_cli",
            )
        }
    })
}

fn collect_redacted_secrets<B: KeyringBackend>(
    rt: &ProfileRuntime<B>,
    cfg: &AppConfig,
    app_token: Option<&str>,
) -> Vec<String> {
    let mut secrets = Vec::new();
    for cand in [
        app_token.map(ToOwned::to_owned),
        rt.read_optional_secret(KEYRING_BOUNDARY_PASSWORD),
        Some(cfg.boundary.password.clone()),
        Some(cfg.boundary.oidc_password.clone()),
    ]
    .into_iter()
    .flatten()
    {
        if !cand.trim().is_empty() {
            secrets.push(cand);
        }
    }
    secrets
}

fn redact_values(input: &str, secrets: &[String]) -> String {
    let mut out = input.to_string();
    for s in secrets {
        if s.len() >= 4 {
            out = out.replace(s, "[REDACTED]");
        }
    }
    out
}

fn has_non_empty(val: Option<&str>) -> bool {
    val.is_some_and(|s| !s.trim().is_empty())
}
