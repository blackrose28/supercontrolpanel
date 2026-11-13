# Super Control Panel

Cross-platform desktop app using:

- Tauri (desktop shell)
- xterm.js (terminal UI)
- Rust PTY backend (`portable-pty`)

Super Control Panel is a PTY-driven desktop control panel where each feature can own and control its own runtime session.

## What is implemented

- Basic Tauri + Vite project structure
- Multi-page UI:
  - **Status** page for all function runtime states (OpenVPN, CC control plane, Boundary auth, server tunnels/SSH)
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
    - `create_remote_browser_directory`, `delete_remote_browser_path`
    - `start_scp_upload` / `start_scp_download`
    - `scp_upload` / `scp_download` (`recursive` supported for folder copy)
  - Boundary target discovery:
    - `boundary_search_targets`
- Event bridge from Rust -> frontend:
  - `pty-output`
  - `pty-exit` with exit code + reason
  - `function-status` (`boundaryTokenSource` is included for Boundary auth status updates)
  - `openvpn-log` (elevated helper log stream)
- Control Plane / CC integration:
  - outbound CC websocket connection with reconnect + heartbeat
  - telemetry upload/history/admin visibility
  - implemented safe server-triggered actions:
    - request fresh status snapshot
    - request inventory
    - request recent logs
    - restart OpenVPN
    - restart Boundary auth
    - restart server tunnel
    - restart server SSH

### Status page behavior

- Shows a card for each function runtime (OpenVPN, CC control plane, Boundary auth, server tunnels/SSH).
- Each card includes a `Dismiss` action that hides just that current status entry for the rest of the app session. If that function later reports a different status, its card automatically reappears.
- Cards are **clickable shortcuts**:
  - Control plane card: navigates to the Config page and focuses the CC settings section.
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
npm run tauri:dev
```

## Build desktop app

```bash
npm run tauri:build
```

## Run helper CLI

```bash
npm run helper:run -- status
npm run helper:run -- openvpn status
```

## Standalone CLI preview

The separate `scpctl` binary runs with the desktop closed and HTTP disabled. This preview
supports read-only server inventory and shared configuration, local profile initialization,
and a Linux-only Caddy foreground command. Other runtime commands and stdio MCP are still planned. The helper above continues to control desktop-owned sessions.

```bash
cargo run -p scp-cli -- server list
cargo run -p scp-cli -- --config-dir /path/to/cli-profile server list --json
cargo run -p scp-cli -- --config-dir /path/to/cli-profile profile init --json
cargo run -p scp-cli -- config show --json --config-dir /path/to/cli-profile
# Linux preview; requires Caddy on PATH and a new output filename:
cargo run -p scp-cli -- caddy run --foreground --folder /path/to/caddy --output /path/to/new-caddy.log --json
cargo build --locked --release -p scp-cli
cargo test --locked --workspace
```

The binary is built in `target/release/` (`scpctl.exe` on Windows). This workspace builds
without Node.js, Tauri, GTK, WebKit, or desktop assets; it needs Rust 1.89 or newer
(for standard-library file locking).
The desktop remains a separate Cargo package under `src-tauri` with its existing commands.

By default, inventory comes from `servers.json` under a separate CLI directory:

| Platform | Default directory |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME/supercontrolpanel-cli`, or `~/.config/supercontrolpanel-cli` |
| macOS | `~/Library/Application Support/supercontrolpanel-cli` |
| Windows | Roaming AppData's `supercontrolpanel-cli` directory |

Use `--config-dir` to select another CLI profile. Listing does not create directories,
migrate settings, read credentials, or start connections. To try an existing inventory,
copy only `servers.json` into a separate CLI profile. Settings import is not available in
this preview. A missing inventory returns an empty list; unreadable or malformed inventory
is an error. `--json` produces a server array on stdout; errors always go to stderr.
Exit codes are `0` for success, `1` for an operation failure, and `2` for invalid arguments.
Run `scpctl --help` or `scpctl --version` for usage/version information.

