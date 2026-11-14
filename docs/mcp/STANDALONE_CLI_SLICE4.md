# Standalone CLI Slice 4: Full stdio MCP Workflows and Qualification Report

Status: **Complete for Linux `x86_64` Slice 4 gate** (Slice 5 release packaging/helper migration documentation, GUI-free Linux CI runner validation without Secret Service, Windows and macOS native qualification, live privileged `openvpn` route tunnel testing, and interactive Browser OIDC validation remain open).

- Baseline commit: `9877efc` (Slice 3)
- Host target: Linux `x86_64`
- Observed toolchain & client packages:
  - `rustc 1.100.0 (0ef1bc386 2026-11-09)`
  - `cargo 1.100.0 (3acc79c0c 2026-11-05)`
  - Node `v24.14.0`
  - `@modelcontextprotocol/sdk 1.32.2`, `@modelcontextprotocol/client 2.4.0`, and `@modelcontextprotocol/inspector 2.14.0`

## Architecture & capabilities delivered

1. **Headless runtime parity in `scp-core` (`crates/scp-core/src/`):**
   - Extended `events.rs` (`FanoutEventSink`), `runtime.rs`, `config_ops.rs`, `ssh_ops.rs`, `sftp_ops.rs`, `boundary_ops.rs`, `openvpn_ops.rs`, and `caddy_ops.rs` so the persistent standalone runtime executes tunnel, SSH, SCP/SFTP persistent browser, Boundary, OpenVPN, Caddy, and config/server/credential/sync-push workflows with cooperative cancellation and isolated profile/keyring boundaries (`crates/scp-core/tests/slice4_stdio_runtime.rs`).
   - Added `crates/scp-core/examples/sshd_fixture_daemon.rs` (userspace `russh` SSH/SFTP/SCP server fixture) so integration and Node E2E suites verify real SSH tunnels, SFTP persistent sessions, SCP upload/download, and cancellation on hosts where `/etc/hosts.deny` tcpwrappers blocks loopback `/usr/sbin/sshd`.
2. **Full 41/45 stdio operation dispatch & bounded lifecycle (`crates/scp-mcp/src/`):**
   - `contracts.rs`: Added `OperationError::for_host`, exported `is_standalone_stdio_available` and `standalone_unavailable_reason`, and enabled all `41` `Capability::Runtime` operations on standalone stdio via `authorize_checked`.
   - `lifecycle.rs`: Preserved `TransportControl::default()` for desktop HTTP and added `TransportControl::standalone_from_env()`, `current_cancel_token()`, and `WorkerCancelGuard` + cancellation watcher bridging tokio future cancellation/drop into `scp_core::supervisor::CancelToken`.
   - `standalone.rs`: Exported `execute_standalone_cancellable`, implemented all `41` standalone `Capability::Runtime` handlers, enforced `protect_standalone_config_replacement`, added live `reload_stdio_policy()` from `stdio-policy.json` (failing closed with `availableOperations: []` in `capabilities_report` when `stdio-policy.json` is corrupt), and updated `capabilities_report` to publish `"sftp": "stdio_persistent_browser"` in `"runtimeFamilies"` with `"unavailableFamilies": {}`.
   - `protocol.rs`: Updated `SdkHost::standalone` to use `TransportControl::standalone_from_env()`, subscribe core `add_event_observer` updates for MCP resource/tool notifications, reload `stdio-policy.json` live per request/list, bind `current_cancel_token()`, trigger mutation invalidations, and surface host-neutral errors via `OperationError::for_host`.
   - `stdio.rs`: Ensured `runtime.shutdown(Duration::from_secs(2))` runs on every return path of `serve_stdio_io` (EOF, `SIGTERM`, `SIGINT`, malformed framing/line-limit close, and stalled stdout write timeout).

## Standalone stdio operation & environment matrix

