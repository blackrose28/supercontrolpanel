# Super Control Panel (MVP)

Minimal cross-platform desktop prototype using:

- Tauri (desktop shell)
- xterm.js (terminal UI)
- Rust PTY backend (`portable-pty`)

This prototype provides a basic function-runner shell where each feature function can own and control a PTY session.

## What is implemented

- Basic Tauri + Vite project structure
- Multi-page UI:
  - **Status** page for all function runtime states (OpenVPN, Boundary auth, server tunnels/SSH)
  - **OpenVPN** function page (first PTY-driven function)
  - **Caddy** page (runs `caddy run --config Caddyfile` in a configured folder)
  - **Servers** page for SSH/Boundary servers, tunnels, SSH, and SCP
  - **Boundary** page for Boundary CLI authentication and target search
  - **Config** page for app, OpenVPN, Boundary, and Git-based config sync settings
- Rust PTY session manager with Tauri commands:
  - `open_pty`
  - `write_pty`
  - `resize_pty`
  - `close_pty`
- Function orchestration commands:
  - `list_function_statuses`
  - `get_app_config`
  - `save_app_config`
  - `detect_openvpn_executable`
  - `start_openvpn_function`
  - `stop_openvpn_function`
  - `get_config_paths` (resolve app config.json path for UI)
  - `sync_config_pull` / `sync_config_push` (copy config between app config dir and Git repo)
  - `start_boundary_auth` / `stop_boundary_auth`
  - Server management:
    - `list_servers`, `upsert_server`, `delete_server`
    - `start_server_tunnel` / `stop_server_tunnel`
    - `start_server_ssh` / `stop_server_ssh`
    - `start_server_rdp`
    - `open_remote_browser`, `close_remote_browser`
    - `list_remote_browser_children`, `stat_remote_browser_path`
    - `stat_remote_path`
    - `start_scp_upload` / `start_scp_download`
    - `scp_upload` / `scp_download` (`recursive` supported for folder copy)
  - Boundary target discovery:
    - `boundary_search_targets`
- Event bridge from Rust -> frontend:
  - `pty-output`
  - `pty-exit` with exit code + reason
  - `function-status` (`boundaryTokenSource` is included for Boundary auth status updates)
  - `openvpn-log` (elevated helper log stream)

### Status page behavior

- Shows a card for each function runtime (OpenVPN, Boundary auth, server tunnels/SSH).
- Each card includes a `Dismiss` action that hides just that current status entry for the rest of the app session. If that function later reports a different status, its card automatically reappears.
- Cards are **clickable shortcuts**:
  - OpenVPN card: navigates to the OpenVPN page, focuses the start button, and scrolls the VPN terminal into view.
  - Boundary auth card: navigates to the Boundary page, focuses the login button, and scrolls the Boundary terminal into view.
  - Server tunnel/SSH cards: are titled as **`Tunnel – {serverName}`** or **`SSH – {serverName}`**, navigate to the Servers page, select the corresponding server row, and scroll/focus that row while restoring its tunnel/SSH terminal scrollback. Once a tunnel or SSH session is explicitly stopped and its PTY torn down (idle state with no session), its card is hidden from the Status page to keep the list focused on active work.

## Project structure

```text
.
├─ src/
│  ├─ main.ts          # multi-page UI + status/log/event bridge
│  └─ styles.css
├─ src-tauri/
│  ├─ src/main.rs      # PTY runtime + function worker orchestration
│  ├─ tauri.conf.json
│  └─ Cargo.toml
├─ index.html
├─ package.json
└─ AGENTS.md
```

## Prerequisites