`scpctl profile init` creates the selected directory and a versioned `profile.json` with
a random, stable credential namespace for future keyring/cache isolation. It does not
access credentials. Repeating initialization preserves the identity; malformed or
unsupported metadata returns an error without replacing it. Keep this identity local to
the profile; future settings imports must preserve it rather than copy the source identity.
Use a separate CLI directory, not the desktop's config directory. On Unix, newly created
profile directories and metadata/lock files restrict access to their owner; existing
directory permissions are preserved. Windows uses inherited directory permissions.

Initialization takes an exclusive OS file lock on `runtime-owner.lock` and releases it
when the command exits. A competing owner fails immediately (exit 1); choose another
profile or stop the current owner. The lock file remains on disk: **do not delete it**
to clear ownership. OS locks release on process exit, including forced termination.
Runtime and config writers must retain the core ownership guard for their full
lifetime. Inventory listing stays read-only and can run while a profile is owned.
This lock coordinates this CLI profile only; it does not isolate host VPN routes or
external Boundary state. Guard drop explicitly unlocks even if a concurrent subprocess
briefly inherits the open file descriptor. Caddy retains this lock through foreground
output drainage and direct-child cleanup.

The core now provides a profile-bound credential store and a cache keyed by namespace
and account. Its optional `os-keyring` feature uses the same persistent native stores as
the desktop. Current CLI commands do not access credentials; the new adapter is a
foundation for runtime/settings operations. The desktop continues using its existing
credential identifiers and cache. Secret debug output is redacted, owned secret buffers
are zeroized on drop, and missing credentials are distinct from locked/unavailable stores.
Cached reads require an explicit refresh after external changes; failed refreshes or
writes evict stale entries. Native calls can prompt/block, so host timeout and interaction
handling must be added before exposing them through commands or MCP.

Developers can validate the optional adapter with
`cargo test --locked --workspace --all-features` (uses injected backend/keyring mocks,
not real credentials). On Linux, building that feature also requires the D-Bus development
library and `pkg-config`; the default inventory/profile CLI build does not. Real native
keyring/unlock checks on supported OSes remain release gates.

Desktop and core also share the device-local `SyncSettings`/patch schema in
`scp_core::local_settings`, including existing JSON field names, defaults and random
MCP token generation. These models contain MCP/CC tokens, identity and paths; they
are not CLI/MCP output models. Desktop loading, normalization, migrations, persistence
and service activation remain host-local. This schema extraction adds no settings
command or automatic activation; settings import is still pending for the CLI.

Desktop and core now share the function-status store, PTY event payload types, and
session-exit status transitions. Host event delivery and Boundary token lookup run
outside the status mutex; late exits recheck the session before changing a replacement
login's status. Long Boundary exit reasons are truncated safely for Unicode text.
Desktop and core also share PTY session handles, launch registration, input, and close.
Reader/writer setup completes before spawning a child; duplicate IDs are rejected without
replacing an existing session, and a child that loses concurrent registration is killed
and reaped. Closing a session or dropping its final owner attempts direct-child cleanup.
Core exit polling is bound to the launched session: a stale monitor cannot poll/remove
another child that reuses its ID or clear that replacement's watchers. Weak monitors do
not keep children alive after the store is dropped. Desktop retains reader/polling
threads, events, telemetry, and session-ID generation. Host event delivery still uses
existing IDs; full lifecycle/publication ordering remains part of runtime supervision.
The core also provides a CLI-local `ProfileRuntime` that owns a private session store
and retains the profile lock until direct-child cleanup finishes. It allocates a fresh
session ID for every launch, using a per-runtime random nonce and a non-wrapping counter;
callers cannot inject or reuse IDs through this context. Returned readers and weak
monitors do not retain child/profile ownership. The Linux Caddy host now uses this context.
Its new core-only `run_foreground` path launches one child, streams raw PTY bytes to a
supplied writer, then flushes output before returning the exit payload. Output errors and
Rust unwinding attempt direct-child cleanup while retaining profile ownership. Nonzero
child exits remain exit payloads, distinct from host I/O errors. This synchronous path
has no input forwarding or cancellation: reads, output writers, native cleanup, or a
descendant retaining the PTY can block indefinitely. It starts no background workers.
A separate core-only `run_foreground_cancellable` path observes a host-owned cancellation
flag while a scoped worker drains output. Cancellation attempts direct-child close before
joining the worker; every worker finishes before the call returns or unwinds, and profile
ownership is retained throughout. Completed runs still drain/flush before returning exit.
Cancellation may discard unread output and skip flush; already-running I/O may finish.
A blocked sink, descendant-held reader, launch, or native cleanup can still delay return
indefinitely. This is cooperative cancellation with no hard shutdown deadline. Worker I/O
errors and panics remain visible after cleanup. The host must keep the flag set until return.
Process-tree/elevated-helper supervision and status/event publication remain pending.
Caddy launch preparation is shared with the desktop:
`scp_core::caddy::build_caddy_command` performs the existing folder/Caddyfile checks
and builds `caddy run --config Caddyfile` with a literal working directory, without a
shell, credential access, file writes, or launch. Existence checks are not Caddyfile
validation or a guarantee that paths remain valid at launch.

