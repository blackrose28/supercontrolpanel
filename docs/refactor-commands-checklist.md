# Refactor Checklist: Split main.rs into command modules

> **Goal**: Extract `#[tauri::command]` functions from `src-tauri/src/main.rs` (2615 lines) into `src-tauri/src/commands/` sub-modules, leaving main.rs as a slim entry point (~100-150 lines).

## Pre-requisites (done)

- [x] Make constants `pub(crate)` in main.rs
- [x] Place `scp_debug!` macro BEFORE `mod commands;` in main.rs
- [x] Verified: macro is accessible from sub-modules via `scp_debug!(...)` (no `crate::` prefix)

## Step 1: Skeleton — `commands/mod.rs` + `commands/mcp.rs` ✅

- [x] Create `commands/mod.rs` with `pub mod mcp; pub use mcp::*;`
- [x] Create `commands/mcp.rs` — extract MCP commands + `get_config_paths_internal`
- [x] Add `mod commands;` and `use commands::*` in main.rs
- [x] Remove extracted functions from main.rs
- [x] `cargo check` passes

## Step 2: Extract `commands/caddy.rs` ✅

Functions to move:
- `start_caddy_action` + `start_caddy_run` #[tauri::command]
- `stop_caddy_action` + `stop_caddy_run` #[tauri::command]

Imports needed:
- `crate::models::StartFunctionResult`
- `crate::services::{close_pty_internal, emit_pty_exit, lock_err, open_pty_with_command_builder, start_caddy_run as _, stop_caddy_run as _, upsert_function_status as _}`
- `crate::stores::{FunctionStore, SessionStore}`
- `crate::CADDY_FUNCTION_ID`
- `crate::upsert_function_status` (still in main.rs for now)
- `crate::update_functions_for_session_exit` (still in main.rs for now)

Actions:
- [ ] Create `commands/caddy.rs`
- [ ] Add `pub mod caddy; pub use caddy::*;` to `commands/mod.rs`
- [ ] Add `use commands::{start_caddy_run, stop_caddy_run}` in main.rs
- [ ] Remove extracted functions from main.rs
- [ ] `cargo check` passes

## Step 3: Extract `commands/remote_browser.rs` + `commands/scp.rs` ✅

### remote_browser.rs — Functions to move:
- `close_remote_browser_sessions_for_server` (used by server.rs later)
- `open_remote_browser_action` + `open_remote_browser` #[tauri::command]
- `close_remote_browser_action` + `close_remote_browser` #[tauri::command]
- `list_remote_browser_children_action` + `list_remote_browser_children` #[tauri::command]
- `stat_remote_browser_path_action` + `stat_remote_browser_path` #[tauri::command]
- `create_remote_browser_directory_action` + `create_remote_browser_directory` #[tauri::command]
- `delete_remote_browser_path_action` + `delete_remote_browser_path` #[tauri::command]

Imports needed:
- `crate::models::{RemoteBrowseEntry, RemoteBrowseNodeInfo, RemoteBrowseSession}`
- `crate::services::{close_remote_browser as _, close_remote_browser_sessions_for_server as _, ...}`
- `crate::stores::{FunctionStore, RemoteBrowserStore}`
- `crate::{server_tunnel_function_id, CREATE_NO_WINDOW}`

### scp.rs — Functions to move:
- `scp_upload_action` + `scp_upload` #[tauri::command]
- `start_scp_upload_action` + `start_scp_upload` #[tauri::command]
- `scp_download_action` + `scp_download` #[tauri::command]
- `start_scp_download_action` + `start_scp_download` #[tauri::command]

Imports needed:
- `crate::models::ScpTransferState`
- `crate::services::{attach_scp_transfer_session, create_scp_transfer_entry, emit_pty_exit, now_ms, open_pty_with_command_builder, scp_download_service, scp_upload_service, start_scp_download_service, start_scp_upload_service, update_scp_transfer_entry}`
- `crate::stores::{ScpTransferStore, SessionStore}`
- `crate::update_functions_for_session_exit` (still in main.rs for now)

Actions:
- [ ] Create `commands/remote_browser.rs`
- [ ] Create `commands/scp.rs`
- [ ] Add both to `commands/mod.rs`
- [ ] Add `use commands::{...}` in main.rs
- [ ] Remove extracted functions from main.rs
- [ ] `cargo check` passes

## Step 4: Extract `commands/config.rs` ✅

Functions to move:
- `load_app_config_from_disk`
- `save_app_config_to_disk`
- `save_local_app_settings`
- `get_app_config_action` + `get_app_config` #[tauri::command]
- ~~`get_config_paths_internal`~~ (already in mcp.rs)
- ~~`get_config_paths`~~ (keep in main.rs, delegates to commands)
- `get_sync_settings` #[tauri::command]
- `save_sync_settings_action` + `save_sync_settings` #[tauri::command]
- `save_app_config_action` + `save_app_config` #[tauri::command]
- `detect_openvpn_executable` #[tauri::command]
- `sync_config_pull_action` + `sync_config_pull` #[tauri::command]
- `sync_config_push_action` + `sync_config_push` #[tauri::command]
- `config_git_pull_action` + `config_git_pull` #[tauri::command]
- `config_git_push_action` + `config_git_push` #[tauri::command]
- Helpers: `config_git_computer_name`, `run_git`

