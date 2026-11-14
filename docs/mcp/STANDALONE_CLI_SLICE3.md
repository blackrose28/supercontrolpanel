# Standalone CLI Slice 3: Complete Shell Surface and Encrypted Settings Backup Report

Status: **Complete for Linux `x86_64` Slice 3 gate** (Slice 4 stdio MCP exposure of extracted workers and SFTP, Slice 5 release packaging/GUI-free Linux CI runner validation, Windows and macOS native qualification, live privileged `openvpn` route tunnel testing, and interactive Browser OIDC validation remain open).

- Baseline commit: `854e3ea` (Slice 2)
- Host target: Linux `x86_64`
- Observed toolchain & client packages:
  - `rustc 1.100.0 (0ef1bc386 2026-11-09)`
  - `cargo 1.100.0 (3acc79c0c 2026-11-05)`
  - Node `v26.13.0`
  - `@modelcontextprotocol/sdk 1.32.2` and `@modelcontextprotocol/client 2.4.0`

## Architecture & extracted capabilities delivered

1. **Shared `.scpbak` backup engine in `scp-core` (`crates/scp-core/src/backup/`):**
   - Extracted the Tauri-free encrypted archive, preview, fingerprint, and sync reference engine into `crates/scp-core/src/backup/archive.rs` (`Archive`, `PendingRestore`, `Preview`, `RestoreOptions`, `SectionSummary`, `ExternalFile`, `snapshot`, `snapshot_with_version`, `compile_restore`, `describe`, `fingerprint`, `known_accounts`, `validate_export_target`) and `crates/scp-core/src/backup/references.rs`, alongside shared `wipe_json_and_credentials` in `crates/scp-core/src/backup/mod.rs` and the undo journal in `crates/scp-core/src/backup/transaction.rs`.
   - `ProfileRuntime` (`crates/scp-core/src/config_ops.rs` + `crates/scp-core/src/runtime.rs`) implements `export_settings_backup`, single-load `prepare_settings_import`, in-memory `refresh_pending_import_preview`, and transactional `commit_pending_import` with profile-isolated `owner.lock`, `MaintenanceGate`, preview ID/TTL/destination revision & fingerprint checks, and `NamespacedSecretStore` (`profile:{ns}:*`) isolation.
2. **Configuration, server, credential, sync-push, and RDP mutations (`crates/scp-core/src/config_ops.rs` & `tty.rs`):**
   - Added transactional operations with `--preview` and optimistic `--expected-revision` conflict checks (`CONFLICT`, exit `5`): `upsert_server`, `delete_server`, `set_server_ssh_password`, `clear_server_ssh_password`, `save_app_config`, `patch_app_config`, `set_config_secret`, `clear_config_secret`, `patch_sync_settings`, `push_config_sync`, `launch_server_rdp`, `validate_server_id`, `validate_rdp_target`, and `resolve_config_secret_account`.
   - Extended `crates/scp-core/src/tty.rs` with signal-aware `read_tty_line` and `read_tty_secret` backed by `TerminalModeGuard` (disabling `ECHO` during secret/passphrase prompts and restoring `termios` on commit or `SIGINT` / `SIGTERM`).
3. **Desktop `src-tauri` delegations (`src-tauri/src/`):**
   - Delegated `src-tauri/src/settings_backup/mod.rs` and `src-tauri/src/settings_backup/desktop.rs` (`Pending(PendingRestore)` wrapper via `Deref`/`DerefMut`, removing duplicated archive/preview/reference code and `src-tauri/src/settings_backup/references.rs`) directly to `scp_core::backup` without altering Tauri backup command signatures or desktop keyring behavior.
   - Delegated `validate_rdp_host` in `src-tauri/src/mcp/parity.rs` to `scp_core::config_ops::validate_rdp_target`.
