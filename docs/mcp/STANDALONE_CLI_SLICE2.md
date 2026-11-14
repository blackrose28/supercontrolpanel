# Standalone CLI Slice 2: Runtime Worker Families and Direct CLI Workflows Report

Status: **Complete for Linux `x86_64` Slice 2 gate** (Slice 3 config/backup CLI workflows, Slice 4 stdio MCP exposure of extracted workers and SFTP, Slice 5 release packaging/GUI-free Linux CI runner validation, Windows and macOS native qualification, live privileged `openvpn` route tunnel testing, and interactive Browser OIDC validation remain open).

- Baseline commit: `c8e8add` (Slice 1)
- Host target: Linux `x86_64`
- Observed toolchain & client packages:
  - `rustc 1.100.0 (0ef1bc386 2026-11-09)`
  - `cargo 1.100.0 (3acc79c0c 2026-11-05)`
  - Node `v26.13.0`
  - `@modelcontextprotocol/sdk 1.32.2` (`2025-11-25`) and `@modelcontextprotocol/client 2.4.0` (`2026-07-28`)

## Architecture & runtime families delivered

1. **Headless `scp-core` runtime worker extraction (`crates/scp-core/src/`):**
   - `supervisor.rs`: `ProcessSupervisor` over pipe (`SpawnMode::Pipe`) and `portable-pty` PTY (`SpawnMode::Pty`) children, bounded 64 KiB output ring (`MAX_LOG_RING_BYTES`), `CancelToken` + `AcceptOutcome` cancel-before-accept race prevention, exit hooks (`on_exit`), and dependency-ordered `shutdown_all`.
   - `tty.rs`: `is_interactive_tty()`, RAII `TerminalModeGuard` restoring `termios`/console attributes, `SignalWatcher` + `TerminationSignal` (`Interrupt` / `Terminate`), and bounded `terminate_child`.
   - `events.rs`, `transfer_store.rs`, and `file_paths.rs`: host-neutral `EventSink` (`DomainEvent`, `NoopEventSink`, `BufferedEventSink`), `ScpTransferStore` (`MAX_TRANSFER_HISTORY`, progress parsing and transfer lifecycle helpers), and `FileOpPathError` / cross-platform local & remote path normalization.
   - `ssh_ops.rs` and `sftp_ops.rs`: server SSH endpoint resolution (`resolve_server_ssh_endpoint`), Boundary local tunnel TCP listener + usable-port readiness checks, temporary one-shot `TunnelLease` with automatic drop/teardown, strict OpenSSH `known_hosts` verification (`verify_ssh_host_key`, `BatchMode=yes` / `StrictHostKeyChecking=yes` with zero implicit trust/password prompts), synchronous and asynchronous SCP upload/download, and `RemoteBrowserStore` persistent SFTP sessions (`open`, `list_children`, `stat_path`, `create_directory`, `delete_path`, `close`) closed automatically on parent tunnel stop or runtime shutdown.
   - `boundary_ops.rs`: isolated CLI environment (`BOUNDARY_SKIP_CACHE_DAEMON=true`, `BOUNDARY_TOKEN_NAME=scpctl-{ns}`), bounded target search (`parse_boundary_targets`, matching `name`, `id`, and `description`), password auth token capture into namespaced `profile:{ns}:boundary_app_token` (with opt-in `SCPCTL_TEST_KEYRING_FILE` JSON test backend in `secrets.rs` for headless subprocess testing), and explicit `CAPABILITY_UNAVAILABLE` gates for `AutomatedOidc` (`webview_oidc_desktop_only`) and unverified `BrowserOidc` (`browser_oidc_unverified`).
   - `openvpn_ops.rs`: profile-free executable discovery (`detect_openvpn_executable_checked`), normalized command building and SHA-1 6-digit TOTP + password generation (`generate_totp_code_with_offset`, `generate_openvpn_password_with_offset`), `terminateOpenVpnWhenHelperExits` ownership rejection (`incompatible_vpn_ownership`), `sudo -n true` noninteractive preflight (`elevation_interaction_required` / `elevation_unavailable`), `OpenVpnRuntimeOverrides`, and cross-profile host-wide `vpn.lock` contention (`CONFLICT`, exit `5`, `vpn_owner_conflict`).
   - `caddy_ops.rs`: `Caddyfile` and folder validation, readiness log probe, and foreground `caddy-run` supervision.