Imports needed:
- `std::{env, fs, path::PathBuf, process::Command}`
- `crate::models::{AppConfig, AppConfigForDisk}`
- `crate::services::{config_file_path, default_app_config, detect_openvpn_executable_path, keyring_get_optional, keyring_set_or_delete, load_sync_settings_from_disk, save_sync_settings_to_disk, servers_file_path, ConfigPaths, SyncSettings, SyncSettingsPatch, KEYRING_*}`
- `crate::stores::FunctionStore`
- `crate::mcp::{McpRuntimeState, sync_mcp_runtime_from_settings}`
- `crate::{CADDY_FUNCTION_ID, CONFIG_FILE_NAME, OPENVPN_FUNCTION_ID, SERVERS_FILE_NAME, SYNC_SETTINGS_FILE_NAME, BOUNDARY_AUTH_FUNCTION_ID, lock_err, now_ms}`

Note: `load_app_config_from_disk` and `save_app_config_to_disk` must be `pub(crate)` — used by openvpn.rs, pty.rs, server.rs, boundary.rs.
Note: `save_sync_settings_action`, `config_git_pull_action`, `config_git_push_action`, `sync_config_pull_action`, `sync_config_push_action`, `get_app_config_action`, `save_app_config_action` must be `pub(crate)` — used by `mcp/tools.rs`.

Actions:
- [ ] Create `commands/config.rs`
- [ ] Add to `commands/mod.rs`
- [ ] Add `use commands::{...}` in main.rs
- [ ] Remove extracted functions from main.rs
- [ ] `cargo check` passes

## Step 5: Extract `commands/server.rs` ✅

Functions to move:
- `list_servers` #[tauri::command]
- `upsert_server_action` + `upsert_server` #[tauri::command]
- `delete_server_action` + `delete_server` #[tauri::command]
- `search_boundary_targets` #[tauri::command]
- `start_server_tunnel_action` + `start_server_tunnel` #[tauri::command]
- `stop_server_tunnel_action` + `stop_server_tunnel` #[tauri::command]
- `start_server_ssh_action` + `start_server_ssh` #[tauri::command]
- `stop_server_ssh_action` + `stop_server_ssh` #[tauri::command]
- `start_server_rdp` #[tauri::command]
- `close_remote_browser_sessions_for_server` (delegates to `commands::remote_browser`)

Imports needed:
- `crate::models::{BoundaryTargetSummary, StartFunctionResult}`
- `crate::server_mgmt::{load_servers, new_server_id, save_servers, Server}`
- `crate::services::{...}`
- `crate::stores::{FunctionStore, RemoteBrowserStore, SessionStore}`
- `crate::commands::remote_browser::close_remote_browser_sessions_for_server`
- `crate::{server_tunnel_function_id, server_ssh_function_id, CREATE_NO_WINDOW}`

Actions:
- [ ] Create `commands/server.rs`
- [ ] Add to `commands/mod.rs`
- [ ] Add `use commands::{...}` in main.rs
- [ ] Remove extracted functions from main.rs
- [ ] `cargo check` passes

## Step 6: Extract shared helpers to `commands/mod.rs` ✅

Move these glue functions from main.rs to `commands/mod.rs` so they're available to all sub-modules:

- `upsert_function_status`
- `update_functions_for_session_exit`
- `upsert_boundary_status`
- `boundary_token_summary`

These reference `handle_openvpn_terminal_state` which is still in main.rs — pass it as a callback or keep a reference.

**Tricky part**: `update_functions_for_session_exit` calls `handle_openvpn_terminal_state` (openvpn logic). Options:
1. Keep `handle_openvpn_terminal_state` in main.rs, pass as fn pointer
2. Move it to `commands/mod.rs` too (it uses `load_app_config_from_disk` which will be in config.rs)
3. Use a trait/dynamic dispatch

Implemented approach: keep the shared status/session-exit glue in `commands/mod.rs`, and place the OpenVPN reconnect helpers with the rest of the OpenVPN orchestration in `commands/openvpn.rs`.

Actions:
- [x] Move `upsert_function_status`, `update_functions_for_session_exit`, `upsert_boundary_status`, `boundary_token_summary` to `commands/mod.rs`
- [x] Keep `handle_openvpn_terminal_state` and reconnect helpers (`compute_openvpn_reconnect_delay_secs`, `mark_openvpn_connected`, `reset_openvpn_reconnect`, `prepare_openvpn_manual_start`) in `commands/openvpn.rs`
- [x] Re-export command modules via `commands/mod.rs`
- [x] Update main.rs to use `commands::*` / `crate::commands::*`
- [x] Update command modules to import shared glue via `super::` where applicable
- [x] `cargo check` passes

## Step 7: Extract `commands/pty.rs` ✅