`scpctl caddy run --foreground --folder FOLDER --output NEW_FILE` now runs Caddy through
the core PTY runtime on Linux. It takes the selected CLI profile's owner lock and uses
only the explicit folder; it does not read desktop configuration or credentials. Windows
and macOS return an explicit unsupported error before creating a profile/log or launching
a child; native host qualification remains pending. Caddy must be available on `PATH`.

Raw PTY bytes go to the new output file (Unix mode `0600`, subject to umask), never stdout.
Existing files and symlinks are rejected; parent directories must already exist. Completed
runs drain and flush all bytes before reporting. The log can grow without limit; there is
no in-memory output queue or log rotation. It can contain sensitive Caddy output. Write or
flush failure attempts direct-child cleanup and returns an error. Setup/launch failure can
leave the newly initialized profile and an empty log; errors do not remove those files.

SIGINT, SIGTERM and SIGHUP set a monotonic cancellation flag. Repeated signals do not force
exit past cleanup guards. This is **cooperative direct-child cleanup**, with no hard
shutdown deadline or process-tree guarantee: file I/O, native calls or descendants holding
the PTY can still block indefinitely. Cancellation may discard remaining output. Stdin is
unused, including EOF, and there is no input forwarding. This policy is for foreground
Caddy only; the future stdio MCP host must separately handle EOF/client shutdown.

`--json` writes one completion result to stdout: `{"outcome":"completed","exitCode":0}`
(or the actual child exit code), or `{"outcome":"cancelled"}`. Diagnostics use stderr.
Exit status is `0` only for a completed zero child exit; cancellation, nonzero/unknown
child exits and host failures return `1`, and invalid arguments return `2`. Setup/I/O
failures produce no completion JSON. These are per-invocation results; no cross-process
status, stop command, background daemon or MCP runtime is provided.

Linux acceptance tests exercise the actual CLI with an exec-only PATH fixture for
SIGINT/SIGTERM/SIGHUP, closed stdin, direct-child reaping and profile release, zero and
nonzero exits, missing executable and profile conflicts. They verify JSON/log separation
without running installed Caddy or user configuration. A controlled Linux descendant
in a separate session demonstrates two limits: normal completion waits for its inherited
PTY descriptors to close, while cancellation can release the profile with that descendant
still alive. Direct-child reaping and profile release therefore do not prove process-tree
cleanup. The test explicitly releases and reaps the descendant. Actual Caddy behavior
remains unqualified; test watchdogs do not establish a production shutdown bound.

`scpctl config show [--json] [--config-dir DIR]` reads only the selected CLI
profile's `config.json` using core's shared disk projection. JSON output is the
projected object, or `null` when the file or directory is missing; human output
is a labeled, pretty-printed projection or an explicit absence message. Missing
files succeed without synthesizing defaults or creating directories. Defaults
for omitted optional fields in an existing file follow the desktop schema.
Malformed/schema-invalid or unreadable files return exit 1 with stderr diagnostics
and no stdout; invalid arguments return 2. Parse errors do not echo file values.