- **Operation availability on `scpctl mcp serve --stdio` (`45` registry operations total):**
  - **`41` `Capability::Runtime` operations exposed:**
    - **Default `StdioPolicy::default()` (`read-only` mode, `["read-only"]` category): `11` operations** (Slice 1 read-only queries plus `detect_openvpn_executable` and `boundary_search_targets`).
    - **With `"file-transfer"` category added (`read-only` mode): `15` operations** (`11` default plus `scp_transfer_list`, `scp_transfer_get`, `remote_browser_list_children`, and `remote_browser_stat_path`).
    - **With `allow-local-policy` mode and all `4` categories enabled: `41` operations** covering `server_*`, `config_*` (excluding sync/git pulls), `sync_settings_save_partial`, `config_sync_push`, `boundary_*`, `openvpn_*`, `caddy_*`, `remote_browser_*`, `scp_*`, `function_status_*`, `app_get_overview`, and `servers_list`.
  - **`4` excluded operations (`CAPABILITY_UNAVAILABLE` on standalone stdio):**
    - `server_rdp_start` (interactive GUI desktop client launch)
    - `config_sync_pull` (protected remote sync import unavailable in standalone mode)
    - `config_git_pull` (reviewed Git pull unavailable in standalone mode)
    - `config_git_push` (reviewed Git push unavailable in standalone mode)
  - **`6` MCP resources (`scp://*`):** `scp://status/functions`, `scp://status/openvpn`, `scp://status/boundary`, `scp://servers/list`, `scp://config/paths`, and `scp://status/transfers`.
- **Transport environment knobs (`lifecycle.rs` & `stdio.rs`):**
  - `SCP_STDIO_MAX_WORKERS` / `SCP_STDIO_MAX_CONCURRENT` (bounded worker concurrency semaphore; clamped `1..=64`, default `16`)
  - `SCP_STDIO_MAX_SUBSCRIPTIONS` (bounded subscription stream semaphore; clamped `1..=64`, default `8`)
  - `SCP_STDIO_MAX_LINE_BYTES` (maximum NDJSON input frame byte length)
  - `SCP_STDIO_WRITE_TIMEOUT_MS` (stdout stalled-reader write deadline triggering bounded runtime shutdown)

## Platform & client qualification matrix

| Target / client | Status | Notes & evidence |
| --- | --- | --- |
| Linux `x86_64` (`scp-core`, `scp-mcp`, `scp-cli` unit & integration suites) | **PASS** | Uses userspace `russh` `sshd_fixture_daemon` for real SSH/SFTP/SCP transfers because loopback `/usr/sbin/sshd` is blocked by container `/etc/hosts.deny` tcpwrappers |
| Linux `x86_64` `@modelcontextprotocol/sdk 1.32.2` (`2025-11-25`) | **PASS** | Validated in `slice1_production_stdio.mjs` & `slice4_production_stdio.mjs` (41 workflows, live policy reload, fail-closed policy errors, notifications, cancellation, worker saturation, write-timeout & signal reaping) |
| Linux `x86_64` `@modelcontextprotocol/client 2.4.0` (`2026-07-28`) | **PASS** | Validated tool/resource discovery, calling, and shutdown against release `scpctl mcp serve --stdio` |
| Linux `x86_64` `@modelcontextprotocol/inspector 2.14.0` CLI | **PASS** | Validated modern and legacy CLI discovery (`tools/list`, `resources/list`) and tool invocation |
| macOS (`aarch64` / `x86_64`) | **NOT RUN IN THIS ENVIRONMENT** | No macOS host/runner available in this container |
| Windows (`x86_64`) | **NOT RUN IN THIS ENVIRONMENT** | No Windows host/runner or UAC elevation helper available in this container |

## Verification ledger (Linux `x86_64`)

