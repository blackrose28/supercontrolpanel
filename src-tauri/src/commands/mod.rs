pub mod boundary;
pub mod caddy;
pub mod config;
pub mod mcp;
pub mod openvpn;
pub mod pty;
pub mod remote_browser;
pub mod scp;
pub mod server;

pub use boundary::*;
pub use caddy::*;
pub use config::*;
pub use mcp::*;
pub use openvpn::*;
pub use pty::*;
pub use remote_browser::*;
pub use scp::*;
pub use server::*;

use tauri::{AppHandle, Emitter};

use crate::cc::telemetry::forward_log_event;
use crate::models::FunctionStatus;
use crate::services::{
    boundary_token_summary as boundary_token_summary_service, lock_err, now_ms,
    BoundaryTokenSummary,
};
use crate::stores::FunctionStore;
#[cfg(target_os = "windows")]
use crate::CREATE_NO_WINDOW;
use crate::{BOUNDARY_AUTH_FUNCTION_ID, OPENVPN_FUNCTION_ID};

pub(crate) fn upsert_function_status(
    app: &AppHandle,
    function_store: &FunctionStore,
    function_id: &str,
    state: &str,
    message: &str,
    session_id: Option<String>,
) -> Result<(), String> {
    crate::services::upsert_function_status(
        app,
        function_store,
        function_id,
        state,
        message,
        session_id,
    )
}

pub(crate) fn upsert_boundary_status(
    app: &AppHandle,
    function_store: &FunctionStore,
    state: &str,
    message: &str,
    boundary_token_source: Option<&str>,
    session_id: Option<String>,
) -> Result<(), String> {
    crate::services::upsert_boundary_status(
        app,
        function_store,
        state,
        message,
        boundary_token_source,
        session_id,
        BOUNDARY_AUTH_FUNCTION_ID,
    )
}

pub(crate) fn boundary_token_summary(app: &AppHandle) -> Result<BoundaryTokenSummary, String> {
    #[cfg(target_os = "windows")]
    {
        boundary_token_summary_service(app, config::load_app_config_from_disk, CREATE_NO_WINDOW)
    }
    #[cfg(not(target_os = "windows"))]
    {
        boundary_token_summary_service(app, config::load_app_config_from_disk)
    }
}

pub(crate) fn update_functions_for_session_exit(
    app: &AppHandle,
    function_store: &FunctionStore,
    session_id: &str,
    exit_code: Option<i32>,
    reason: &str,
) -> Result<(), String> {
    let mut changed = Vec::<FunctionStatus>::new();
    let mut openvpn_terminal: Option<(String, String)> = None;
    {
        let mut statuses = lock_err(&function_store.statuses)?;
        for status in statuses.values_mut() {
            if status.session_id.as_deref() != Some(session_id) {
                continue;
            }
            if status.function_id == BOUNDARY_AUTH_FUNCTION_ID {
                status.session_id = None;
                status.updated_at_ms = now_ms();
                let (state, message, source) = match boundary_token_summary(app) {
                    Ok(summary) => (
                        "authenticated".to_string(),
                        summary.message,
                        Some(summary.source.to_string()),
                    ),
                    Err(_) => {
                        if exit_code == Some(0) {
                            (
                                "authenticated".to_string(),
                                "Login completed, but the saved Boundary token could not be verified yet. Refresh Status or retry login."
                                    .to_string(),
                                None,
                            )
                        } else {
                            (
                                "unauthenticated".to_string(),
                                format!(
                                    "Not logged in or login interrupted (exit {}). {}",
                                    exit_code
                                        .map(|v| v.to_string())
                                        .unwrap_or_else(|| "?".to_string()),
                                    if reason.len() > 80 {
                                        format!("{}…", &reason[..80])
                                    } else {
                                        reason.to_string()
                                    }
                                ),
                                None,
                            )
                        }
                    }
                };
                status.state = state;
                status.message = message;
                status.boundary_token_source = source;
                changed.push(status.clone());
                continue;
            }
            let function_id = status.function_id.clone();
            status.state = if exit_code == Some(0) {
                "stopped".to_string()
            } else {
                "error".to_string()
            };
            status.message = format!(
                "PTY exited (code={}, reason={reason})",
                exit_code
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            );
            // Keep sessionId so tunnel logs remain queryable after PTY exits.
            status.updated_at_ms = now_ms();
            changed.push(status.clone());
            if function_id == OPENVPN_FUNCTION_ID {
                openvpn_terminal = Some((status.state.clone(), status.message.clone()));
            }
        }
    }

    if let Some((state, message)) = openvpn_terminal {
        openvpn::handle_openvpn_terminal_state(app, &state, &message);
    }
    for status in changed {
        let _ = app.emit("function-status", status.clone());
        forward_log_event(
            app,
            "function-status",
            status.session_id.as_deref(),
            Some(&status.function_id),
            &status,
        );
    }
    Ok(())
}
