use scp_core::runtime::{
    BoundaryTokenStatus, FunctionStatus, FunctionStore, PtyExitPayload, PtyOutputPayload,
};
use serde_json::json;
use std::{collections::HashMap, sync::Mutex};

fn running(id: &str, session: &str) -> FunctionStatus {
    FunctionStatus {
        function_id: id.into(),
        state: "running".into(),
        message: "Ready".into(),
        boundary_token_source: None,
        session_id: Some(session.into()),
        updated_at_ms: 1,
    }
}

fn store(statuses: Vec<FunctionStatus>) -> FunctionStore {
    FunctionStore {
        statuses: Mutex::new(
            statuses
                .into_iter()
                .map(|s| (s.function_id.clone(), s))
                .collect(),
        ),
    }
}

fn exit(code: Option<i32>, reason: &str) -> PtyExitPayload {
    PtyExitPayload {
        id: "session-1".into(),
        exit_code: code,
        reason: reason.into(),
    }
}

#[test]
fn wire_payloads_keep_camel_case_fields_and_explicit_nulls() {
    assert_eq!(
        serde_json::to_value(running("server-tunnel-one", "session-1")).unwrap(),
        json!({
            "functionId":"server-tunnel-one", "state":"running", "message":"Ready",
            "boundaryTokenSource":null, "sessionId":"session-1", "updatedAtMs":1
        })
    );
    assert_eq!(
        serde_json::to_value(exit(None, "wait_error")).unwrap(),
        json!({"id":"session-1","exitCode":null,"reason":"wait_error"})
    );
    assert_eq!(
        serde_json::to_value(PtyOutputPayload {
            id: "session-1".into(),
            data: "line\r\n".into()
        })
        .unwrap(),
        json!({"id":"session-1","data":"line\r\n"})
    );
}

#[test]
fn default_store_has_only_the_three_existing_idle_functions() {
    let statuses = FunctionStore::default().snapshot().unwrap();
    assert_eq!(
        statuses
            .iter()
            .map(|s| s.function_id.as_str())
            .collect::<Vec<_>>(),
        ["boundary-auth", "caddy-run", "openvpn-connect"]
    );
    for status in statuses {
        assert_eq!(status.state, "idle");
        assert!(status.session_id.is_none());
        assert!(status.boundary_token_source.is_none());
        assert!(status.updated_at_ms > 0);
        assert_eq!(
            status.message,
            if status.function_id == "boundary-auth" {
                "Not authenticated"
            } else {
                "Ready"
            }
        );
    }
}

#[test]
fn publish_commits_before_calling_host_and_releases_the_mutex() {
    let store = store(vec![]);
    let mut emitted = Vec::new();
    store
        .publish(running("server-ssh-one", "session-1"), |status| {
            let statuses = store
                .statuses
                .try_lock()
                .expect("host must run outside store lock");
            assert_eq!(statuses[&status.function_id].session_id, status.session_id);
            emitted.push(serde_json::to_value(status).unwrap());
        })
        .unwrap();
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0]["sessionId"], "session-1");
}

#[test]
fn regular_exit_updates_every_matching_function_and_retains_session_history() {
    for (code, state, code_text) in [
        (Some(0), "stopped", "0"),
        (Some(17), "error", "17"),
        (None, "error", "unknown"),
    ] {
        let store = store(vec![
            running("server-tunnel-one", "session-1"),
            running("openvpn-connect", "session-1"),
            running("server-ssh-two", "other"),
        ]);
        let changed = store
            .finish_session(&exit(code, "finished"), "boundary-auth", 42, || {
                panic!("not Boundary")
            })
            .unwrap();
        assert_eq!(changed.len(), 2);
        for status in changed {
            assert_eq!(status.state, state);
            assert_eq!(
                status.message,
                format!("PTY exited (code={code_text}, reason=finished)")
            );
            assert_eq!(status.session_id.as_deref(), Some("session-1"));
            assert_eq!(status.updated_at_ms, 42);
        }
        assert_eq!(
            store.statuses.lock().unwrap()["server-ssh-two"].updated_at_ms,
            1
        );
    }
}

