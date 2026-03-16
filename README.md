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
    - `scp_upload` / `scp_download`
  - Boundary target discovery:
    - `boundary_search_targets`
- Event bridge from Rust -> frontend:
  - `pty-output`
  - `pty-exit` with exit code + reason
  - `function-status`
  - `openvpn-log` (elevated helper log stream)

### Status page behavior

- Shows a card for each function runtime (OpenVPN, Boundary auth, server tunnels/SSH).
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
  - auth type: `oidc` (browser login) or `password`.
  - auth method id, login name, password (password stored in OS keyring).
  - optional extra SSH options for `boundary connect ssh`.
- Boundary auth page:
  - Runs `boundary authenticate oidc` or `boundary authenticate password` in a PTY.
  - Shows a live login terminal.
  - Status page reflects CLI token state via `boundary config get-token` (`boundary-auth` function card with “Token active” pill).
- Boundary target search:
  - Searches Boundary targets via CLI.
  - Displays results in a table (Name, ID, Type, Description, Address).
  - “Add to list” creates a server entry using the target **Address** as `host`.

### Config page and Git-based sync

- The Config page covers:
  - **App runtime** defaults (terminal cols/rows).
  - **OpenVPN** settings. Credentials stay in the OS keyring; the OpenVPN executable path and `.ovpn` file path are device-local and are not synced to the repo.
  - **Boundary** controller/auth settings. The Boundary CLI executable path is device-local and is not synced to the repo.
  - **Config sync (Git repo)**:
    - A `Config repo root` field pointing to a local clone of a private Git repo (e.g. `config-sync`) that contains `app-config/config.json` and (optionally) `app-config/servers.json`.
    - Read-only display of the resolved app `config.json` path.
    - Two helper buttons:
      - **Copy from repo → app** (`sync_config_pull`): copies shared `app-config/config.json` (and `app-config/servers.json` when present) from the repo into the app config directory.
      - **Copy from app → repo** (`sync_config_push`): copies the shared app config file and `servers.json` into `app-config/config.json` / `app-config/servers.json` in the repo.
    - Recommended usage:
      - Use a private Git repo as the source of truth for config.
      - Optionally place the repo clone inside a cloud-synced folder (Google Drive/OneDrive/Dropbox) so Git handles history/merging and the cloud service provides backup and multi-device availability.

### Servers page (SSH / Boundary / SCP)

- Each server record includes:
  - name, host, **OS** (`linux` or `windows`), SSH port, SSH username.
  - connection method: `ssh` (direct) or `boundary`.
  - for Boundary: target ID, tunnel listen port, optional services list.
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
    - Shows a dedicated SCP result indicator below the buttons with clear success/error states.

## Notes and limitations

- No additional auth/sandboxing beyond what OpenVPN/Boundary/SSH provide.
- Boundary features assume a working Boundary CLI and controller configuration.
- SCP assumes `scp` is available on the system and uses `StrictHostKeyChecking=accept-new` and a modern MAC algorithm by default.

## Suggested next steps

1. Add more function workers (e.g., other VPNs or maintenance tasks).
2. Persist per-server terminal history between app launches.
3. Add richer error handling and diagnostics for Boundary/SSH/SCP flows.
4. Harden security (permission scoping, safer command construction, audit logging).