4. **Shared `scp-mcp` standalone schema & capability surface (`crates/scp-mcp/src/standalone.rs`):**
   - Extended `standalone_schema_catalog()` with `slice3_config_backup_schemas()` to `31` CLI command schemas (`8` Slice 1 + `9` Slice 2 + `14` Slice 3), including `previewSupported` and `idempotent` metadata.
   - Updated `capabilities_report()` to report `"config": "cli_local"`, `"backup": "cli_local"`, and `"rdp": "cli_interactive_gui"` under `"runtimeFamilies"`, narrowing `"unavailableFamilies"` to `{"sftp": "slice4_planned"}`.
   - Preserved the Slice 1 stdio MCP read-only boundary (`11` read-only MCP ops `available: true`, `34` mutating/worker MCP ops `available: false`, `6` `scp://*` resources, and `9` default permitted ops under `stdio-policy.json`) until Slice 4.

## Standalone `scpctl` Slice 3 commands & security contracts

- **Implemented Slice 3 subcommands (`crates/scp-cli/src/`):**
  - `scpctl completion <bash|zsh|fish|powershell>` (or `--shell <SHELL>`) — profile-free shell script generation (`completion.rs`; never creates profile dirs, locks, or keyring entries, and never emits secret/passphrase flags).
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] server upsert (--file <PATH> | --stdin) [--preview] [--expected-revision <REV>] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] server delete --server-id <ID> [--preview] [--expected-revision <REV>] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] server credentials set --server-id <ID> (--secret-file <PATH> | --secret-stdin | --interactive) [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] server credentials clear --server-id <ID> [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] server rdp --server-id <ID> --interactive`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] config save (--file <PATH> | --stdin) [--preview] [--expected-revision <REV>] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] config patch (--file <PATH> | --stdin) [--preview] [--expected-revision <REV>] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] config secret set --key <KEY> (--secret-file <PATH> | --secret-stdin | --interactive) [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] config secret clear --key <KEY> [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] config sync push [--repo-root <DIR>] [--preview] [--expected-revision <REV>] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] settings patch (--file <PATH> | --stdin) [--preview] [--expected-revision <REV>] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] settings export --output <PATH> [--overwrite] (--passphrase-file <PATH> | --passphrase-stdin | --interactive) [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] settings import --input <PATH> --preview (--passphrase-file <PATH> | --passphrase-stdin | --interactive) [--no-shared] [--no-servers] [--no-local] [--no-secrets] [--use-source-paths] [--remap <KEY=PATH>] [--json]`
  - `scpctl [--config-dir <DIR>] [--profile <NAME>] settings import --input <PATH> --apply --interactive [--passphrase-file <PATH> | --passphrase-stdin] [--no-shared] [--no-servers] [--no-local] [--no-secrets] [--use-source-paths] [--remap <KEY=PATH>]`
- **Secret, passphrase & non-interactive guarantees (`args.rs`, `secret_input.rs`, `mutation_cmds.rs`, `backup_cmds.rs`):**
  - Inline `--password`, `--secret`, `--passphrase`, and `--recover-identity` flags (including `--flag=...`) fail fast with exit `2` (`unsafe_secret_flag`). Secret/passphrase buffers are wrapped in `zeroize::Zeroizing` via the `zeroize = "1"` crate dependency on `scp-cli`.
  - `--interactive` combined with `--json` or `--non-interactive` fails fast with exit `2` on Slice 3 commands (`boundary auth` retains Slice 2 behavior).
  - `server rdp` without `--interactive`, under `--non-interactive`, without a controlling TTY, or without `DISPLAY` / `WAYLAND_DISPLAY` fails fast with `CAPABILITY_UNAVAILABLE` (exit `4`) before touching profiles or keyring state.
  - `settings import --apply` fails fast with `CAPABILITY_UNAVAILABLE` (exit `4`) when `--non-interactive`, missing `--interactive`, or no TTY is present, holds a single decrypted in-memory `PendingRestore` across interactive `sections`/`paths`/`remap` adjustments (each minting a fresh preview ID), enforces preview TTL (`SCPCTL_TEST_PREVIEW_TTL_MS`), rejects modified destinations, and preserves CLI profile credential namespace & `stdio-policy.json` isolation across desktop `.scpbak` round-trips.

## Verification ledger (Linux `x86_64`)