- Node.js 20+
- Rust stable toolchain
- Tauri system prerequisites for your OS:
  - [https://tauri.app/start/prerequisites/](https://tauri.app/start/prerequisites/)

## Run locally (dev)

```bash
npm install
npm run tauri dev
```

## Build desktop app

```bash
npm run tauri build
```

## Feature details

### OpenVPN function

- One concrete function worker: `openvpn-connect`.
- Config page provides OpenVPN executable **Browse** + **Auto detect** helpers.
- OpenVPN runs via elevated helper process on Windows (UAC prompt).
- OpenVPN runs via `sudo` inside PTY on Linux/macOS.
- OpenVPN launch is built from config (`executablePath` + `ovpnPath` + `extraArgs`).
- OpenVPN auth password is sent as `6-digit OTP + password` when OTP secret is configured.
- Non-secret config is local JSON under app config directory.
- OpenVPN password and OTP secret use OS secure storage (Windows Credential Manager, macOS Keychain, Linux Secret Service via libsecret).
- Linux needs a running Secret Service (e.g. GNOME Keyring / KWallet) for keyring access.

### Boundary integration

- Boundary config (on Config page):
  - CLI executable path (or `boundary` on `PATH`).
  - controller URL (`BOUNDARY_ADDR`).
  - auth type: `oidc` or `password`.
  - OIDC login mode: browser or automated hidden window.
  - auth method id, login name, password (password stored in OS keyring).
  - optional OIDC-site username, password, and OTP secret for automated OIDC; the OIDC password and OTP secret are also stored in the OS keyring.
  - optional extra SSH options for `boundary connect ssh`.
- Boundary auth page:
  - Runs `boundary authenticate oidc`, `boundary authenticate password`, or an app-managed automated OIDC flow.
  - Automated OIDC uses Boundary's HTTP auth API plus a hidden Tauri webview to fill common username/password/OTP forms, then stores the resulting token in the app secure store and injects it into later Boundary CLI commands via `BOUNDARY_TOKEN`.
  - Shows a compact live login terminal intended for status/output, not long interactive work.
- Status page reflects the current Boundary token via `boundary config get-token`, using either the CLI keyring token or the app-managed automation token, and labels the auth card as `App token` or `CLI token` based on an explicit backend status field.
- Boundary target search:
  - Lives on the Boundary page next to the auth terminal.
  - Searches Boundary targets via CLI.
  - Filter text matches target name, target ID, and description.
  - Displays results in a table (Name, ID, Type, Description, Address).
  - “Add to list” creates a server entry using the target **Address** as `host`.

### Config page and Git-based sync

- The Config page covers:
  - **App runtime** defaults (terminal cols/rows).
  - **OpenVPN** settings. Credentials stay in the OS keyring; the OpenVPN executable path and `.ovpn` file path are device-local and are not synced to the repo.
  - **Caddy** settings. The selected Caddy working folder (must contain `Caddyfile`) is device-local and is not synced to the repo.
  - **Boundary** controller/auth settings. The Boundary CLI executable path is device-local and is not synced to the repo.
  - **Config sync (Git repo)**:
    - A `Config repo root` field pointing to a local clone of a private Git repo (e.g. `config-sync`) that contains `app-config/config.json` and (optionally) `app-config/servers.json`.
    - Read-only display of the resolved app `config.json` path.
    - Helper buttons:
      - **Copy from repo → app** (`sync_config_pull`): copies `app-config/config.json` (and `app-config/servers.json` when present) from the repo into the app config directory.
      - **Copy from app → repo** (`sync_config_push`): copies the app config file and `servers.json` into `app-config/config.json` / `app-config/servers.json` in the repo.
      - **Git pull** (`config_git_pull`): runs `git pull` inside the repo root.
      - **Git add + commit + push** (`config_git_push`): runs `git add .`, then `git commit -m "Change config from {computerName}"`, then `git push` inside the repo root.
    - Recommended usage:
      - Use a private Git repo as the source of truth for config.
      - Optionally place the repo clone inside a cloud-synced folder (Google Drive/OneDrive/Dropbox) so Git handles history/merging and the cloud service provides backup and multi-device availability.

### Servers page (SSH / Boundary / SCP)

- Each server record includes:
  - name, host, **OS** (`linux` or `windows`), SSH port, SSH username.
  - connection method: `ssh` (direct) or `boundary`.
  - for Boundary: target ID, tunnel listen port, optional services list.
- Local server discovery:
  - A dedicated search field filters the internal server list by name, host, SSH user, service text, or Boundary target ID.
  - This is frontend-only filtering; it does not query Boundary.
- Per-server PTYs:
  - **Tunnel** PTY for `boundary connect` (per server).
  - **SSH** PTY per server (direct or via localhost + listen port).
  - Status page shows separate cards for tunnel and SSH (with server name in the title as `Tunnel – {serverName}` / `SSH – {serverName}`).
- Terminals:
  - Two xterm panes under the table: Tunnel and SSH.
  - A **Show tunnel panel** checkbox in the Servers actions bar lets you hide the tunnel PTY to reclaim space for the SSH terminal when you mostly care about the interactive shell.
  - Only one pair of xterms is rendered; each server has its own PTY session IDs and **buffered scrollback**.
  - When you select a server row, the app restores that server’s buffered tunnel/SSH output so switching servers preserves their terminal history.
- Controls:
  - Start/Stop Tunnel.
  - Start/Stop SSH.
  - **RDP** (Windows servers only):
    - Enabled when a server’s OS is set to `windows`.
    - Launches the OS-native Remote Desktop client (e.g. `mstsc.exe` on Windows; `rdp://` URL via `open`/`xdg-open` elsewhere) targeting the server `host`.
  - **SCP Upload / Download**:
    - Works for direct SSH and for Boundary servers over the tunnel (connecting to `127.0.0.1` and the listen port).
    - Opens a two-pane remote browser modal: a lazy folder tree on the left and the current folder contents on the right.
    - The remote browser keeps a persistent backend remote-browser session open while the modal is visible, so repeated navigation reuses the same remote connection instead of reconnecting per click.
    - On Windows, the browser uses one persistent OpenSSH-backed shell session for navigation while the modal is open, so repeated browse commands reuse the same SSH connection.
    - Lifecycle safety is enforced on both sides: leaving the Servers view, changing the selected server, editing/deleting that server, or restarting/stopping its tunnel will close the active remote-browser session instead of leaving stale connections behind.
    - The browser opens at the remote user’s home directory, and the tree starts collapsed at that root instead of opening from `/`.
    - Breadcrumbs above the contents pane let you jump back up the active path, while manual path entry remains available as a fallback instead of the primary flow.
    - Tree expand/collapse is now a local UI state change; opening a folder is what fetches its contents.
    - Picker behavior is mode-aware: upload flows only accept destination folders, download-file only accepts remote files, and download-folder only accepts remote folders.
    - Single-click selects valid items; double-click opens folders for deeper browsing.
    - Supports explicit file and folder actions; folder transfers use recursive SCP.
    - Shows an in-session transfer history below the buttons with running, success, and error states plus source/destination summaries.
    - Transfer history now includes a progress bar and percentage when SCP reports progress through the PTY-backed transfer path.
    - For Boundary servers, the app now checks that the tunnel is running before browse/transfer actions and shows a clearer error if it is not.
    - Upload flows normalize remote paths and confirm before overwriting known remote targets.
  - Remote browse groundwork:
    - Backend commands now expose the persistent browser session API (`open_remote_browser`, `close_remote_browser`, `list_remote_browser_children`, `stat_remote_browser_path`).
    - `stat_remote_path` remains temporarily for SCP overwrite checks outside the browser session flow.
    - The persistent browser layer is SFTP-backed and is the path the remote browser now uses for repeated browse operations.

## Notes and limitations

- No additional auth/sandboxing beyond what OpenVPN/Boundary/SSH provide.
- Boundary features assume a working Boundary CLI and controller configuration.
- SCP assumes `scp` is available on the system and uses `StrictHostKeyChecking=accept-new` and a modern MAC algorithm by default.

## Suggested next steps

1. Add more function workers (e.g., other VPNs or maintenance tasks).
2. Persist per-server terminal history between app launches.
3. Add richer error handling and diagnostics for Boundary/SSH/SCP flows.
4. Harden security (permission scoping, safer command construction, audit logging).