This read does not initialize or lock a profile, read `sync-local.json` or the
keyring, migrate, write, or start MCP/CC/services. It can run while another process
owns the profile. Known credential/device-path fields are excluded and the legacy
OpenVPN command is cleared. **Free-text arguments, names and URLs may still contain
sensitive values** and are preserved; review output before sharing. Human output
escapes control characters using JSON encoding. Unknown fields are ignored. This
is a synchronous, unbounded single-file read with ordinary symlink/concurrent-writer
semantics, not a lossless settings backup, coordinated snapshot, import validator,
or unrestricted MCP resource. Desktop loading and saving keep their behavior.

The proposed Linux containment upgrade cannot be qualified on this devbox: creating
a dedicated cgroup fails with permission denied. The
[supervision design and release gates](docs/mcp/LINUX_OPERATION_SUPERVISION.md)
record the evidence and remaining ownership/crash requirements. The current Caddy
preview's cleanup guarantees are unchanged.

See [the standalone CLI delivery plan](docs/mcp/SETTINGS_BACKUP_STREAMABLE_HTTP_STANDALONE_CLI_PLAN.md)
for runtime ownership, credential isolation, and remaining delivery gates.

## MCP support

SuperControlPanel can now expose a **minimal localhost HTTP MCP surface** for AI agents and local helper tooling.

Current implementation highlights:

- disabled by default
- bearer-token protected
- localhost-only bind enforcement
- configurable from the Config page
- read-only, runtime-control, config-mutation, remote-browser, SCP, config-sync, and git tool coverage
- sensitive tools can be blocked unless MCP policy is switched to `allow-with-token`

Current HTTP routes:

- `GET /health`
- `GET /mcp/status`
- `GET /mcp/tools`
- `GET /mcp/resources`
- `POST /mcp/tools/call`
- `POST /mcp/resources/read`

This is still a **repo-local minimal transport**, not a final spec-polished MCP server.

A separate [published Rust SDK spike](docs/mcp/PUBLISHED_RUST_SDK_SPIKE.md) pins
`rmcp 3.10.0` and exercises official-client HTTP calls for `2026-07-28` and
`2025-11-25`, with JSON/SSE fixtures. Run
`cargo test --locked --manifest-path spikes/mcp-sdk/Cargo.toml` to reproduce it.
The desktop has a dev-only dependency probe; production still uses the routes above.
Gated tests also distinguish modern stream-close cancellation from legacy explicit
cancellation and verify an unrelated request survives. A cancelled request token
or joined listener does not stop the fixture's non-cooperative handler; tests
release and observe handler cleanup separately. An isolated injected adapter now
maps the one read-only `app_get_overview` operation using the shared policy,
catalog and result DTO. Official-client tests qualify that adapter for both revisions
in JSON and SSE modes: filtered schemas, DTO mapping, denial before host entry and
redacted returned host errors. After consuming matching progress notifications,
SSE tests verify legacy stateful explicit cancellation and modern stream-close
cancellation reach cooperative fake work while another read completes. Progress is
supplied by a test wrapper; the adapter host seam stays unchanged. A separately
named isolated HTTP boundary now replaces 4xx/5xx diagnostics with fixed text,
preserving status and known method guidance without buffering JSON/SSE responses.
Raw parser probes remain separate. HTTP 200 protocol errors and SDK logging are
outside that boundary; legacy unknown-method errors can still reflect input, so
protocol-error/logging policy remains a shipping gate. A separate unwrapped protocol
adapter now returns fixed `MCP method not supported` / `-32601` for custom requests,
including malformed known-method fallback. Raw and official clients preserve modern
404 and legacy 200 envelopes, including initialized legacy SSE. This does not cover
pre-dispatch parser/header/session errors. A separate opt-in negotiation mapper now
replaces selected finite HTTP 400 JSON errors with `MCP request negotiation failed`,
preserving native status, code and ID while dropping diagnostics/data. Raw and official
probes cover negotiation rejection and subsequent live JSON/SSE use. It inspects only
exact-size bodies up to 64 KiB; excluded bodies and HTTP 200 errors pass unchanged.
Neither mapping prevents SDK logs. Composing with the HTTP-status experiment still
removes error envelopes, in either order. The SDK dependency remains isolated; no desktop
wiring or CLI MCP command is enabled. Production host ownership/authentication,
shutdown bounds, third-party clients and native OS builds remain to be qualified
before standard `/mcp` is available.

