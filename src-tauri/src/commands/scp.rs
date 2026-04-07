use tauri::{AppHandle, State};

use crate::models::ScpTransferState;
use crate::services::{
    attach_scp_transfer_session, create_scp_transfer_entry, now_ms,
    open_pty_with_command_builder, scp_download_service, scp_upload_service,
    start_scp_download_service, start_scp_upload_service, update_scp_transfer_entry,
};
use crate::stores::{ScpTransferStore, SessionStore};
use super::update_functions_for_session_exit;

pub(crate) fn scp_upload_action(
    app: AppHandle,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    let transfer_id = create_scp_transfer_entry(
        &transfer_store,
        now_ms,
        "Upload",
        "Uploading…",
        Some(local_path.clone()),
        Some(remote_path.clone()),
    )?;
    match scp_upload_service(&app, server_id, local_path, remote_path, recursive) {
        Ok(message) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Success,
                message.clone(),
                Some(100),
                now_ms,
            )?;
            Ok(message)
        }
        Err(err) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Error,
                err.clone(),
                None,
                now_ms,
            )?;
            Err(err)
        }
    }
}

#[tauri::command]
pub fn scp_upload(
    app: AppHandle,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    scp_upload_action(
        app,
        server_id,
        local_path,
        remote_path,
        recursive,
        transfer_store,
    )
}

pub(crate) fn start_scp_upload_action(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    let transfer_id = create_scp_transfer_entry(
        &transfer_store,
        now_ms,
        "Upload",
        "Uploading…",
        Some(local_path.clone()),
        Some(remote_path.clone()),
    )?;
    let result = start_scp_upload_service(
        &app,
        &session_store,
        server_id,
        local_path,
        remote_path,
        recursive,
        open_pty_with_command_builder,
        update_functions_for_session_exit,
    );
    match result {
        Ok(session_id) => {
            attach_scp_transfer_session(&transfer_store, &transfer_id, &session_id)?;
            Ok(session_id)
        }
        Err(err) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Error,
                err.clone(),
                None,
                now_ms,
            )?;
            Err(err)
        }
    }
}

#[tauri::command]
pub fn start_scp_upload(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    server_id: String,
    local_path: String,
    remote_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    start_scp_upload_action(
        app,
        session_store,
        server_id,
        local_path,
        remote_path,
        recursive,
        transfer_store,
    )
}

pub(crate) fn scp_download_action(
    app: AppHandle,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    let transfer_id = create_scp_transfer_entry(
        &transfer_store,
        now_ms,
        "Download",
        "Downloading…",
        Some(remote_path.clone()),
        Some(local_path.clone()),
    )?;
    match scp_download_service(&app, server_id, remote_path, local_path, recursive) {
        Ok(message) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Success,
                message.clone(),
                Some(100),
                now_ms,
            )?;
            Ok(message)
        }
        Err(err) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Error,
                err.clone(),
                None,
                now_ms,
            )?;
            Err(err)
        }
    }
}

#[tauri::command]
pub fn scp_download(
    app: AppHandle,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    scp_download_action(
        app,
        server_id,
        remote_path,
        local_path,
        recursive,
        transfer_store,
    )
}

pub(crate) fn start_scp_download_action(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    let transfer_id = create_scp_transfer_entry(
        &transfer_store,
        now_ms,
        "Download",
        "Downloading…",
        Some(remote_path.clone()),
        Some(local_path.clone()),
    )?;
    let result = start_scp_download_service(
        &app,
        &session_store,
        server_id,
        remote_path,
        local_path,
        recursive,
        open_pty_with_command_builder,
        update_functions_for_session_exit,
    );
    match result {
        Ok(session_id) => {
            attach_scp_transfer_session(&transfer_store, &transfer_id, &session_id)?;
            Ok(session_id)
        }
        Err(err) => {
            update_scp_transfer_entry(
                &transfer_store,
                &transfer_id,
                ScpTransferState::Error,
                err.clone(),
                None,
                now_ms,
            )?;
            Err(err)
        }
    }
}

#[tauri::command]
pub fn start_scp_download(
    app: AppHandle,
    session_store: State<'_, SessionStore>,
    server_id: String,
    remote_path: String,
    local_path: String,
    recursive: Option<bool>,
    transfer_store: State<'_, ScpTransferStore>,
) -> Result<String, String> {
    start_scp_download_action(
        app,
        session_store,
        server_id,
        remote_path,
        local_path,
        recursive,
        transfer_store,
    )
}
