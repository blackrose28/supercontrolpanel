use std::{
    path::PathBuf,
    process::Command,
    sync::{atomic::{AtomicBool, Ordering}, Arc},
    thread,
    time::Duration,
};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use portable_pty::CommandBuilder;
use serde::Deserialize;
use reqwest::blocking::Client;
use serde_json::json;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::{
    models::{
        AppConfig, BoundaryAuthKind, BoundaryConfig, BoundaryOidcLoginMode,
        PtyExitPayload,
    },
    services::{apply_boundary_env, boundary_app_token, boundary_cli_path, lock_err},
    stores::{BoundaryAutomationStore, FunctionStore, SessionStore},
};

pub struct BoundaryTokenSummary {
    pub source: &'static str,
    pub message: String,
}

pub enum BoundaryAuthLaunch {
    AutomatedOidc,
    Cli(CommandBuilder),
}

pub struct BoundaryAuthStartPlan {
    pub launch: BoundaryAuthLaunch,
    pub status_label: String,
    pub password_to_send: Option<String>,
}

pub fn prepare_boundary_auth_start(
    cfg: &AppConfig,
    apply_boundary_token_arg_if_present_builder: fn(&mut CommandBuilder) -> Result<(), String>,
    apply_boundary_env_builder: fn(&mut CommandBuilder, &AppConfig) -> Result<(), String>,
) -> Result<BoundaryAuthStartPlan, String> {
    let am = cfg.boundary.auth_method_id.trim();
    let addr = cfg.boundary.addr.trim();
    match cfg.boundary.auth_kind {
        BoundaryAuthKind::Oidc => {
            if am.is_empty() || addr.is_empty() {
                return Err(
                    "OIDC: set Controller URL and OIDC auth method ID (amoidc_…) in Config, then Save."
                        .into(),
                );
            }
            if cfg.boundary.oidc_login_mode == BoundaryOidcLoginMode::Automated
                && (cfg.boundary.oidc_username.trim().is_empty()
                    || cfg.boundary.oidc_password.is_empty())
            {
                return Err(
                    "OIDC automation: set OIDC username and password in Config, then Save.".into(),
                );
            }
        }
        BoundaryAuthKind::Password => {
            if am.is_empty()
                || cfg.boundary.login_name.trim().is_empty()
                || cfg.boundary.password.is_empty()
            {
                return Err(
                    "Password: set auth method ID (ampw_…), login name, and password in Config, then Save."
                        .into(),
                );
            }
        }
    }

    let is_oidc_automation = cfg.boundary.auth_kind == BoundaryAuthKind::Oidc
        && cfg.boundary.oidc_login_mode == BoundaryOidcLoginMode::Automated;

    if is_oidc_automation {
        return Ok(BoundaryAuthStartPlan {
            launch: BoundaryAuthLaunch::AutomatedOidc,
            status_label: "Boundary OIDC (automated)".to_string(),
            password_to_send: None,
        });
    }

    let exe = boundary_cli_path(cfg);
    let mut cmd = CommandBuilder::new(exe);
    cmd.arg("authenticate");
    apply_boundary_token_arg_if_present_builder(&mut cmd)?;
    apply_boundary_env_builder(&mut cmd, cfg)?;
    match cfg.boundary.auth_kind {
        BoundaryAuthKind::Oidc => {
            cmd.arg("oidc");
            cmd.arg("-auth-method-id");
            cmd.arg(am);
            Ok(BoundaryAuthStartPlan {
                launch: BoundaryAuthLaunch::Cli(cmd),
                status_label: "Boundary OIDC (browser)".to_string(),
                password_to_send: None,
            })
        }
        BoundaryAuthKind::Password => {
            cmd.arg("password");
            cmd.arg("-auth-method-id");
            cmd.arg(am);
            cmd.arg("-login-name");
            cmd.arg(cfg.boundary.login_name.trim());
            Ok(BoundaryAuthStartPlan {
                launch: BoundaryAuthLaunch::Cli(cmd),
                status_label: "Boundary password login".to_string(),
                password_to_send: Some(cfg.boundary.password.clone()),
            })
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BoundaryApiAuthenticateResponse {
    #[serde(default)]
    pub attributes: BoundaryApiAuthenticateAttributes,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct BoundaryApiAuthenticateAttributes {
    #[serde(default)]
    pub auth_url: String,
    #[serde(default)]
    pub token_id: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub expiration_time: String,
}

pub fn boundary_token_summary(
    app: &AppHandle,
    load_app_config: fn(&AppHandle) -> Result<AppConfig, String>,
    #[cfg(target_os = "windows")] create_no_window: u32,
) -> Result<BoundaryTokenSummary, String> {
    let cfg = load_app_config(app)?;
    let exe = boundary_cli_path(&cfg);
    let _addr = cfg.boundary.addr.trim();
    let using_app_token = boundary_app_token()?
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);
    if exe != "boundary" && !PathBuf::from(&exe).exists() {
        return Err(format!("Boundary CLI not found: {exe}"));
    }
    let mut cmd = Command::new(&exe);
    cmd.arg("config");
    cmd.arg("get-token");
    apply_boundary_env(&mut cmd, &cfg)?;
    #[cfg(target_os = "windows")]
    cmd.creation_flags(create_no_window);
    let out = cmd
        .output()
        .map_err(|e| format!("failed to run `{exe} config get-token`: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("no token".into());
        }
        return Err(stderr);
    }
    let token_trimmed = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token_trimmed.is_empty() {
        return Err("no token".into());
    }
    Ok(BoundaryTokenSummary {
        source: if using_app_token { "app" } else { "cli" },
        message: if using_app_token {
            "Authenticated via hidden OIDC window. Token is stored by SuperControlPanel and injected into Boundary commands."
                .to_string()
        } else {
            "Authenticated via Boundary CLI. Token is stored in the CLI keyring.".to_string()
        },
    })
}

pub fn boundary_oidc_init_script(
    cfg: &BoundaryConfig,
    generate_totp_code: fn(&str) -> Result<String, String>,
) -> String {
    let username =
        serde_json::to_string(cfg.oidc_username.trim()).unwrap_or_else(|_| "\"\"".into());
    let password = serde_json::to_string(&cfg.oidc_password).unwrap_or_else(|_| "\"\"".into());
    let otp = match generate_totp_code(&cfg.oidc_otp_secret) {
        Ok(code) => serde_json::to_string(&code).unwrap_or_else(|_| "\"\"".into()),
        Err(_) => "\"\"".into(),
    };
    format!(
        r#"
(() => {{
  const cfg = {{ username: {username}, password: {password}, otp: {otp} }};
  const visible = (el) => !!el && !el.disabled && el.type !== "hidden" && el.offsetParent !== null;
  const findFirst = (selectors) => {{
    for (const selector of selectors) {{
      const el = document.querySelector(selector);
      if (visible(el)) return el;
    }}
    return null;
  }};
  const setValue = (el, value) => {{
    if (!el || !value) return false;
    el.focus();
    el.value = value;
    el.dispatchEvent(new Event("input", {{ bubbles: true }}));
    el.dispatchEvent(new Event("change", {{ bubbles: true }}));
    return true;
  }};
  const submit = (el) => {{
    if (!el) return false;
    const form = el.closest("form");
    if (form) {{
      if (typeof form.requestSubmit === "function") form.requestSubmit();
      else form.submit();
      return true;
    }}
    const btn = findFirst([
      "button[type=submit]",
      "input[type=submit]",
      "button[id*=login i]",
      "button[name*=login i]",
      "button[id*=verify i]",
      "button[name*=verify i]",
      "button"
    ]);
    if (btn) {{
      btn.click();
      return true;
    }}
    return false;
  }};
  const tick = () => {{
    const pwd = findFirst(["input[type=password]", "input[name*=pass i]", "input[id*=pass i]"]);
    const otpField = findFirst([
      "input[autocomplete='one-time-code']",
      "input[name*=otp i]",
      "input[id*=otp i]",
      "input[name*=totp i]",
      "input[id*=totp i]",
      "input[name*=code i]",
      "input[id*=code i]"
    ]);
    if (otpField && cfg.otp && !otpField.value) {{
      setValue(otpField, cfg.otp);
      submit(otpField);
      return;
    }}
    const user = findFirst([
      "input[autocomplete=username]",
      "input[type=email]",
      "input[name*=user i]",
      "input[id*=user i]",
      "input[name*=email i]",
      "input[id*=email i]",
      "input[type=text]"
    ]);
    if (user && cfg.username && !user.value) setValue(user, cfg.username);
    if (pwd && cfg.password && !pwd.value) {{
      setValue(pwd, cfg.password);
      submit(pwd);
      return;
    }}
    if (user && cfg.username && !pwd) {{
      const next = findFirst([
        "button[type=submit]",
        "input[type=submit]",
        "button[id*=next i]",
        "button[name*=next i]"
      ]);
      if (next) next.click();
    }}
  }};
  tick();
  setInterval(tick, 900);
}})();
"#
    )
}

pub fn stop_boundary_automation_session(
    app: &AppHandle,
    automation_store: &BoundaryAutomationStore,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    boundary_oidc_window_label: &str,
    close_pty_internal: fn(&str, &SessionStore) -> Result<Option<PtyExitPayload>, String>,
    emit_pty_exit: fn(&AppHandle, PtyExitPayload),
    update_functions_for_session_exit: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        Option<i32>,
        &str,
    ) -> Result<(), String>,
) -> Result<(), String> {
    let prior = {
        let mut guard = lock_err(&automation_store.session)?;
        guard.take()
    };
    if let Some(prior) = prior {
        prior.cancel.store(true, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window(boundary_oidc_window_label) {
            let _ = window.close();
        }
        if let Some(payload) = close_pty_internal(&prior.session_id, session_store)? {
            emit_pty_exit(app, payload.clone());
            let _ = update_functions_for_session_exit(
                app,
                function_store,
                &prior.session_id,
                payload.exit_code,
                &payload.reason,
            );
        }
    }
    Ok(())
}

pub fn complete_boundary_automation_session(
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    session_id: &str,
    exit_code: Option<i32>,
    reason: &str,
    close_pty_internal: fn(&str, &SessionStore) -> Result<Option<PtyExitPayload>, String>,
    emit_pty_exit: fn(&AppHandle, PtyExitPayload),
    update_functions_for_session_exit: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        Option<i32>,
        &str,
    ) -> Result<(), String>,
) {
    if let Ok(Some(payload)) = close_pty_internal(session_id, session_store) {
        emit_pty_exit(app, payload.clone());
        let _ = update_functions_for_session_exit(
            app,
            function_store,
            session_id,
            payload.exit_code,
            &payload.reason,
        );
    } else {
        let _ = update_functions_for_session_exit(app, function_store, session_id, exit_code, reason);
    }
}

pub fn stop_boundary_auth(
    app: &AppHandle,
    session_store: &SessionStore,
    function_store: &FunctionStore,
    automation_store: &BoundaryAutomationStore,
    boundary_auth_function_id: &str,
    boundary_oidc_window_label: &str,
    close_pty_internal: fn(&str, &SessionStore) -> Result<Option<PtyExitPayload>, String>,
    emit_pty_exit: fn(&AppHandle, PtyExitPayload),
    update_functions_for_session_exit: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        Option<i32>,
        &str,
    ) -> Result<(), String>,
    boundary_token_summary: fn(&AppHandle) -> Result<BoundaryTokenSummary, String>,
    upsert_boundary_status: fn(
        &AppHandle,
        &FunctionStore,
        &str,
        &str,
        Option<&str>,
        Option<String>,
    ) -> Result<(), String>,
) -> Result<(), String> {
    stop_boundary_automation_session(
        app,
        automation_store,
        session_store,
        function_store,
        boundary_oidc_window_label,
        close_pty_internal,
        emit_pty_exit,
        update_functions_for_session_exit,
    )?;
    let sid = match lock_err(&function_store.statuses)?
        .get(boundary_auth_function_id)
        .and_then(|s| s.session_id.clone())
    {
        Some(s) => s,
        None => return Ok(()),
    };
    if let Some(p) = close_pty_internal(&sid, session_store)? {
        emit_pty_exit(app, p.clone());
        let _ = update_functions_for_session_exit(app, function_store, &sid, p.exit_code, &p.reason);
    } else {
        let (state, msg, source) = match boundary_token_summary(app) {
            Ok(summary) => ("authenticated", summary.message, Some(summary.source)),
            Err(_) => (
                "unauthenticated",
                "Login was stopped; no Boundary token is available.".to_string(),
                None,
            ),
        };
        upsert_boundary_status(app, function_store, state, &msg, source, None)?;
    }
    Ok(())
}


#[allow(clippy::too_many_arguments)]
pub fn run_boundary_oidc_automation(
    app: AppHandle,
    session_id: String,
    cfg: BoundaryConfig,
    cancel: Arc<AtomicBool>,
    boundary_oidc_window_label: &str,
    keyring_boundary_app_token: &str,
    boundary_addr_base: fn(&str) -> String,
    emit_pty_output_line: fn(&AppHandle, &str, String),
    boundary_http_client: fn() -> Result<Client, String>,
    boundary_oidc_init_script: fn(&BoundaryConfig) -> String,
    keyring_set_or_delete: fn(&str, &str) -> Result<(), String>,
    complete_boundary_automation_session: fn(&AppHandle, &SessionStore, &FunctionStore, &str, Option<i32>, &str),
) {
    let function_store = app.state::<FunctionStore>();
    let session_store = app.state::<SessionStore>();
    let automation_store = app.state::<BoundaryAutomationStore>();

    let finish = |exit_code: Option<i32>, reason: &str| {
        if let Ok(mut guard) = automation_store.session.lock() {
            guard.take();
        }
        if let Some(window) = app.get_webview_window(boundary_oidc_window_label) {
            let _ = window.close();
        }
        complete_boundary_automation_session(
            &app,
            &session_store,
            &function_store,
            &session_id,
            exit_code,
            reason,
        );
    };

    let addr = boundary_addr_base(&cfg.addr);
    let auth_endpoint = format!(
        "{addr}/v1/auth-methods/{}:authenticate",
        cfg.auth_method_id.trim()
    );
    emit_pty_output_line(
        &app,
        &session_id,
        format!("Starting automated OIDC login against {addr}"),
    );

    let client = match boundary_http_client() {
        Ok(client) => client,
        Err(err) => {
            emit_pty_output_line(&app, &session_id, format!("HTTP client error: {err}"));
            finish(Some(1), &err);
            return;
        }
    };

    let login_resp = match client
        .post(&auth_endpoint)
        .json(&json!({
            "command": "start"
        }))
        .send()
    {
        Ok(resp) => resp,
        Err(err) => {
            let msg = format!("Boundary login request failed: {err}");
            emit_pty_output_line(&app, &session_id, msg.clone());
            finish(Some(1), &msg);
            return;
        }
    };
    if !login_resp.status().is_success() {
        let status = login_resp.status();
        let body = login_resp.text().unwrap_or_default();
        let msg = format!(
            "Boundary login request failed: HTTP {status} {}",
            body.trim()
        );
        emit_pty_output_line(&app, &session_id, msg.clone());
        finish(Some(1), &msg);
        return;
    }
    let login_data: BoundaryApiAuthenticateResponse = match login_resp.json() {
        Ok(data) => data,
        Err(err) => {
            let msg = format!("Failed to decode Boundary login response: {err}");
            emit_pty_output_line(&app, &session_id, msg.clone());
            finish(Some(1), &msg);
            return;
        }
    };
    if login_data.attributes.auth_url.trim().is_empty()
        || login_data.attributes.token_id.trim().is_empty()
    {
        let msg = "Boundary login response did not include auth_url/token_id".to_string();
        emit_pty_output_line(&app, &session_id, msg.clone());
        finish(Some(1), &msg);
        return;
    }

    let parsed_auth_url = match login_data.attributes.auth_url.parse() {
        Ok(url) => url,
        Err(err) => {
            let msg = format!("Invalid OIDC auth URL: {err}");
            emit_pty_output_line(&app, &session_id, msg.clone());
            finish(Some(1), &msg);
            return;
        }
    };
    let builder = WebviewWindowBuilder::new(
        &app,
        boundary_oidc_window_label,
        WebviewUrl::External(parsed_auth_url),
    )
    .visible(false)
    .title("Boundary OIDC Automation")
    .initialization_script_for_all_frames(boundary_oidc_init_script(&cfg));
    if let Err(err) = builder.build() {
        let msg = format!("Failed to open automation webview: {err}");
        emit_pty_output_line(&app, &session_id, msg.clone());
        finish(Some(1), &msg);
        return;
    }

    emit_pty_output_line(
        &app,
        &session_id,
        "Hidden OIDC window launched. Waiting for callback and token issuance.".to_string(),
    );
    let token_id = login_data.attributes.token_id;
    let mut attempts: u32 = 0;
    loop {
        if cancel.load(Ordering::SeqCst) {
            emit_pty_output_line(&app, &session_id, "OIDC automation cancelled.".to_string());
            finish(Some(1), "cancelled");
            return;
        }
        thread::sleep(Duration::from_secs(1));
        attempts += 1;
        let token_resp = match client
            .post(&auth_endpoint)
            .json(&json!({
                "command": "token",
                "token_id": token_id,
                "attributes": { "token_id": token_id }
            }))
            .send()
        {
            Ok(resp) => resp,
            Err(err) => {
                let msg = format!("Boundary token polling failed: {err}");
                emit_pty_output_line(&app, &session_id, msg.clone());
                finish(Some(1), &msg);
                return;
            }
        };
        if token_resp.status().is_success() {
            let token_data: BoundaryApiAuthenticateResponse = match token_resp.json() {
                Ok(data) => data,
                Err(err) => {
                    let msg = format!("Failed to decode token response: {err}");
                    emit_pty_output_line(&app, &session_id, msg.clone());
                    finish(Some(1), &msg);
                    return;
                }
            };
            if !token_data.attributes.token.trim().is_empty() {
                if let Err(err) = keyring_set_or_delete(
                    keyring_boundary_app_token,
                    token_data.attributes.token.trim(),
                ) {
                    let msg = format!("Failed to store Boundary token: {err}");
                    emit_pty_output_line(&app, &session_id, msg.clone());
                    finish(Some(1), &msg);
                    return;
                }
                emit_pty_output_line(
                    &app,
                    &session_id,
                    if token_data.attributes.expiration_time.trim().is_empty() {
                        "OIDC login succeeded. Token stored in the app secure store.".to_string()
                    } else {
                        format!(
                            "OIDC login succeeded. Token expires at {}.",
                            token_data.attributes.expiration_time.trim()
                        )
                    },
                );
                finish(Some(0), "automated OIDC login complete");
                return;
            }
        } else if !matches!(token_resp.status().as_u16(), 400 | 404) {
            let status = token_resp.status();
            let body = token_resp.text().unwrap_or_default();
            let msg = format!(
                "Boundary token polling failed: HTTP {status} {}",
                body.trim()
            );
            emit_pty_output_line(&app, &session_id, msg.clone());
            finish(Some(1), &msg);
            return;
        } else if attempts % 5 == 0 {
            emit_pty_output_line(
                &app,
                &session_id,
                "Still waiting for the IdP to complete login and callback.".to_string(),
            );
        }
    }
}
