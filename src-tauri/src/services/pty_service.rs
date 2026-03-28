use std::{
    io::Read,
    sync::{mpsc::Sender, Arc, Mutex},
    thread,
    time::Duration,
};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    models::{PtyExitPayload, PtyOutputPayload},
    services::{handle_scp_transfer_output, lock_err, complete_scp_transfer_session},
    stores::{FunctionStore, PtySession, ScpTransferStore, SessionStore},
};

pub fn emit_pty_output_line(app: &AppHandle, session_id: &str, line: impl AsRef<str>) {
    let data = format!("{}\r\n", line.as_ref());
    let _ = app.emit(
        "pty-output",
        PtyOutputPayload {
            id: session_id.to_string(),
            data: data.clone(),
        },
    );
    let session_state = app.state::<SessionStore>();
    let _ = broadcast_watchers(&session_state, session_id, &data);
    if let Some(transfer_state) = app.try_state::<ScpTransferStore>() {
        let _ = handle_scp_transfer_output(&transfer_state, session_id, &data, crate::services::now_ms);
    }
}

pub fn passive_pty_command() -> CommandBuilder {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = CommandBuilder::new("powershell.exe");
        cmd.arg("-NoLogo");
        cmd.arg("-NoProfile");
        cmd.arg("-Command");
        cmd.arg("while ($true) { Start-Sleep -Seconds 3600 }");
        cmd
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-c");
        cmd.arg("while true; do sleep 3600; done");
        cmd
    }
}

pub fn next_session_id(store: &SessionStore) -> Result<String, String> {
    let mut next = lock_err(&store.next_id)?;
    let id = *next;
    *next += 1;
    Ok(format!("pty-{id}"))
}

pub fn remove_session(store: &SessionStore, id: &str) -> Result<Option<Arc<PtySession>>, String> {
    {
        let mut watchers = lock_err(&store.watchers)?;
        watchers.remove(id);
    }
    let mut sessions = lock_err(&store.sessions)?;
    Ok(sessions.remove(id))
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn add_watcher(store: &SessionStore, id: &str, sender: Sender<String>) -> Result<(), String> {
    let mut watchers = lock_err(&store.watchers)?;
    watchers.entry(id.to_string()).or_default().push(sender);
    Ok(())
}

pub fn broadcast_watchers(store: &SessionStore, id: &str, data: &str) -> Result<(), String> {
    let mut watchers = lock_err(&store.watchers)?;
    if let Some(list) = watchers.get_mut(id) {
        list.retain(|sender| sender.send(data.to_string()).is_ok());
    }
    Ok(())
}

pub fn emit_pty_exit(app: &AppHandle, payload: PtyExitPayload) {
    if let Some(transfer_state) = app.try_state::<ScpTransferStore>() {
        let _ = complete_scp_transfer_session(
            &transfer_state,
            &payload.id,
            payload.exit_code,
            &payload.reason,
            crate::services::now_ms,
        );
    }
    let _ = app.emit("pty-exit", payload);
}

pub fn open_pty_with_command_builder(
    cols: u16,
    rows: u16,
    cmd: CommandBuilder,
    app: &AppHandle,
    store: &SessionStore,
    on_session_exit: fn(&AppHandle, &FunctionStore, &str, Option<i32>, &str) -> Result<(), String>,
) -> Result<String, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("failed to open pty: {e}"))?;

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("failed to spawn process in pty: {e}"))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("failed to clone pty reader: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("failed to get pty writer: {e}"))?;

    let session_id = next_session_id(store)?;
    let session = Arc::new(PtySession {
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        child: Mutex::new(child),
    });

    {
        let mut sessions = lock_err(&store.sessions)?;
        sessions.insert(session_id.clone(), session);
    }

    let output_session_id = session_id.clone();
    let output_app = app.clone();
    thread::spawn(move || {
        let mut buf = [0_u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = output_app.emit(
                        "pty-output",
                        PtyOutputPayload {
                            id: output_session_id.clone(),
                            data: data.clone(),
                        },
                    );
                    let session_state = output_app.state::<SessionStore>();
                    let _ = broadcast_watchers(&session_state, &output_session_id, &data);
                    if let Some(transfer_state) = output_app.try_state::<ScpTransferStore>() {
                        let _ = handle_scp_transfer_output(
                            &transfer_state,
                            &output_session_id,
                            &data,
                            crate::services::now_ms,
                        );
                    }
                }
                Err(_) => break,
            }
        }
    });

    let monitor_session_id = session_id.clone();
    let monitor_app = app.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(100));

        let session = {
            let state = monitor_app.state::<SessionStore>();
            let lookup = match lock_err(&state.sessions) {
                Ok(sessions) => sessions.get(&monitor_session_id).cloned(),
                Err(_) => None,
            };
            lookup
        };

        let Some(session) = session else {
            break;
        };

        let exit_payload = {
            let mut child = match lock_err(&session.child) {
                Ok(child) => child,
                Err(_) => break,
            };
            match child.try_wait() {
                Ok(Some(status)) => Some(PtyExitPayload {
                    id: monitor_session_id.clone(),
                    exit_code: i32::try_from(status.exit_code()).ok(),
                    reason: status.to_string(),
                }),
                Ok(None) => None,
                Err(err) => Some(PtyExitPayload {
                    id: monitor_session_id.clone(),
                    exit_code: None,
                    reason: format!("wait_error: {err}"),
                }),
            }
        };

        if let Some(payload) = exit_payload {
            let removed = {
                let state = monitor_app.state::<SessionStore>();
                remove_session(&state, &monitor_session_id)
                    .ok()
                    .flatten()
                    .is_some()
            };

            if removed {
                emit_pty_exit(&monitor_app, payload.clone());
                if let Some(transfer_state) = monitor_app.try_state::<ScpTransferStore>() {
                    let _ = complete_scp_transfer_session(
                        &transfer_state,
                        &monitor_session_id,
                        payload.exit_code,
                        &payload.reason,
                        crate::services::now_ms,
                    );
                }
                let function_state = monitor_app.state::<FunctionStore>();
                let _ = on_session_exit(
                    &monitor_app,
                    &function_state,
                    &monitor_session_id,
                    payload.exit_code,
                    &payload.reason,
                );
            }
            break;
        }
    });

    Ok(session_id)
}

pub fn write_pty_internal(id: &str, data: &str, store: &SessionStore) -> Result<(), String> {
    let session = {
        let sessions = lock_err(&store.sessions)?;
        sessions
            .get(id)
            .cloned()
            .ok_or_else(|| format!("session not found: {id}"))?
    };

    let mut writer = lock_err(&session.writer)?;
    writer
        .write_all(data.as_bytes())
        .map_err(|e| format!("failed to write pty input: {e}"))?;
    writer
        .flush()
        .map_err(|e| format!("failed to flush pty input: {e}"))?;

    Ok(())
}

pub fn close_pty_internal(id: &str, store: &SessionStore) -> Result<Option<PtyExitPayload>, String> {
    let session = remove_session(store, id)?;
    if let Some(session) = session {
        let mut child = lock_err(&session.child)?;
        let _ = child.kill();
        let exit_status = child
            .wait()
            .map_err(|e| format!("failed to wait child: {e}"))?;
        return Ok(Some(PtyExitPayload {
            id: id.to_string(),
            exit_code: i32::try_from(exit_status.exit_code()).ok(),
            reason: "closed_by_command".to_string(),
        }));
    }
    Ok(None)
}