The desktop's existing 40-tool discovery catalog now lives in
`scp-core::mcp_catalog`, with unchanged names, titles, categories and `readOnly`
fields. This is discovery metadata only: it does not enable these operations in
`scpctl` or supply execution authorization.

The five resource descriptors and exact URI-to-tool mappings now live in
`scp-core::mcp_resources`. Discovery order, fields and desktop resource-read
responses are unchanged. Resource labels (`status`, `inventory`, `config`) are
metadata; reads still use the tool dispatcher's execution policy. Unknown URIs
retain the existing error including the supplied URI. This extraction does not
expose resources in `scpctl` or add a standard MCP transport.

The custom tool/resource response envelope now lives in `scp-core::mcp_dto`,
re-exported through the existing desktop paths. Its five serialized fields and
opaque `data` are unchanged; desktop dispatch still supplies messages, warnings
and timestamps. This shared DTO does not add CLI operations or standard MCP results.

Function-status get/list argument and result contracts also live in `mcp_dto`.
Get keeps its existing permissive string lookup and errors; list returns a bare
array in store iteration order, and get returns a bare status object. Optional
status fields still serialize as null. These shared types add no CLI status command.

The overview result also uses a shared `McpAppOverviewResult`: `appVersion`,
`activeFunctions` (stored statuses with a session ID) and `knownFunctionCount`
(all stored statuses). Desktop counting and ignored-argument behavior are unchanged;
the schema example now matches those existing fields. No CLI overview command is added.

OpenVPN and Boundary MCP status results use shared `McpOpenVpnGetStatusResult`
and `McpBoundaryGetStatusResult` aliases. Each returns the bare stored status or
`null` when absent, with unchanged fields and values. Arguments remain ignored and
the desktop retains its fixed function-ID lookups. These contracts add no CLI
status command.

Server inventory MCP contracts also share `McpServerGetArgs`, `McpServerGetResult`
and `McpServersListResult`. Results remain bare server objects/arrays in loader
order, with every inventory field and duplicate preserved. Get accepts any string
`serverId` unchanged and retains the existing required/not-found errors; list
ignores all arguments. Desktop policy, loader and envelope behavior are unchanged.
This extraction does not add standalone MCP or CLI commands.

The configuration-path model now lives in `scp-core::config::ConfigPaths`, with
`McpConfigGetPathsResult` as its MCP result alias. `config_get_paths` still returns
only `appConfigPath`, ignores all arguments, and uses desktop path resolution and
errors. The existing UI command and resource keep their contracts. This adds no
CLI path command or path-resolution behavior to core.

SCP transfer MCP contracts share `McpScpTransferGetArgs`, `McpScpTransferGetResult`
and `McpScpTransferListResult`, backed by the unchanged core transfer models.
List returns the bare store-ordered array and ignores all arguments. Get accepts
any string `transferId` unchanged, returns the first exact match, and preserves
`transferId is required` / `transfer not found` errors. All ten result fields,
nullable values and lowercase states are unchanged. Desktop transfer execution,
policy, events and envelopes retain their behavior; no CLI transfer command is added.