| Check | Command | Observed result |
| --- | --- | --- |
| Workspace formatting | `cargo fmt --check` | Exited `0` |
| Desktop delegated file formatting | `rustfmt --edition 2021 --check src-tauri/src/settings_backup/mod.rs src-tauri/src/settings_backup/desktop.rs src-tauri/src/mcp/parity.rs` | Exited `0` |
| Workspace lint | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Exited `0`, `0` warnings |
| Core, MCP & CLI tests | `cargo test --locked -p scp-core -p scp-mcp -p scp-cli` | Exited `0` (`76 passed; 0 failed`: `scp-core` `38` [`1` unit + `9` `slice1_core` + `1` `slice2_async_scp` + `4` `slice2_boundary` + `2` `slice2_caddy` + `4` `slice2_openvpn` + `4` `slice2_ssh_scp_sftp` + `6` `slice2_supervisor` + `7` `slice3_backup_and_config`], `scp-mcp` `13` [`5` unit + `2` `ledger` + `6` `slice1_standalone`], `scp-cli` `25` [`9` `cli_slice1` + `6` `cli_slice2` + `7` `cli_slice3` + `3` `discovery`]) |
| `scp-mcp` feature matrix & ledger | `cargo test --locked -p scp-mcp --all-features && cargo test --locked -p scp-mcp && cargo test --locked -p scp-mcp --test ledger` | Exited `0` (`13 passed; 0 failed` with `--all-features`, `12 passed; 0 failed` default features, `2 passed; 0 failed` in `ledger`) |
| Release binary & dep purity | `cargo build --locked --release -p scp-cli && cargo tree --locked -p scp-cli -e normal,build` | Exited `0`; zero `tauri`, `wry`, `tao`, `webkit2gtk`, `gtk`, `glib`, `libdbus-sys`, `reqwest`, `axum`, or `hyper` dependencies |
| Production `scpctl` stdio E2E | `node --test --test-reporter=spec experiments/cli-stdio-slice0/tests/slice1_production_stdio.mjs` | Exited `0` (`3 passed; 0 failed`) |
| Ripwire quality delta | `ripwire . --quality-delta --legend=compact` | Zero duplication, verbosity, or new-clone `gating="1"` findings in `crates/` and `src-tauri/` (`3` remaining `gating="1"` rows overall: `2` pre-existing rows in `cc-server/` and `1` expected `short-horizon-churn` row on `crates/scp-mcp/src/standalone.rs::capabilities_report` updated across consecutive Slices 1–3) |

## Environment-blocked checks on this host & remaining scope

1. **Environment-blocked checks on this minimal headless offline Linux container (not run here):**
   - `cargo test --manifest-path src-tauri/Cargo.toml`: blocked because system GTK/GLib development pkg-config (`glib-2.0.pc`) is not installed (`src-tauri/src/settings_backup/mod.rs`, `src-tauri/src/settings_backup/desktop.rs`, and `src-tauri/src/mcp/parity.rs` passed `rustfmt --edition 2021 --check`).
   - `npm test --prefix experiments/cli-stdio-slice0` (disposable Slice 0 `scp-cli-stdio-slice0` binary): blocked because `dbus-secret-service` / `libdbus-sys` requires system `dbus-1.pc` (`libdbus-1-dev`) not installed on this container (`production scpctl` does not depend on `libdbus-sys` and passed `slice1_production_stdio.mjs` `3/3`).
   - Root `npm run build` (`tsc && vite build`) and `npx playwright test`: blocked because root `node_modules/` is absent in this offline workspace (no frontend TypeScript/CSS files were modified in Slice 3).
2. **Remaining planned scope:**
   - Slice 4: stdio MCP exposure and qualification of extracted runtime workers and SFTP browser tools.
   - Slice 5 & OS matrix: release packaging, GUI-free Linux CI runner validation without Secret Service, Windows and macOS native builds/keyrings/PTY/elevation, live privileged `openvpn` route tunnel smoke, and interactive Browser OIDC validation in [`STANDALONE_CLI_STDIO_MCP_PLAN.md`](./STANDALONE_CLI_STDIO_MCP_PLAN.md).
