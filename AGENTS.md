# Agent guide

## Scope and working rules

Maintain the PTY-driven function runner: features launch/control PTYs and report runtime status to the UI.

- Keep changes focused; preserve Windows, macOS, and Linux behavior.
- Read relevant source and tests before editing. Prefer explicit errors; avoid heavy frameworks.
- Preserve command/event names and payloads unless updating every producer and consumer together, including desktop, CC/admin, and MCP integrations.
- Update `README.md` for user-facing changes and relevant design docs for contract/architecture changes. Keep this guide limited to working rules and critical invariants.

## Repository map

- `src/main.ts`: Vite/TypeScript/xterm.js UI, page navigation, PTY/event wiring.
- `src/styles.css`: layout and terminal styling.
- `src-tauri/src/main.rs`: Tauri v2 commands, Rust/portable-pty lifecycle, function orchestration.
- `src-tauri/src/mcp/`: localhost HTTP MCP transport, tools, resources; design docs in `docs/mcp/`.
- `crates/{scp-core,scp-mcp,scp-cli}/`: independent Rust workspace for shared runtime/backup core, desktop+stdio MCP adapters, and standalone `scpctl` CLI; agent guide and release docs in `docs/mcp/`.
- `scripts/{package-scpctl.mjs,install-scpctl.sh}`, `Dockerfile.cli`, `examples/standalone-cli/`: standalone CLI packaging, checksum-verified prefix installer, and headless workflow examples.
- `src-tauri/tauri.conf.json`: desktop build and window configuration.

Install: `npm install`. Develop: `npm run tauri dev`. Build: `npm run tauri build`.

## Required workflow

For every code change, including small fixes, follow strict TDD:

1. Write a test for the desired behavior; run it and confirm it fails for the expected reason before implementation.
2. Implement only enough to pass; rerun the test.
3. Refactor as needed, then run the relevant existing suite. Resolve regressions before finishing or committing.

Cover business logic, errors/edge cases, integration boundaries, and UI interactions; use E2E tests for critical flows where applicable. Never commit code without corresponding tests.

Test commands (run in the applicable package):

- Admin frontend: Vitest + React Testing Library, `npm test -- --run`; `*.test.ts` / `*.test.tsx`.
- NestJS backend: Jest, `npm test` (filter with `-- --testPathPattern=<file>`); `*.spec.ts`.
- Tauri backend: `cargo test` in `src-tauri`; `#[cfg(test)]` modules or `tests/`.
- Standalone CLI workspace: `cargo test --locked -p scp-core -p scp-mcp -p scp-cli`, `scp-mcp` default vs `--all-features`, and `node --test experiments/cli-stdio-slice0/tests/slice{1_production_stdio,4_production_stdio,5_release_qualification}.mjs`.

Use Ripwire from the repository root before broad code searches/whole-file reads:

- Orient: `ripwire . --for="<task>" --legend=compact`.
- Inspect: `ripwire . --expand="<symbol>" --legend=compact`.
- Before editing an existing symbol: run `--impact="<symbol>"` and `--uses="<symbol>"`; report callers, affected paths, and risk. Warn before high-risk edits.
- Select tests: `--affected="<symbol>"`.
- Before committing: inspect `git diff` and run `ripwire . --quality-delta --legend=compact`.

Static analysis is approximate; verify dynamic dispatch and command/event contracts in source and tests. Setup: README “Code intelligence”.

## Runtime invariants

- PTY commands: `open_pty`, `write_pty`, `resize_pty`, `close_pty`; events: `pty-output`, `pty-exit`, `openvpn-log`, `function-status`. Preserve existing payloads.
- Function IDs: `openvpn-connect`, `boundary-auth`, `server-tunnel-{serverId}`, `server-ssh-{serverId}`.
- `SessionStore` owns active PTYs; `FunctionStore` owns runtime status. Reader threads emit output; exit monitoring updates both PTY and function status. Keep locks short and avoid overlapping locks.
- Status starts from `list_function_statuses`, then follows events. Hide deleted-server statuses and stopped server sessions. Cards navigate to their owning controls; Boundary status identifies App/CLI token source.
- Preserve each server’s tunnel/SSH sessions and scrollback when switching rows. `request_attach_server_ssh` adopts an existing local PTY after admin reload.
- Windows OpenVPN uses an elevated helper and `openvpn-log`; macOS/Linux use a sudo PTY and `pty-output`. Password prompts receive six-digit TOTP + password.
- Boundary supports browser/password PTY auth and automated OIDC via HTTP + hidden webview. Target search matches name, ID, and description; server search stays local.
- Remote browsing reuses one persistent SFTP session per modal. Close on modal dismissal, server switch, navigation away, tunnel stop/restart, server edit/delete, or replacement browser session.
- SCP picker enforces transfer type: upload destination folder, download file, or download folder. Boundary transfers require a running tunnel. Normalize upload paths and confirm known overwrites. Preserve PTY progress/history and `scp-transfer-status` for remotely initiated transfers.

## Configuration and security

- Shared `config.json` / `servers.json` sync only to `app-config/` in the configured Git repo.
- Device-local settings stay in `sync-local.json`: executable/OVPN paths, Boundary CLI path, Caddy folder, sync root, enrollment, and MCP settings.
- OpenVPN/Boundary passwords and OTP secrets belong in the OS keyring; migrate legacy plaintext on load. `save_app_config` must never write these secrets to disk.
- `save_sync_settings` is a partial patch: omitted fields retain their values.
- First-run enrollment saves CC credentials locally, enables CC, and connects its WebSocket. Both enrollment and skip mark `firstRunCompleted`.
- HTTP MCP binds localhost only and requires bearer auth on every route. Redact secrets/auth tokens. Under `require-ui-approval`, reject sensitive HTTP tools.
- MCP routes: GET `/health`, `/mcp/status`, `/mcp/tools`, `/mcp/resources`; POST `/mcp/tools/call`, `/mcp/resources/read`. Consult source/design docs before changing transport behavior.
- Standalone `scpctl` keeps an isolated profile directory (`SCPCTL_CONFIG_DIR`, separate from desktop profiles) with single-writer `owner.lock`, fail-closed live-reloaded `stdio-policy.json`, and `StrictHostKeyChecking=yes` SSH/SFTP/SCP checks.