Remote-browser list/stat MCP contracts now share `McpRemoteBrowserReadArgs`,
`McpRemoteBrowserListChildrenResult` and `McpRemoteBrowserStatPathResult`.
The entry/node models and five lowercase path kinds live in core, reexported
through the existing desktop model paths. Both tools require a string `browserId`;
missing or non-string `path` still becomes an empty string before desktop path
handling. Results keep their existing bare array/object fields. Session ownership,
SFTP execution, errors and policy remain in the desktop; no CLI browser is added.

Remote-browser open/close MCP contracts also live in core. Open reuses the
permissive server-ID argument contract and returns the unchanged `browserId` /
`rootPath` wire value; close requires a string `browserId` and returns `{}` on
success. The desktop model path reexports the shared session value, which owns
no connection. ID allocation, replacement-session cleanup, live SFTP ownership
and missing-session behavior remain in the desktop. No CLI open/close commands
are added.

Remote-browser create-directory/delete-path MCP contracts now share required
browser/path parsers and the existing `{}` success result in core. Both require a
string `browserId` before a string `path`, with unchanged required-field errors;
strings pass through unchanged. Delete preserves optional `recursive` booleans,
with missing or non-boolean values treated as absent. Desktop path handling,
SFTP execution and policy are unchanged; no CLI browser mutation is added.

The desktop dispatcher now uses `scp-core::mcp_policy` before every tool action,
including HTTP, direct Tauri MCP calls and resource reads delegated to tools.
Unknown operation names, sensitive-action policies or category entries reject the
call, including read-only calls. The supported policies are exactly
`require-ui-approval` and `allow-with-token`; supported categories are exactly
`read-only`, `runtime-control`, `config-mutation` and `file-transfer`. An enabled
category is still required under either policy. Sensitivity remains an explicit
list independent of discovery `readOnly` flags. There is no approval queue:
`require-ui-approval` blocks sensitive calls with an explicit error. Correct
unsupported settings locally through the Config page.

Persistence remains permissive. The desktop loader retains its existing defaults
for empty categories and blank policy; the pure validator itself applies no
normalization (empty categories deny all, and blank policy rejects). Discovery,
listener authentication and local desktop actions retain their existing behavior.

`sync_settings_save_partial` accepts a JSON object containing only `repoRoot`,
`openvpnExecutablePath`, `openvpnOvpnPath`, `boundaryCliPath` and `caddyFolder`.
MCP/CC authorization, tokens, listener, identity and enrollment controls must be
changed through desktop settings. Any other field (including unknown fields or
controls set to null) rejects the whole patch before saving. This applies to HTTP
and the Tauri MCP command, including with `allow-with-token`; desktop-local partial
settings updates retain their existing schema and behavior.

Detailed design docs live under `docs/mcp/`:

- `docs/mcp/MCP_IMPLEMENTATION_PLAN.md`
- `docs/mcp/MCP_TOOL_CATALOG.md`
- `docs/mcp/MCP_PHASED_CHECKLIST.md`
- `docs/mcp/MCP_SECURITY_MODEL.md`
- `docs/mcp/MCP_TRANSPORT_AND_AUTH.md`
- `docs/mcp/MCP_HELPER_CLI_SPEC.md`
- `docs/mcp/MCP_TOOL_SCHEMAS.md`
- `docs/mcp/MCP_RUST_MODULE_PLAN.md`
- `docs/mcp/MCP_FIRST_REFACTOR_SLICE.md`
- `docs/mcp/MCP_PR1_RUST_SCAFFOLDING.md`

Helper CLI note:

- first working helper binary lives at `src-tauri/src/bin/supercontrolpanelhelper.rs`
- it talks to the app's local MCP HTTP endpoint and can discover endpoint/token from `sync-local.json`

Pre-MCP refactor planning docs live under `docs/refactor/`:

- `docs/refactor/PRE_MCP_REFACTOR_CHECKLIST.md`
- `docs/refactor/BOUNDARY_SERVICE_EXTRACTION_PLAN.md`

## Feature details

### OpenVPN function

- One concrete function worker: `openvpn-connect`.
- Config page provides OpenVPN executable **Browse** + **Auto detect** helpers.
- OpenVPN runs via elevated helper process on Windows (UAC prompt).
- Once the elevated helper is already running on Windows, a later CC-triggered OpenVPN restart reuses that helper and restarts only the OpenVPN child process without requiring another UAC prompt.
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

