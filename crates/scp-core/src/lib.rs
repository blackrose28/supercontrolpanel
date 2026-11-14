//! Headless Super Control Panel core runtime, profile locking, secrets, and backup engine.
pub mod backup;
pub mod boundary_ops;
pub mod caddy_ops;
pub mod config_repo;
pub mod error;
pub mod events;
pub mod file_paths;
pub mod gate;
pub mod models;
pub mod openvpn_ops;
pub mod profile;
pub mod runtime;
pub mod secrets;
pub mod sftp_ops;
pub mod ssh_ops;
pub mod supervisor;
pub mod transfer_store;
pub mod tty;

use error::CoreError;
use serde_json::{json, Value};

fn envelope(runtime_id: Option<&str>, ok: bool, data: Value, error: Value) -> Value {
    json!({
        "schemaVersion": 1,
        "ok": ok,
        "runtimeId": runtime_id,
        "data": data,
        "error": error
    })
}

pub fn success(data: Value) -> Value {
    envelope(None, true, data, Value::Null)
}

pub fn success_with_runtime(runtime_id: &str, data: Value) -> Value {
    envelope(Some(runtime_id), true, data, Value::Null)
}

pub fn invalid_arguments() -> Value {
    let err = CoreError::invalid_arguments(
        "invalid_arguments",
        "Unsupported command or arguments. Run scpctl --help.",
    );
    error_envelope(None, &err)
}

pub fn error_envelope(runtime_id: Option<&str>, error: &CoreError) -> Value {
    envelope(runtime_id, false, Value::Null, json!(error))
}
