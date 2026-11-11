pub use scp_core::runtime::{lock_err, now_ms};

use tauri::{AppHandle, Emitter, Manager};

use crate::{
    cc::telemetry::forward_log_event,
    models::FunctionStatus,
    stores::{FunctionStore, RemotePtyStore},
};

/// Top-level `sessionId` for CC log ingest: canonical `ccpty:…` when this local PTY is admin-bound,
/// otherwise the same local id callers store on `FunctionStatus` (OpenVPN, Boundary, etc.).
fn telemetry_session_id_for_cc_ingest(
    local_session_id: Option<&str>,
    remote_pty_store: &RemotePtyStore,
) -> Option<String> {
    let local = local_session_id?;
    Some(
        remote_pty_store
            .remote_session_for_local(local)
            .unwrap_or_else(|| local.to_string()),
    )
}

pub fn upsert_function_status(
    app: &AppHandle,
    function_store: &FunctionStore,
    function_id: &str,
    state: &str,
    message: &str,
    session_id: Option<String>,
) -> Result<(), String> {
    let status = FunctionStatus {
        function_id: function_id.to_string(),
        state: state.to_string(),
        message: message.to_string(),
        boundary_token_source: None,
        session_id,
        updated_at_ms: now_ms(),
    };

    function_store.publish(status, |status| {
        let _ = app.emit("function-status", status.clone());
        let telemetry_session_id = telemetry_session_id_for_cc_ingest(
            status.session_id.as_deref(),
            &*app.state::<RemotePtyStore>(),
        );
        forward_log_event(
            app,
            "function-status",
            telemetry_session_id.as_deref(),
            Some(&status.function_id),
            &status,
        );
    })
}

pub fn upsert_boundary_status(
    app: &AppHandle,
    function_store: &FunctionStore,
    state: &str,
    message: &str,
    boundary_token_source: Option<&str>,
    session_id: Option<String>,
    boundary_auth_function_id: &str,
) -> Result<(), String> {
    let status = FunctionStatus {
        function_id: boundary_auth_function_id.to_string(),
        state: state.to_string(),
        message: message.to_string(),
        boundary_token_source: boundary_token_source.map(|s| s.to_string()),
        session_id,
        updated_at_ms: now_ms(),
    };

    function_store.publish(status, |status| {
        let _ = app.emit("function-status", status.clone());
        let telemetry_session_id = telemetry_session_id_for_cc_ingest(
            status.session_id.as_deref(),
            &*app.state::<RemotePtyStore>(),
        );
        forward_log_event(
            app,
            "function-status",
            telemetry_session_id.as_deref(),
            Some(&status.function_id),
            &status,
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stores::RemotePtyStore;

    #[test]
    fn desktop_store_and_payloads_use_the_shared_core_types() {
        let store = FunctionStore::default();
        let shared: &scp_core::runtime::FunctionStore = &store;
        let snapshot = shared.snapshot().unwrap();
        assert_eq!(
            snapshot
                .iter()
                .map(|status| status.function_id.as_str())
                .collect::<Vec<_>>(),
            [
                crate::BOUNDARY_AUTH_FUNCTION_ID,
                crate::CADDY_FUNCTION_ID,
                crate::OPENVPN_FUNCTION_ID
            ],
        );
        let payload = crate::models::PtyExitPayload {
            id: "pty-test".into(),
            exit_code: None,
            reason: "wait_error".into(),
        };
        let shared_payload: scp_core::runtime::PtyExitPayload = payload;
        assert_eq!(
            serde_json::to_value(shared_payload).unwrap(),
            serde_json::json!({"id":"pty-test","exitCode":null,"reason":"wait_error"})
        );
    }

    #[test]
    fn telemetry_session_uses_cc_id_when_bound() {
        let store = RemotePtyStore::default();
        store
            .bind_session("ccpty:dev-1:pty-2", "pty-2")
            .expect("bind");
        assert_eq!(
            telemetry_session_id_for_cc_ingest(Some("pty-2"), &store).as_deref(),
            Some("ccpty:dev-1:pty-2")
        );
    }

    #[test]
    fn telemetry_session_falls_back_to_local_when_unbound() {
        let store = RemotePtyStore::default();
        assert_eq!(
            telemetry_session_id_for_cc_ingest(Some("pty-9"), &store).as_deref(),
            Some("pty-9")
        );
    }

    #[test]
    fn telemetry_session_none_when_status_has_no_session() {
        let store = RemotePtyStore::default();
        assert_eq!(telemetry_session_id_for_cc_ingest(None, &store), None);
    }
}