2. **Shared `scp-mcp` standalone schemas & capability surface (`crates/scp-mcp/src/standalone.rs`):**
   - Extended `standalone_schema_catalog()` to 17 CLI command schemas (8 Slice 1 + 9 Slice 2 across `slice1_discovery_schemas`, `slice1_profile_schemas`, `slice2_tunnel_vpn_schemas`, `slice2_ssh_scp_schemas`).
   - Updated `capabilities_report()` to report `"runtimeFamilies"` (`"openvpn": "cli_foreground"`, `"boundary": "cli_foreground_and_query"`, `"caddy": "cli_foreground"`, `"ssh": "cli_interactive_pty"`, `"scp": "cli_one_shot"`) and narrow `"unavailableFamilies"` (`"sftp": "slice4_planned"`, `"backup": "slice3_planned"`).
   - Preserved the Slice 1 stdio MCP boundary (`11` read-only tools `available: true`, `34` mutating/worker ops `available: false` with `runtime_not_extracted` / `interactive_host_required` / `protected_import` / `reviewed_git`, `6` `scp://*` resources, and `9` default permitted ops under `stdio-policy.json`) until Slice 4.
3. **Desktop `src-tauri` shared delegations:**
   - Delegated pure helpers and constants in `src-tauri/src/{main.rs,commands/boundary.rs,commands/openvpn.rs,models/boundary.rs,models/remote_browser.rs,services/automation_service.rs,services/config_service.rs,services/file_op_path_service.rs,services/remote_browser_service.rs,services/scp_transfer_service.rs,services/server_service.rs,stores/scp_transfer_store.rs}` to `scp-core` without changing any Tauri command/event payload, desktop REST helper route, or `/mcp` contract.

## Standalone `scpctl` Slice 2 commands & runtime behavior

- **Implemented Slice 2 runtime subcommands (`crates/scp-cli`):**
  - `scpctl openvpn detect [--json]` — profile-free local binary detection (never creates a profile directory).
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] openvpn start --foreground [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] boundary targets search [--filter <TEXT>] [--limit <N>] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] boundary auth [--interactive] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] caddy start --foreground [--folder <PATH>] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] server tunnel start --server-id <ID> --foreground [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] server ssh --server-id <ID> --interactive`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] scp upload --server-id <ID> --local <PATH> --remote <PATH> [--recursive] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] scp download --server-id <ID> --remote <PATH> --local <PATH> [--recursive] [--overwrite] [--json]`
- **Foreground readiness & signal contract (`runtime_cmds.rs`):**
  - Readiness messages (`Started <function_id> (session <session_id>)`) emit strictly on `stderr`; `stdout` emits only the single final JSON envelope when `--json` is passed.
  - `SIGTERM` stops the supervised child/runtime cleanly, runs ordered `shutdown()`, and exits `0` with `{ "ok": true, "data": { "state": "stopped", ... } }`.
  - `SIGINT` stops the supervised child/runtime cleanly and exits `130` (`INTERRUPTED`).
- **Interactive SSH TTY enforcement:** `server ssh --interactive` fails closed with `CAPABILITY_UNAVAILABLE` (exit `4`, reason `interactive_tty_required`) when stdin/stdout is not a TTY, and restores original `termios` flags via `TerminalModeGuard` on exit.

## Verification ledger (Linux `x86_64`)

