# SuperControlPanel MCP Phased Checklist

## Phase 0 — Design

- [x] Finalize MCP transport choice
- [x] Finalize auth/token approach
- [x] Finalize v1 tool list
- [x] Classify sensitive tools
- [x] Decide whether raw PTY tools are deferred to v2
- [x] Confirm helper CLI in v2 will reuse MCP rather than a separate IPC protocol

## Phase 1 — Backend refactor

- [x] Extract shared model types from `src-tauri/src/main.rs` into `src-tauri/src/models/`
- [x] Extract function status service from Tauri command handlers
- [x] Extract PTY/session service
- [x] Extract OpenVPN service
- [x] Extract Boundary service
- [x] Extract server management service
- [x] Extract remote browser and SCP service
- [x] Extract config and sync service
- [x] Extract Caddy service
- [x] Keep existing Tauri commands as thin adapters

## Phase 2 — Config and UI

- [x] Add MCP server settings to device-local config
- [x] Add Config page MCP section
- [x] Add enable/disable toggle
- [x] Add bind address and port fields
- [x] Add auth token field and regenerate action
- [x] Add allowed tool category controls
- [x] Add sensitive-action confirmation setting
- [x] Add MCP running/error status display
- [x] Add copyable client config snippet

## Phase 3 — MCP server skeleton

- [x] Add MCP module(s) to Rust backend
- [x] Implement server startup and shutdown lifecycle
- [x] Implement tools listing
- [x] Implement tool dispatch
- [x] Implement resources listing
- [x] Implement resource reads
- [x] Wire MCP handlers to shared services

## Phase 4 — Read-only tool coverage

- [x] `app_get_overview`
- [x] `function_status_list`
- [x] `function_status_get`
- [x] `openvpn_get_status`
- [x] `boundary_get_status`
- [x] `servers_list`
- [x] `server_get`
- [x] `config_get_paths`

## Phase 5 — Runtime control coverage

- [x] `openvpn_start`
- [x] `openvpn_stop`
- [x] `boundary_auth_start`
- [x] `boundary_auth_stop`
- [x] `server_tunnel_start`
- [x] `server_tunnel_stop`
- [x] `server_ssh_start`
- [x] `server_ssh_stop`
- [x] `caddy_start`
- [x] `caddy_stop`

## Phase 6 — Mutation tools

- [ ] `server_upsert`
- [ ] `server_delete`
- [ ] `config_save`
- [ ] `config_save_partial`
- [ ] `sync_settings_get`
- [ ] `sync_settings_save_partial`

## Phase 7 — Remote browser and SCP

- [ ] `remote_browser_open`
- [ ] `remote_browser_close`
- [ ] `remote_browser_list_children`
- [ ] `remote_browser_stat_path`
- [ ] `remote_browser_create_directory`
- [ ] `remote_browser_delete_path`
- [ ] `scp_upload`
- [ ] `scp_download`
- [ ] `scp_start_upload`
- [ ] `scp_start_download`
- [ ] `scp_transfer_list`
- [ ] `scp_transfer_get`

## Phase 8 — Config sync and Git

- [ ] `config_sync_pull`
- [ ] `config_sync_push`
- [ ] `config_git_pull`
- [ ] `config_git_push`

## Phase 9 — Hardening and polish

- [ ] Enforce localhost-only default bind
- [x] Enforce token auth
- [ ] Redact secrets in all config and status outputs
- [ ] Add approval policy for destructive tools
- [ ] Add audit logging for MCP calls
- [ ] Standardize error and warning responses
- [ ] Add notifications for status and transfer changes if needed
- [ ] Update README and AGENTS docs if contract changes

## Phase 10 — `supercontrolpanelhelper` CLI over MCP

- [x] Define helper command tree and argument model
- [ ] Implement helper-to-MCP connection logic
- [ ] Reuse MCP auth/token configuration
- [x] Add shell-friendly exit codes
- [x] Add `--json` output mode
- [ ] Add helper commands for core status and runtime controls
- [x] Document helper usage in docs

## Progress notes

- MCP planning docs have been created under `docs/mcp/`
- Old SCP planning/checklist docs were moved from the repo root into `docs/scp/`
- PR1 model extraction has been implemented and verified with `cargo check`


## Progress notes update

- Phase 0 auth decision: use a device-local bearer token stored in `sync-local.json`
- MCP config fields now exist in device-local sync settings so the UI/server bootstrap can build on a stable contract
- Backend refactor items are now fully complete for Phase 1


## Skeleton status note

- Phase 3 now includes a minimal localhost HTTP listener bound from the desktop app
- MCP runtime state, tool/resource catalogs, dispatch/read plumbing, and token-gated HTTP routes now exist in Rust
- Current runtime reports `implementationStage = http-minimal`; full MCP-spec transport polish is still pending
- Phase 5 runtime-control tools now dispatch through the same internal action paths used by the Tauri commands
