# AGENTS.md

Guidance for AI agents working in this repository.

Detailed design docs for MCP work live under `docs/mcp/`.
The repo now also contains a minimal localhost HTTP MCP implementation in `src-tauri/src/mcp/`.

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
- `src-tauri/src/mcp/`: minimal localhost HTTP MCP runtime, tool dispatch, resources, and transport glue
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
- `is_first_run() -> bool`
- `enroll_device({ serverUrl, bootstrapToken, deviceName? }) -> { deviceId, ccAgentToken, deviceName }`
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
  - `request_attach_server_ssh` control-plane message adopts an existing local SSH PTY with `{ serverId, sessionId, localSessionId, cols, rows }` so admin UI reloads can attach to live local sessions.
  - `stop_server_ssh({ serverId })`
  - `start_server_rdp({ serverId }) -> void`
  - New remote browser contract used by the picker:
    - `open_remote_browser({ serverId }) -> { browserId, rootPath }`
    - `close_remote_browser({ browserId }) -> void`
    - `list_remote_browser_children({ browserId, path }) -> RemoteBrowseEntry[]`
    - `stat_remote_browser_path({ browserId, path }) -> RemoteBrowseNodeInfo`
    - `create_remote_browser_directory({ browserId, path }) -> void`
    - `delete_remote_browser_path({ browserId, path, recursive? }) -> void`
    - Backed by a persistent SFTP session while the modal is open.
    - Backend cleanup rules:
      - opening a new browser session closes any previously leaked browser session
      - stopping/restarting a server tunnel closes browser sessions for that server
      - editing or deleting a server closes browser sessions for that server
  - `scp_upload({ serverId, localPath, remotePath, recursive? }) -> string`
  - `scp_download({ serverId, remotePath, localPath, recursive? }) -> string`
  - `start_scp_upload({ serverId, localPath, remotePath, recursive? }) -> string` (PTY-backed session id for progress-aware transfers)
  - `start_scp_download({ serverId, remotePath, localPath, recursive? }) -> string` (PTY-backed session id for progress-aware transfers)
  - `scp-transfer-status` desktop event mirrors CC/admin-started SCP transfer lifecycle with `{ transferId, sessionId?, state, direction, serverId, localPath, remotePath, phase, message?, reasonCode?, reasonDetail? }` so the app can show Recent transfers even when the action started remotely.
  - Function IDs:
    - OpenVPN: `openvpn-connect`
    - Boundary auth: `boundary-auth`
    - Server tunnel: `server-tunnel-{serverId}`
    - Server SSH: `server-ssh-{serverId}`

- **Device enrollment (first-run onboarding)**
  - `is_first_run()` checks `firstRunCompleted` in `sync-local.json` (defaults to false).
  - On first launch, the app shows an **enrollment overlay** with fields for CC Server URL, Bootstrap Token, and optional Device Name.
  - `enroll_device({ serverUrl, bootstrapToken, deviceName? })` POSTs to `{serverUrl}/v1/enroll`, saves the returned `ccDeviceId` / `ccAgentToken` to `sync-local.json`, enables CC, marks `firstRunCompleted`, and initiates the CC WebSocket connection.
  - Users can skip enrollment; clicking "Skip for now" sets `firstRunCompleted: true` without enrolling.
  - On subsequent launches, `is_first_run()` returns false and the overlay is not shown.

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

## MCP notes

- Current transport is a minimal localhost HTTP listener, not a full spec-polished MCP transport.
- Runtime/config is driven by device-local `sync-local.json` MCP fields.
- HTTP MCP currently exposes:
  - `GET /health`
  - `GET /mcp/status`
  - `GET /mcp/tools`
  - `GET /mcp/resources`
  - `POST /mcp/tools/call`
  - `POST /mcp/resources/read`
- Bearer token is required for all HTTP MCP requests.
- Backend currently enforces localhost-only bind addresses.
- When MCP sensitive-action policy is `require-ui-approval`, sensitive HTTP tools are rejected instead of executed.
- MCP config-related outputs redact secrets and the MCP auth token.

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

## Test-Driven Development (TDD) - MANDATORY

**All incremental changes MUST follow strict TDD practices. No exceptions.**

### TDD Workflow (Red-Green-Refactor)

1. **RED: Write the test first**
   - Write a failing test that describes the desired behavior
   - Run the test to confirm it fails for the right reason
   - Do NOT write any implementation code yet

2. **GREEN: Make it pass**
   - Write the minimal code needed to make the test pass
   - Run the test to confirm it passes
   - Do not add extra features or "nice-to-haves"

3. **REFACTOR: Clean up**
   - Improve code quality without changing behavior
   - Run tests to ensure they still pass
   - Update documentation if needed

### Mandatory Rules

- **NEVER write implementation code before writing the test**
- **NEVER skip writing tests** for any code change, no matter how small
- **NEVER commit code** without corresponding tests
- **ALWAYS run tests** before and after making changes
- **ALWAYS ensure** all existing tests still pass

### What Requires Tests

- New functions, methods, or components
- Bug fixes (write a test that reproduces the bug first)
- API endpoints and handlers
- State management logic
- UI interactions and user flows
- Edge cases and error handling
- Integration points between modules

### Test Coverage Requirements

- **Unit tests**: All business logic, utilities, and pure functions
- **Integration tests**: API endpoints, database operations, external service calls
- **Component tests**: React/UI components with user interactions
- **E2E tests**: Critical user flows (when applicable)

### Testing Stack

**Frontend (admin-next):**
- Framework: Vitest + React Testing Library
- Run: `npm test` or `npm test -- --run`
- Location: `*.test.tsx` or `*.test.ts` files

