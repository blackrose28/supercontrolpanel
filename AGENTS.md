# AGENTS.md

Guidance for AI agents working in this repository.

## Goal

Maintain and extend a PTY-driven function runner: each feature function can launch/control a PTY and report runtime status to the UI.

## Tech stack

- Frontend: Vite + TypeScript + xterm.js
- Desktop shell: Tauri v2
- Backend: Rust + `portable-pty`

## Key files

- `src/main.ts`: multi-page UI (Status + OpenVPN + Servers + Boundary + Config), PTY/event wiring
- `src/styles.css`: terminal layout and basic app styling
- `src-tauri/src/main.rs`: PTY session lifecycle + function orchestration
- `src-tauri/tauri.conf.json`: build/dev config and window settings

## PTY command contract

Frontend invokes:

- `open_pty({ cols, rows }) -> string` (session id)
- `write_pty({ id, data })`
- `resize_pty({ id, cols, rows })`
- `close_pty({ id })`

Rust emits:

- `pty-output` with `{ id, data }`
- `pty-exit` with `{ id, exitCode, reason }`
- `openvpn-log` with `{ functionId, sessionId, data }`

Function runtime contract:

- `list_function_statuses() -> FunctionStatus[]`
- `get_app_config() -> AppConfig`
- `save_app_config({ config }) -> AppConfig`
- `detect_openvpn_executable() -> string`
- `start_openvpn_function({ command, username, password }) -> { functionId, sessionId }`
- `stop_openvpn_function()`
 - `get_config_paths() -> { appConfigPath }`
 - `sync_config_pull({ repoRoot }) -> void`
 - `sync_config_push({ repoRoot }) -> void`
- `function-status` event with:
  - `{ functionId, state, message, boundaryTokenSource?, sessionId, updatedAtMs }`

Additional commands/events added in this iteration:

- **Server management (SSH / Boundary / SCP)**
  - `list_servers() -> Server[]`
  - `upsert_server({ server }) -> Server[]`
  - `delete_server({ id }) -> Server[]`
  - `start_server_tunnel({ serverId, cols, rows }) -> { functionId, sessionId }`
  - `stop_server_tunnel({ serverId })`
  - `start_server_ssh({ serverId, cols, rows }) -> { functionId, sessionId }`
  - `stop_server_ssh({ serverId })`
  - `start_server_rdp({ serverId }) -> void`
  - `scp_upload({ serverId, localPath, remotePath }) -> string`
  - `scp_download({ serverId, remotePath, localPath }) -> string`
  - Function IDs:
    - OpenVPN: `openvpn-connect`
    - Boundary auth: `boundary-auth`
    - Server tunnel: `server-tunnel-{serverId}`
    - Server SSH: `server-ssh-{serverId}`

- **Boundary integration**
  - `start_boundary_auth({ cols, rows }) -> { functionId, sessionId }`
  - `stop_boundary_auth()`
  - `boundary_search_targets({ filter }) -> BoundaryTarget[]`

Config behavior:

- `config.json` persists in the app config directory.
- OpenVPN `extraArgs`/`username` live in `config.json`; `executablePath` and `ovpnPath` are device-local in `sync-local.json`; password and otpSecret live in the OS keyring (`keyring` crate: Windows Credential Manager, macOS Keychain, Linux Secret Service).
- Boundary config section stores:
  - CLI path (device-local in `sync-local.json`, not synced)
  - controller URL (`BOUNDARY_ADDR`)
  - auth kind (`oidc` or `password`)
  - OIDC login mode (`browser` or `automated`)
  - auth method id
  - login + password (password stored in OS keyring)
  - optional OIDC site username, password, and OTP secret for automated OIDC (password + OTP secret stored in OS keyring)
  - extra SSH options for `boundary connect ssh`
