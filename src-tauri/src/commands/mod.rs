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
use crate::models::PtyExitPayload;
use crate::services::{
    boundary_token_summary as boundary_token_summary_service, now_ms, BoundaryTokenSummary,
};
use crate::stores::FunctionStore;
#[cfg(target_os = "windows")]
use crate::CREATE_NO_WINDOW;
use crate::{BOUNDARY_AUTH_FUNCTION_ID, OPENVPN_FUNCTION_ID};
use scp_core::runtime::BoundaryTokenStatus;

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
    let changed = function_store.finish_session(
        &PtyExitPayload {
            id: session_id.to_string(),
            exit_code,
            reason: reason.to_string(),
        },
        BOUNDARY_AUTH_FUNCTION_ID,
        now_ms(),
        || {
            boundary_token_summary(app)
                .ok()
                .map(|summary| BoundaryTokenStatus {
                    message: summary.message,
                    source: summary.source.to_string(),
                })
        },
    )?;
    let openvpn_terminal = changed
        .iter()
        .find(|status| status.function_id == OPENVPN_FUNCTION_ID)
        .map(|status| (status.state.clone(), status.message.clone()));

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