**Backend (NestJS):**
- Framework: Jest
- Run: `npm test` or `npm test -- --testPathPattern=<file>`
- Location: `*.spec.ts` files

**Rust (Tauri):**
- Framework: Built-in Rust testing
- Run: `cargo test`
- Location: `#[cfg(test)]` modules or `tests/` directory

### Example TDD Flow

```
# 1. RED - Write failing test
$ vim src/api/adminApi.test.ts
# Add test for requestRestartOpenvpn()
$ npm test
# ❌ Test fails - function doesn't exist

# 2. GREEN - Implement minimal code
$ vim src/api/adminApi.ts
# Add requestRestartOpenvpn() function
$ npm test
# ✅ Test passes

# 3. REFACTOR - Clean up if needed
$ vim src/api/adminApi.ts
# Extract common patterns, improve naming
$ npm test
# ✅ Tests still pass
```

### Enforcement

- Code reviews will reject PRs without tests
- CI/CD pipeline will fail if tests don't pass
- Agents must verify test coverage before completing tasks
- If you're tempted to skip tests, **STOP** and write them first

### Benefits of Strict TDD

- Catches bugs early in development
- Provides living documentation
- Enables confident refactoring
- Reduces debugging time
- Improves code design and modularity
- Makes code review easier

**Remember: Tests are not optional. They are part of the implementation.**

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
    - SCP Upload/Download, including recursive folder copy with `scp -r` (works for both direct SSH and Boundary via the tunnel).
    - SCP path selection now uses a two-pane remote browser modal with a lazy folder tree, current-folder contents, breadcrumb navigation, parent navigation, refresh, and manual path fallback.
    - The browser toolbar supports creating remote folders and deleting the currently selected remote file/folder through the active browser session.
    - The picker now opens one backend remote-browser session per modal instance and reuses it for repeated navigation until the modal closes, the selected server changes, or the user navigates away from Servers.
    - Picker mode is transfer-aware: upload flows accept destination folders only, download-file accepts remote files only, and download-folder accepts remote folders only; valid items are single-click selectable and folders support double-click open.
    - SCP feedback is shown as a short in-session transfer history with running, success, and error entries instead of a single transient line.
    - SCP transfers can now run through PTY-backed execution so the transfer history can show progress bars and reported percentages.
    - Boundary-backed SCP actions require the tunnel to be running first; upload flows normalize remote paths and confirm before overwriting known remote targets.
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

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **SuperControlPanel** (5476 symbols, 13156 relationships, 300 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## When Debugging

1. `gitnexus_query({query: "<error or symptom>"})` — find execution flows related to the issue
2. `gitnexus_context({name: "<suspect function>"})` — see all callers, callees, and process participation
3. `READ gitnexus://repo/SuperControlPanel/process/{processName}` — trace the full execution flow step by step
4. For regressions: `gitnexus_detect_changes({scope: "compare", base_ref: "main"})` — see what your branch changed

## When Refactoring

- **Renaming**: MUST use `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` first. Review the preview — graph edits are safe, text_search edits need manual review. Then run with `dry_run: false`.
- **Extracting/Splitting**: MUST run `gitnexus_context({name: "target"})` to see all incoming/outgoing refs, then `gitnexus_impact({target: "target", direction: "upstream"})` to find all external callers before moving code.
- After any refactor: run `gitnexus_detect_changes({scope: "all"})` to verify only expected files changed.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Tools Quick Reference

| Tool | When to use | Command |
|------|-------------|---------|
| `query` | Find code by concept | `gitnexus_query({query: "auth validation"})` |
| `context` | 360-degree view of one symbol | `gitnexus_context({name: "validateUser"})` |
| `impact` | Blast radius before editing | `gitnexus_impact({target: "X", direction: "upstream"})` |
| `detect_changes` | Pre-commit scope check | `gitnexus_detect_changes({scope: "staged"})` |
| `rename` | Safe multi-file rename | `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` |
| `cypher` | Custom graph queries | `gitnexus_cypher({query: "MATCH ..."})` |

## Impact Risk Levels

| Depth | Meaning | Action |
|-------|---------|--------|
| d=1 | WILL BREAK — direct callers/importers | MUST update these |
| d=2 | LIKELY AFFECTED — indirect deps | Should test |
| d=3 | MAY NEED TESTING — transitive | Test if critical path |

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/SuperControlPanel/context` | Codebase overview, check index freshness |
| `gitnexus://repo/SuperControlPanel/clusters` | All functional areas |
| `gitnexus://repo/SuperControlPanel/processes` | All execution flows |
| `gitnexus://repo/SuperControlPanel/process/{name}` | Step-by-step execution trace |

## Self-Check Before Finishing

Before completing any code modification task, verify:
1. `gitnexus_impact` was run for all modified symbols
2. No HIGH/CRITICAL risk warnings were ignored
3. `gitnexus_detect_changes()` confirms changes match expected scope
4. All d=1 (WILL BREAK) dependents were updated

## Keeping the Index Fresh

After committing code changes, the GitNexus index becomes stale. Re-run analyze to update it:

```bash
npx gitnexus analyze
```

If the index previously included embeddings, preserve them by adding `--embeddings`:

```bash
npx gitnexus analyze --embeddings
```

To check whether embeddings exist, inspect `.gitnexus/meta.json` — the `stats.embeddings` field shows the count (0 means no embeddings). **Running analyze without `--embeddings` will delete any previously generated embeddings.**

> Claude Code users: A PostToolUse hook handles this automatically after `git commit` and `git merge`.

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