| Check | Command | Observed result |
| --- | --- | --- |
| Workspace formatting | `cargo fmt --check` | Exited `0` |
| Workspace lint | `cargo clippy --locked -p scp-core -p scp-mcp -p scp-cli --all-targets --all-features -- -D warnings` | Exited `0`, `0` warnings |
| Core, MCP & CLI tests | `cargo test --locked -p scp-core -p scp-mcp -p scp-cli` | Exited `0` (`87 passed; 0 failed`: `scp-core` `44` [`1` unit + `9` `slice1_core` + `1` `slice2_async_scp` + `4` `slice2_boundary` + `2` `slice2_caddy` + `4` `slice2_openvpn` + `4` `slice2_ssh_scp_sftp` + `6` `slice2_supervisor` + `7` `slice3_backup_and_config` + `6` `slice4_stdio_runtime`], `scp-mcp` `18` [`5` unit + `2` `ledger` + `6` `slice1_standalone` + `5` `slice4_standalone_stdio`], `scp-cli` `25` [`9` `cli_slice1` + `6` `cli_slice2` + `7` `cli_slice3` + `3` `discovery`]) |
| `scp-mcp` feature matrix & ledger | `cargo test --locked -p scp-mcp --all-features && cargo test --locked -p scp-mcp && cargo test --locked -p scp-mcp --test ledger` | Exited `0` (`18 passed; 0 failed` with `--all-features`, `16 passed; 0 failed` default features, `2 passed; 0 failed` in `ledger`) |
| Release builds & dep purity | `cargo build --locked --release -p scp-cli && cargo build --locked --release -p scp-core --example sshd_fixture_daemon && cargo tree --locked -p scp-cli -e normal,build` | Exited `0`; zero `tauri`, `wry`, `tao`, `webkit2gtk`, `gtk`, `glib`, `libdbus-sys`, `reqwest`, `axum`, or `hyper` dependencies |
| Release `scpctl` stdio E2E | `SCPCTL_BIN="target/release/scpctl" SSHD_FIXTURE_BIN="target/release/examples/sshd_fixture_daemon" node --test tests/slice1_production_stdio.mjs tests/slice4_production_stdio.mjs` | Exited `0` (`8 passed; 0 failed`: `3` Slice 1 + `5` Slice 4 tests covering `1.32.2`, `2.4.0`, and `2.14.0` modern/legacy CLI in `9.61s`) |
| Ripwire quality delta | `ripwire . --quality-delta --legend=compact` | Zero complexity, verbosity, params, duplication, or new-clone `gating="1"` findings in `crates/` or `experiments/` (only `2` untouched `cc-server/*` rows and `2` expected `short-horizon-churn` rows on `capabilities_report` and `cli_schema_and_capabilities_slice2_surface`) |

## Environment-blocked checks on this host & remaining scope

1. **Environment-blocked checks on this minimal headless offline Linux container (not run here):**
   - `cargo test --manifest-path src-tauri/Cargo.toml`: blocked because system GTK/GLib development pkg-config (`glib-2.0.pc`) is not installed (`src-tauri/` was untouched in Slice 4).
   - `npm test --prefix experiments/cli-stdio-slice0` (disposable Slice 0 `scp-cli-stdio-slice0` binary): blocked because `dbus-secret-service` / `libdbus-sys` requires system `dbus-1.pc` not installed on this container (`production scpctl` does not depend on `libdbus-sys` and passed `slice1_production_stdio.mjs` + `slice4_production_stdio.mjs` `8/8`).
   - Root `npm run build` and `npx playwright test`: blocked because root `node_modules/` is absent in this offline workspace (no frontend files were modified in Slice 4).
   - macOS (`aarch64`/`x86_64`) and Windows (`x86_64`) native targets: **NOT RUN IN THIS ENVIRONMENT** (no macOS or Windows runners available).
2. **Remaining planned scope:**
   - Slice 5 & OS matrix: release packaging, helper migration documentation, GUI-free Linux CI runner validation without Secret Service, Windows and macOS native builds/keyrings/PTY/elevation, live privileged `openvpn` route tunnel smoke, and interactive Browser OIDC validation in [`STANDALONE_CLI_STDIO_MCP_PLAN.md`](./STANDALONE_CLI_STDIO_MCP_PLAN.md).