Functions to move:
- `open_pty_internal`
- `adapt_command_for_shell`
- `default_shell_command`
- `open_pty` #[tauri::command]
- `write_pty` #[tauri::command]
- `resize_pty` #[tauri::command]
- `close_pty` #[tauri::command]
- `list_function_statuses` #[tauri::command]

Imports needed:
- `portable_pty::{CommandBuilder, PtySize}`
- `crate::models::{FunctionStatus, PtyExitPayload}`
- `crate::services::{...}`
- `crate::stores::{FunctionStore, SessionStore}`
- `super::{update_functions_for_session_exit, upsert_function_status, boundary_token_summary}` (from commands/mod.rs)
- `crate::commands::config::load_app_config_from_disk`
- `crate::{BOUNDARY_AUTH_FUNCTION_ID, OPENVPN_FUNCTION_ID}`

Actions:
- [x] Create `commands/pty.rs`
- [x] Add to `commands/mod.rs`
- [x] Add `use commands::{...}` in main.rs
- [x] Remove extracted functions from main.rs
- [x] `cargo check` passes

## Step 8: Extract `commands/openvpn.rs` ✅

Functions to move:
- `compute_openvpn_reconnect_delay_secs` (moved to mod.rs in step 6, can stay there)
- `mark_openvpn_connected` (moved to mod.rs in step 6, can stay there)
- `reset_openvpn_reconnect` (moved to mod.rs in step 6, can stay there)
- `start_openvpn_admin_worker` #[cfg(windows)]
- `run_openvpn_automation` #[cfg(not(windows))]
- `start_openvpn_action` + `start_openvpn_function` #[tauri::command]
- `start_openvpn_manual_action`
- `stop_openvpn_action` + `stop_openvpn_function` #[tauri::command]
- `restart_openvpn_action`

Imports needed:
- `crate::models::{BoundaryConfig, FunctionStatus, PtyExitPayload, StartFunctionResult, AppConfig, AppConfigForDisk}`
- `crate::services::{...}`
- `crate::stores::{...}`
- `super::{update_functions_for_session_exit, upsert_function_status, upsert_boundary_status, handle_openvpn_terminal_state, mark_openvpn_connected, reset_openvpn_reconnect, prepare_openvpn_manual_start}` (from commands/mod.rs)
- `crate::commands::config::{load_app_config_from_disk, save_local_app_settings}`
- `crate::commands::pty::{adapt_command_for_shell, open_pty_internal}`
- `crate::{OPENVPN_FUNCTION_ID, OPENVPN_HELPER_SCRIPT_NAME, CREATE_NO_WINDOW, ...}`

Actions:
- [x] Create `commands/openvpn.rs`
- [x] Add to `commands/mod.rs`
- [x] Add `use commands::{...}` in main.rs
- [x] Remove extracted functions from main.rs
- [x] Update `cc/connection.rs`: `use crate::restart_openvpn_action` → `use crate::commands::openvpn::restart_openvpn_action`
- [x] `cargo check` passes

## Step 9: Extract `commands/boundary.rs` ✅

Functions to move:
- `stop_boundary_automation_session`
- `complete_boundary_automation_session`
- `emit_pty_output_line_owned`
- `boundary_oidc_init_script`
- `run_boundary_oidc_automation`
- `start_boundary_auth_action` + `start_boundary_auth` #[tauri::command]
- `stop_boundary_auth_action` + `stop_boundary_auth` #[tauri::command]

Imports needed:
- `crate::models::{BoundaryConfig, StartFunctionResult}`
- `crate::services::{...}`
- `crate::stores::{...}`
- `super::{update_functions_for_session_exit, upsert_function_status, upsert_boundary_status, boundary_token_summary}` (from commands/mod.rs)
- `crate::commands::config::load_app_config_from_disk`
- `crate::{BOUNDARY_AUTH_FUNCTION_ID, BOUNDARY_OIDC_WINDOW_LABEL}`

Actions:
- [x] Create `commands/boundary.rs`
- [x] Add to `commands/mod.rs`
- [x] Add `use commands::{...}` in main.rs
- [x] Remove extracted functions from main.rs
- [x] `cargo check` passes

## Step 10: Slim down main.rs + update external refs

Actions:
- [x] Remove all remaining extracted code from main.rs
- [x] main.rs should contain only: `#![cfg_attr]`, `mod` declarations, `use` imports, constants (`pub(crate)`), `scp_debug!` macro, `FunctionStore::Default` impl, `is_windows_pid_running` (windows), `main()` function
- [x] Update `mcp/tools.rs` imports: change `crate::xxx_action` → `crate::commands::xxx_action` for all action functions
- [x] Verify all `generate_handler![]` entries are correctly imported
- [x] `cargo check` passes

## Step 11: Final cleanup ✅

- [x] `cargo clippy` passes
- [x] Remove unused imports from main.rs
- [x] Verify `wc -l src/main.rs` is ~100-150 lines
- [x] Verify total `wc -l src/commands/*.rs` + `src/main.rs` ≈ original 2615 lines (no code lost)
- [x] Run `npx gitnexus analyze` to refresh the index
