//! Shared status state and typed event payloads. Process control and event delivery belong to hosts.

use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PtyOutputPayload {
    pub id: String,
    pub data: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PtyExitPayload {
    pub id: String,
    pub exit_code: Option<i32>,
    pub reason: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FunctionStatus {
    pub function_id: String,
    pub state: String,
    pub message: String,
    pub boundary_token_source: Option<String>,
    pub session_id: Option<String>,
    pub updated_at_ms: u64,
}

/// The host's verified Boundary token summary, supplied without holding runtime locks.
pub struct BoundaryTokenStatus {
    pub message: String,
    pub source: String,
}

pub struct FunctionStore {
    // Public during desktop migration: existing callers still access this same mutex.
    pub statuses: Mutex<HashMap<String, FunctionStatus>>,
}

impl Default for FunctionStore {
    fn default() -> Self {
        Self {
            statuses: Mutex::new(default_function_statuses(
                "openvpn-connect",
                "caddy-run",
                "boundary-auth",
            )),
        }
    }
}

impl FunctionStore {
    /// An owned, deterministic snapshot of this process's state only.
    pub fn snapshot(&self) -> Result<Vec<FunctionStatus>, String> {
        let mut values: Vec<_> = lock_err(&self.statuses)?.values().cloned().collect();
        values.sort_by(|a, b| a.function_id.cmp(&b.function_id));
        Ok(values)
    }

    /// Commit before notifying the host. No lock is held across event delivery or telemetry.
    pub fn publish(
        &self,
        status: FunctionStatus,
        sink: impl FnOnce(FunctionStatus),
    ) -> Result<(), String> {
        lock_err(&self.statuses)?.insert(status.function_id.clone(), status.clone());
        sink(status);
        Ok(())
    }

    /// Apply an exit to statuses still attached to that session. The host performs
    /// side effects and delivers the returned status events after this method returns.
    pub fn finish_session(
        &self,
        exit: &PtyExitPayload,
        boundary_function_id: &str,
        timestamp_ms: u64,
        lookup_boundary_token: impl FnOnce() -> Option<BoundaryTokenStatus>,
    ) -> Result<Vec<FunctionStatus>, String> {
        let needs_boundary = lock_err(&self.statuses)?.values().any(|status| {
            status.function_id == boundary_function_id
                && status.session_id.as_deref() == Some(&exit.id)
        });
        let token = if needs_boundary {
            lookup_boundary_token()
        } else {
            None
        };
        let mut changed = Vec::new();
        for status in lock_err(&self.statuses)?.values_mut() {
            // A host lookup may have yielded to a restart. Never replace the new session's status.
            if status.session_id.as_deref() != Some(&exit.id) {
                continue;
            }
            if status.function_id == boundary_function_id {
                complete_boundary(status, exit, token.as_ref());
            } else {
                status.state = if exit.exit_code == Some(0) {
                    "stopped"
                } else {
                    "error"
                }
                .into();
                status.message = format!(
                    "PTY exited (code={}, reason={})",
                    exit.exit_code
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                    exit.reason
                );
                // Retain sessionId so completed tunnel/SSH logs remain queryable.
            }
            status.updated_at_ms = timestamp_ms;
            changed.push(status.clone());
        }
        Ok(changed)
    }
}

fn complete_boundary(
    status: &mut FunctionStatus,
    exit: &PtyExitPayload,
    token: Option<&BoundaryTokenStatus>,
) {
    status.session_id = None;
    status.boundary_token_source = token.map(|summary| summary.source.clone());
    if let Some(summary) = token {
        status.state = "authenticated".into();
        status.message = summary.message.clone();
    } else if exit.exit_code == Some(0) {
        status.state = "authenticated".into();
        status.message = "Login completed, but the saved Boundary token could not be verified yet. Refresh Status or retry login.".into();
    } else {
        status.state = "unauthenticated".into();
        status.message = format!(
            "Not logged in or login interrupted (exit {}). {}",
            exit.exit_code
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
            short_reason(&exit.reason)
        );
    }
}

fn short_reason(reason: &str) -> String {
    if reason.len() <= 80 {
        return reason.into();
    }
    let mut end = 80;
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &reason[..end])
}

pub fn lock_err<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, String> {
    mutex.lock().map_err(|_| "mutex poisoned".to_string())
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn default_function_statuses(
    openvpn_function_id: &str,
    caddy_function_id: &str,
    boundary_auth_function_id: &str,
) -> HashMap<String, FunctionStatus> {
    let mut statuses = HashMap::new();
    for (id, message) in [
        (openvpn_function_id, "Ready"),
        (caddy_function_id, "Ready"),
        (boundary_auth_function_id, "Not authenticated"),
    ] {
        statuses.insert(
            id.into(),
            FunctionStatus {
                function_id: id.into(),
                state: "idle".into(),
                message: message.into(),
                boundary_token_source: None,
                session_id: None,
                updated_at_ms: now_ms(),
            },
        );
    }
    statuses
}