#[test]
fn boundary_exit_uses_verified_host_token_and_clears_the_auth_session() {
    for code in [Some(0), Some(1), None] {
        for source in ["app", "cli"] {
            let store = store(vec![running("boundary-auth", "session-1")]);
            let changed = store
                .finish_session(&exit(code, "done"), "boundary-auth", 42, || {
                    assert!(
                        store.statuses.try_lock().is_ok(),
                        "token lookup must not hold the status lock"
                    );
                    Some(BoundaryTokenStatus {
                        message: "Token valid".into(),
                        source: source.into(),
                    })
                })
                .unwrap();
            let status = &changed[0];
            assert_eq!(status.state, "authenticated");
            assert_eq!(status.message, "Token valid");
            assert_eq!(status.boundary_token_source.as_deref(), Some(source));
            assert!(status.session_id.is_none());
            assert_eq!(status.updated_at_ms, 42);
        }
    }
}

#[test]
fn boundary_exit_without_token_preserves_success_and_failure_messages() {
    for (code, state) in [
        (Some(0), "authenticated"),
        (Some(1), "unauthenticated"),
        (None, "unauthenticated"),
    ] {
        let store = store(vec![running("boundary-auth", "session-1")]);
        let changed = store
            .finish_session(&exit(code, "interrupted"), "boundary-auth", 42, || None)
            .unwrap();
        let status = &changed[0];
        assert_eq!(status.state, state);
        assert!(status.boundary_token_source.is_none());
        assert!(status.session_id.is_none());
        assert_eq!(status.message, match code {
            Some(0) => "Login completed, but the saved Boundary token could not be verified yet. Refresh Status or retry login.",
            Some(1) => "Not logged in or login interrupted (exit 1). interrupted",
            _ => "Not logged in or login interrupted (exit ?). interrupted",
        });
    }
}

#[test]
fn boundary_error_reason_truncates_at_a_valid_utf8_boundary() {
    let store = store(vec![running("boundary-auth", "session-1")]);
    let reason = format!("{}é{}", "a".repeat(79), "x".repeat(10));
    let changed = store
        .finish_session(&exit(Some(1), &reason), "boundary-auth", 42, || None)
        .unwrap();
    assert!(changed[0]
        .message
        .ends_with(&format!("{}…", "a".repeat(79))));
}

#[test]
fn late_exit_does_not_overwrite_a_replacement_boundary_session() {
    let store = store(vec![running("boundary-auth", "session-1")]);
    let changed = store
        .finish_session(&exit(Some(0), "finished"), "boundary-auth", 42, || {
            store.statuses.try_lock().unwrap().insert(
                "boundary-auth".into(),
                running("boundary-auth", "replacement"),
            );
            Some(BoundaryTokenStatus {
                message: "Old token".into(),
                source: "app".into(),
            })
        })
        .unwrap();
    assert!(changed.is_empty());
    assert_eq!(
        store.statuses.lock().unwrap()["boundary-auth"]
            .session_id
            .as_deref(),
        Some("replacement")
    );
    assert!(store
        .finish_session(
            &exit(None, "unknown session"),
            "boundary-auth",
            43,
            || panic!("no matching session")
        )
        .unwrap()
        .is_empty());
}

#[test]
fn poison_errors_do_not_emit_or_run_host_callbacks() {
    let store = FunctionStore {
        statuses: Mutex::new(HashMap::new()),
    };
    let _ = std::panic::catch_unwind(|| {
        let _guard = store.statuses.lock().unwrap();
        panic!("poison fixture");
    });
    assert!(store
        .publish(running("test", "session-1"), |_| panic!("must not emit"))
        .unwrap_err()
        .contains("mutex poisoned"));
    assert!(store
        .finish_session(&exit(None, "failed"), "boundary-auth", 42, || panic!(
            "must not inspect token"
        ))
        .is_err());
    assert!(store.snapshot().is_err());
}

#[test]
fn shared_lock_helper_preserves_the_desktop_error_contract() {
    let mutex = Mutex::new(7);
    assert_eq!(*scp_core::runtime::lock_err(&mutex).unwrap(), 7);
    let _ = std::panic::catch_unwind(|| {
        let _guard = mutex.lock().unwrap();
        panic!("poison fixture");
    });
    assert_eq!(
        scp_core::runtime::lock_err(&mutex).unwrap_err(),
        "mutex poisoned"
    );
}
