use tauri::{AppHandle, State};

use crate::models::{RemoteBrowseEntry, RemoteBrowseNodeInfo, RemoteBrowseSession};
use crate::services::{
    close_remote_browser as close_remote_browser_service,
    close_remote_browser_sessions_for_server as close_remote_browser_sessions_for_server_service,
    create_remote_browser_directory as create_remote_browser_directory_service,
    delete_remote_browser_path as delete_remote_browser_path_service,
    list_remote_browser_children as list_remote_browser_children_service,
    open_remote_browser as open_remote_browser_service,
    stat_remote_browser_path as stat_remote_browser_path_service,
};
#[cfg(target_os = "windows")]
use crate::CREATE_NO_WINDOW;
use crate::server_tunnel_function_id;
use crate::stores::{FunctionStore, RemoteBrowserStore};

pub(crate) fn close_remote_browser_sessions_for_server(
    server_id: &str,
    browser_store: &RemoteBrowserStore,
) -> Result<usize, String> {
    close_remote_browser_sessions_for_server_service(server_id, browser_store)
}

pub(crate) fn open_remote_browser_action(
    app: AppHandle,
    server_id: String,
    browser_store: State<'_, RemoteBrowserStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<RemoteBrowseSession, String> {
    #[cfg(target_os = "windows")]
    {
        open_remote_browser_service(
            &app,
            server_id,
            &browser_store,
            &function_store,
            server_tunnel_function_id,
            CREATE_NO_WINDOW,
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        open_remote_browser_service(
            &app,
            server_id,
            &browser_store,
            &function_store,
            server_tunnel_function_id,
        )
    }
}

#[tauri::command]
pub fn open_remote_browser(
    app: AppHandle,
    server_id: String,
    browser_store: State<'_, RemoteBrowserStore>,
    function_store: State<'_, FunctionStore>,
) -> Result<RemoteBrowseSession, String> {
    open_remote_browser_action(app, server_id, browser_store, function_store)
}

pub(crate) fn close_remote_browser_action(
    browser_id: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    close_remote_browser_service(browser_id, &browser_store)
}

#[tauri::command]
pub fn close_remote_browser(
    browser_id: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    close_remote_browser_action(browser_id, browser_store)
}

pub(crate) fn list_remote_browser_children_action(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<RemoteBrowseEntry>, String> {
    list_remote_browser_children_service(browser_id, path, &browser_store)
}

#[tauri::command]
pub fn list_remote_browser_children(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<Vec<RemoteBrowseEntry>, String> {
    list_remote_browser_children_action(browser_id, path, browser_store)
}

pub(crate) fn stat_remote_browser_path_action(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<RemoteBrowseNodeInfo, String> {
    stat_remote_browser_path_service(browser_id, path, &browser_store)
}

#[tauri::command]
pub fn stat_remote_browser_path(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<RemoteBrowseNodeInfo, String> {
    stat_remote_browser_path_action(browser_id, path, browser_store)
}

pub(crate) fn create_remote_browser_directory_action(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    create_remote_browser_directory_service(browser_id, path, &browser_store)
}

#[tauri::command]
pub fn create_remote_browser_directory(
    browser_id: String,
    path: String,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    create_remote_browser_directory_action(browser_id, path, browser_store)
}

pub(crate) fn delete_remote_browser_path_action(
    browser_id: String,
    path: String,
    recursive: Option<bool>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    delete_remote_browser_path_service(browser_id, path, recursive, &browser_store)
}

#[tauri::command]
pub fn delete_remote_browser_path(
    browser_id: String,
    path: String,
    recursive: Option<bool>,
    browser_store: State<'_, RemoteBrowserStore>,
) -> Result<(), String> {
    delete_remote_browser_path_action(browser_id, path, recursive, browser_store)
}