- `save_app_config` writes secrets to keyring and never writes them to disk; legacy plaintext in JSON is migrated on load.
- When password prompt appears (non-Windows PTY path), backend sends `TOTP(otpSecret, 6 digits) + password`.
- On Windows, OpenVPN uses an elevated helper (UAC) instead of the normal PTY flow.
- On Linux/macOS, OpenVPN runs in PTY via `sudo`.
- Config sync helpers (`sync_config_pull` / `sync_config_push`) copy only the shared `config.json` and `servers.json` between the app config directory and `app-config/config.json` / `app-config/servers.json` inside a Git-managed repo. Device-local settings such as OpenVPN executable/OVPN paths, Boundary CLI path, Caddy folder, and sync repo root stay in `sync-local.json`. The repo root path is configured via the Config page “Config sync (Git repo)” section, which is the recommended way to sync config across machines (optionally inside a Google Drive/OneDrive/Dropbox folder).

- `save_sync_settings` uses partial-patch semantics: only fields explicitly provided by the caller are updated on disk, so unrelated device-local settings (e.g. `caddyFolder`) are not accidentally cleared.

Do not break these names unless updating both frontend and backend in the same change.

## Development workflow

1. Install deps: `npm install`
2. Start dev app: `npm run tauri dev`
3. Build app: `npm run tauri build`

## Editing rules

- Keep changes minimal and coherent.
- Preserve cross-platform behavior (Windows + macOS/Linux).
- Prefer explicit error messages over silent failure paths.
- Avoid introducing heavy frameworks unless requested.
- Keep docs in sync when command/event contracts change.

## Rust backend notes

- `SessionStore` keeps active PTY sessions in memory.
- `FunctionStore` keeps function runtime states for Status page.
- PTY reads are handled on a dedicated thread and forwarded as Tauri events.
- A monitor thread watches child exit and updates both `pty-exit` and `function-status`.
- Keep locks short-lived and avoid holding multiple locks unnecessarily.

## Frontend notes

- Navigation is page-style toggling, no router framework.
- Status page:
  - Renders from `function-status` events plus initial `list_function_statuses`.
  - Each status card is clickable and navigates to the owning page (OpenVPN, Servers, Boundary) while focusing the relevant control or server row.
  - Special card for Boundary auth (`boundary-auth`) with a source-aware token status pill (`App token` or `CLI token` when authenticated).
  - Server tunnel/SSH statuses are titled as `Tunnel – {serverName}` / `SSH – {serverName}`; orphan statuses for deleted servers are filtered out, and stopped tunnel/SSH entries (idle state with no session) are hidden once their PTY has been torn down.
- OpenVPN page:
  - May run without PTY input when elevated helper mode is active.
  - Output path is `openvpn-log` (helper) or `pty-output` (non-Windows fallback).
- Servers page:
  - Table of `Server` records (`name`, `host`, `os`, connection method, SSH port, SSH user, services).
  - Includes its own local search field for filtering the internal server list without querying Boundary.
  - Supports two connection methods:
    - `ssh` (direct host/port/user)
    - `boundary` (target ID, tunnel listen port, SSH user to connect to `127.0.0.1`).
  - Per-server actions:
    - Start/stop **Tunnel** (Boundary `connect` PTY).
    - Start/stop **SSH** (either direct or via localhost + listen port).
  - **RDP** launch for Windows servers (fires `start_server_rdp` and opens the OS-native RDP client based on server `host`).
    - SCP Upload/Download (works for both direct SSH and Boundary via the tunnel).
  - Each server maintains its own PTY session IDs (tunnel + ssh) and buffered terminal output so switching rows restores that server's scrollback instead of clearing it.
  - Servers view includes a "Show tunnel panel" checkbox that toggles visibility of the tunnel PTY panel so the SSH terminal can use more space when tunnel output is not needed.
- Boundary page:
  - Runs `boundary authenticate` in a PTY for browser/password mode, or an app-managed automated OIDC flow using Boundary's HTTP auth API plus a hidden Tauri webview.
  - Shows a compact live login PTY and updates `boundary-auth` function status based on the active Boundary token (CLI keyring or app-managed automation token).
  - Owns the Boundary target search UI; search results can be added into the Servers list.
  - Boundary target search filter matches target name, target ID, and description text.

## If you add features

- Update `README.md` with user-facing behavior.
- Update this file if architecture/contracts change.