| Check | Command | Observed result |
| --- | --- | --- |
| Workspace formatting | `cargo fmt --check` | Exited `0` |
| Workspace lint | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Exited `0`, `0` warnings |
| Core, MCP & CLI tests | `cargo test --locked -p scp-core -p scp-mcp -p scp-cli` | Exited `0` (`61 passed; 0 failed`: `scp-core` `31` [`1` unit + `9` `slice1_core` + `1` `slice2_async_scp` + `4` `slice2_boundary` + `2` `slice2_caddy` + `4` `slice2_openvpn` + `4` `slice2_ssh_scp_sftp` + `6` `slice2_supervisor`], `scp-mcp` `12` [`5` unit + `2` `ledger` + `5` `slice1_standalone`], `scp-cli` `18` [`9` `cli_slice1` + `6` `cli_slice2` + `3` `discovery`]) |
| `scp-mcp` all features | `cargo test --locked -p scp-mcp --all-features` | Exited `0` (`12 passed; 0 failed`) |
| Ledger & operation doc contract | `cargo test --locked -p scp-mcp --test ledger && cargo test --locked -p scp-mcp` | Exited `0` (`2 passed; 0 failed` in `ledger`; `11 passed; 0 failed` default features) |
| Release binary & dep purity | `cargo build --locked --release -p scp-cli && cargo tree --locked -p scp-cli -e normal,build` | Exited `0`; zero `tauri`, `wry`, `tao`, `webkit2gtk`, `gtk`, `glib`, `libdbus-sys`, `reqwest`, `axum`, or `hyper` dependencies |
| Production `scpctl` stdio E2E | `node --test --test-reporter=spec experiments/cli-stdio-slice0/tests/slice1_production_stdio.mjs` | Exited `0` (`3 passed; 0 failed` across `2025-11-25`, `2026-07-28`, redaction, default-deny, NDJSON purity, EOF exit `0`, lock conflict exit `5`) |
| Desktop leaf formatting | `rustfmt --edition 2021 --check` on the 11 modified leaf `src-tauri/src/{commands,models,services,stores}` files | Exited `0` |
| Ripwire quality delta | `ripwire . --quality-delta --legend=compact` | `0` `gating="1"` findings in `crates/` and `src-tauri/` (only 2 pre-existing rows in untracked `cc-server/`) |

## Environment-blocked checks on this host & remaining scope

1. **Environment-blocked checks on this minimal headless offline Linux container (not run here):**
   - `cargo test --manifest-path src-tauri/Cargo.toml`: blocked because system GTK/GLib development pkg-config (`glib-2.0.pc`) is not installed.
   - `npm test --prefix experiments/cli-stdio-slice0` (disposable Slice 0 `scp-cli-stdio-slice0` spike binary): blocked because `experiments/cli-stdio-slice0` compiles `dbus-secret-service v4.1.0` -> `libdbus-sys v0.2.7`, which requires system `dbus-1.pc` (`libdbus-1-dev`) not installed on this container (`production scpctl` does not depend on `libdbus-sys` and passed `slice1_production_stdio.mjs` `3/3`).
   - Root `npm run build` (`tsc && vite build`) and `npx playwright test`: blocked because root `node_modules/` is absent in this offline workspace (no frontend TypeScript/CSS files were modified in Slice 2).
2. **Remaining planned scope:**
   - Slice 3: config/server mutations, safe noninteractive secret input, and encrypted `.scpbak` CLI export/preview/apply.
   - Slice 4: stdio MCP exposure and qualification of extracted runtime workers and SFTP browser tools.
   - Slice 5 & OS matrix: release packaging, GUI-free Linux CI runner validation without Secret Service, Windows and macOS native builds/keyrings/PTY/elevation, live privileged `openvpn` route tunnel smoke, and interactive Browser OIDC validation in [`STANDALONE_CLI_STDIO_MCP_PLAN.md`](./STANDALONE_CLI_STDIO_MCP_PLAN.md).
