# SuperControlPanel MCP Phased Checklist

## Phase 0 — Design

- [x] Finalize MCP transport choice
- [ ] Finalize auth/token approach
- [x] Finalize v1 tool list
- [x] Classify sensitive tools
- [x] Decide whether raw PTY tools are deferred to v2
- [x] Confirm helper CLI in v2 will reuse MCP rather than a separate IPC protocol

## Phase 1 — Backend refactor

- [x] Extract shared model types from `src-tauri/src/main.rs` into `src-tauri/src/models/`
- [ ] Extract function status service from Tauri command handlers
- [ ] Extract PTY/session service
- [ ] Extract OpenVPN service
- [ ] Extract Boundary service
- [ ] Extract server management service
- [ ] Extract remote browser and SCP service
- [ ] Extract config and sync service
- [ ] Extract Caddy service
- [ ] Keep existing Tauri commands as thin adapters

## Phase 2 — Config and UI

- [ ] Add MCP server settings to device-local config
- [ ] Add Config page MCP section
- [ ] Add enable/disable toggle
- [ ] Add bind address and port fields
- [ ] Add auth token field and regenerate action
- [ ] Add allowed tool category controls
- [ ] Add sensitive-action confirmation setting
- [ ] Add MCP running/error status display
- [ ] Add copyable client config snippet

## Phase 3 — MCP server skeleton

- [ ] Add MCP module(s) to Rust backend
- [ ] Implement server startup and shutdown lifecycle
- [ ] Implement tools listing
- [ ] Implement tool dispatch
- [ ] Implement resources listing
- [ ] Implement resource reads
- [ ] Wire MCP handlers to shared services

## Phase 4 — Read-only tool coverage

- [ ] `app_get_overview`
- [ ] `function_status_list`
- [ ] `function_status_get`
- [ ] `openvpn_get_status`
- [ ] `boundary_get_status`
- [ ] `servers_list`
- [ ] `server_get`
- [ ] `config_get_paths`

## Phase 5 — Runtime control coverage

- [ ] `openvpn_start`
- [ ] `openvpn_stop`
- [ ] `boundary_auth_start`
- [ ] `boundary_auth_stop`
- [ ] `server_tunnel_start`
- [ ] `server_tunnel_stop`
- [ ] `server_ssh_start`
- [ ] `server_ssh_stop`
- [ ] `caddy_start`
- [ ] `caddy_stop`

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
- [ ] Enforce token auth
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