### Encrypted settings backup

Config → **Export settings** creates a passphrase-encrypted `.scpbak` containing all three settings files and the app's known OS-keyring credentials, including per-server SSH passwords. Locked/unreadable credentials abort a full export. The passphrase must match its confirmation and cannot be recovered.

**Import settings** authenticates the backup and shows a redacted preview before confirmed replacement of selected sections. Destination paths are preserved by default, with optional remapping. Portable imports rotate MCP access and require CC re-enrollment; MCP and CC remain disabled. Stop sessions/transfers and close remote browsers first; disable MCP/CC and restart if CC has been running. Active work is never terminated automatically.

External certificate/key/OVPN/SSH-identity/Caddy files are referenced, not bundled. Boundary app tokens and telemetry queue keys remain encrypted in the archive but cannot be restored until their compatibility can be validated; the destination telemetry key stays intact, while a changed Boundary controller/auth identity clears the old app token for fresh login. Restore uses an encrypted crash-recovery journal. If recovery blocks startup, keep the journal/key, unlock the OS credential store and restart. See [backup scope, format and recovery](docs/mcp/SETTINGS_BACKUP.md) for details and platform qualification limits.

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
  - When Runtime Summary loads a running SSH status, the admin UI reattaches to the existing browser PTY or adopts the live local SSH PTY with recent history.
- Controls:
  - Start/Stop Tunnel.
  - Start/Stop SSH.
  - **RDP** (Windows servers only):
    - Enabled when a server’s OS is set to `windows`.
    - Launches the OS-native Remote Desktop client (e.g. `mstsc.exe` on Windows; `rdp://` URL via `open`/`xdg-open` elsewhere) targeting the server `host`.
  - **SCP Upload / Download**:
    - Works for direct SSH and for Boundary servers over the tunnel (connecting to `127.0.0.1` and the listen port).
    - Opens a two-pane remote browser modal: a lazy folder tree on the left and the current folder contents on the right.
    - The browser toolbar can now create remote folders and delete the currently selected remote file/folder using the active browser session.
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
    - SCP transfers launched through CC/admin also appear in the desktop Recent transfers history and bind their PTY session so progress and completion remain visible.
    - For Boundary servers, the app now checks that the tunnel is running before browse/transfer actions and shows a clearer error if it is not.
    - Upload flows normalize remote paths and confirm before overwriting known remote targets.
  - Remote browse groundwork:
    - Backend commands now expose the persistent browser session API (`open_remote_browser`, `close_remote_browser`, `list_remote_browser_children`, `stat_remote_browser_path`).
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

## Code intelligence

Use [Ripwire](https://github.com/redhat-et/ripwire) for local code navigation and impact analysis. It is a development tool, separate from the application runtime and npm dependencies.

Install the official prebuilt release on Linux or macOS (see [platform requirements](https://github.com/redhat-et/ripwire/blob/main/INSTALL.md)):

```bash
curl -fsSL https://raw.githubusercontent.com/redhat-et/ripwire/main/scripts/install.sh -o /tmp/ripwire-install.sh
# Review the downloaded script before running it.
RIPWIRE_REPO=redhat-et/ripwire bash /tmp/ripwire-install.sh
export PATH="$HOME/.local/bin:$PATH"
ripwire --version
```

The installer verifies the release checksum and activates skills for detected coding agents. Re-run it to update. The CLI works without MCP setup or advisory hooks. Windows contributors can run it from a Linux environment such as WSL.

From this repository:

```bash
ripwire . --for="server SSH session lifecycle" --legend=compact
ripwire . --impact="start_server_ssh" --legend=compact
ripwire . --uses="start_server_ssh" --legend=compact
```

Ripwire builds an incremental cache outside the repository automatically; no committed index or manual indexing step is needed. Follow AGENTS.md for impact review and the required TDD workflow.
