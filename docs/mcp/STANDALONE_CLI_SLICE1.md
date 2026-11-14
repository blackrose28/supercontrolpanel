# Standalone CLI Slice 1: Profile/Config Core, Read-Only CLI, and Stdio MCP Host Report

Status: **Complete for Linux `x86_64` Slice 1 gate** (runtime families OpenVPN/Boundary/Caddy/SSH/SFTP/SCP in Slices 2–4, Windows and macOS native qualification, GUI-free Linux CI runner validation without Secret Service, and release packaging in Slice 5 remain open).

- Host target: Linux `x86_64`
- Observed toolchain & client packages:
  - `rustc 1.94.0 (4a4ef493e 2026-03-02)`
  - `cargo 1.94.0 (85eff7c80 2026-01-15)`
  - Node `v24.14.0`
  - `@modelcontextprotocol/sdk 1.32.2` (`2025-11-25`) and `@modelcontextprotocol/client 2.4.0` (`2026-07-28`)

## Architecture & deduplication delivered

1. **Headless `scp-core` (`crates/scp-core`):**
   - Domain models (`models.rs`), `CoreError` + deterministic CLI exit code mapping (`error.rs`), and single-line JSON response envelopes (`lib.rs`).
   - Extracted shared encrypted `.scpbak` primitives, bounded IO, and atomic crash-recovery journal transaction engine (`backup/{mod.rs,crypto.rs,io.rs,transaction.rs}`).
   - `MaintenanceGate` (`gate.rs`), canonical profile resolution and desktop-alias rejection (`profile.rs`), exclusive OS `OwnerLease` (`flock` `LOCK_EX | LOCK_NB` on Unix / `LockFileEx` on Windows on un-unlinked `owner.lock`), `ProfileMetadata` (`profile-meta.json` storing stable local `credentialNamespace`), mandatory journal recovery before config/server parsing, `NamespacedSecretStore` with non-prompting Linux Secret Service preflight (`secrets.rs`), repository loaders/migrations (`config_repo.rs`), and read-only `ProfileRuntime` (`runtime.rs`).
2. **Shared `scp-mcp` (`crates/scp-mcp`):**
   - Generalized `Capability` (`Runtime`, `InteractiveHost`, `ProtectedImport`, `ReviewedGit`) and `Lifetime` (`Query`, `Request`, `HostOwned`, `Stop`, `Mutation`), host-neutral standalone errors (`contracts.rs`), and `From<&scp_core::models::*>` into `Safe*` DTOs (`public_dto.rs`).
   - Standalone Slice 1 read-only dispatch (`standalone.rs`), optional `sdk` feature (`lifecycle::TransportControl`, `protocol::SdkHost`), and optional `stdio` feature (`stdio::serve_stdio_runtime` / `serve_stdio_io` over `rmcp = "=3.9.0"` with `EofWatchReader` + `TimedAsyncWriter`).
   - Dependency isolation verified: `cargo tree --locked -p scp-cli -e normal,build` excludes `tauri`, `tauri-build`, `tauri-runtime`, `wry`, `tao`, `webkit2gtk`, `gtk`, `axum`, and `hyper`.
3. **Desktop `src-tauri` integration:**
   - Updated `src-tauri/Cargo.toml` to depend on `scp-core` and `scp-mcp` (`features = ["sdk"]`).
   - Replaced duplicated models, service helpers/constants, `settings_backup/{crypto,io,transaction}`, and MCP DTO/lifecycle/SDK adapter code with `pub use` re-exports from `scp-core` and `scp-mcp`, preserving existing Tauri commands, desktop HTTP/REST behavior, Streamable HTTP `/mcp` contracts, and tests.

## 5-step profile startup & isolation contract

For every profile-backed command (`capabilities` without `--no-profile`, `status`, `server`, `config paths`, `settings show`, `mcp serve --stdio`):

1. Resolve profile location (`--config-dir` > `SCPCTL_CONFIG_DIR` > `XDG_CONFIG_HOME/scpctl` > `~/.config/scpctl`, with `--profile` validated and defaulting to `default`), reject aliases pointing inside the desktop config directory (`POLICY_DENIED`, exit `3`, reason `desktop_profile_rejected`), and acquire exclusive `owner.lock` (`CONFLICT`, exit `5`, reason `profile_owner_conflict`).
2. Read or atomically create `profile-meta.json` (`credentialNamespace`). Existing pending journal without metadata blocks startup rather than regenerating a new namespace.
3. Execute `transaction::recover` using the profile namespace before reading configuration, server lists, or initializing secret caches.
4. Load `servers.json`, `sync-local.json`, `stdio-policy.json`, and `config.json` (migrating any legacy plaintext secrets to `profile:{ns}:{account}` without prompting when keyring is unavailable and no plaintext secrets exist).
5. Construct read-only `ProfileRuntime` with deterministic `runtimeId` (`rt-...`).

## Standalone `scpctl` commands & stdio policy

- **Implemented Slice 1 commands:** `--help`, `--version`, `schema [--command <CMD>] --json`, `capabilities [--no-profile] [--json]`, `status [--function-id <ID>] [--json]`, `server list [--limit <N>] [--after <ID>] [--json]`, `server get --server-id <ID> [--json]`, `config paths [--json]`, `settings show [--json]`, and `mcp serve --stdio`.
- **Exit codes:** `0` success, `1` operational failure, `2` `INVALID_ARGUMENTS`, `3` `POLICY_DENIED`, `4` `CAPABILITY_UNAVAILABLE`, `5` `CONFLICT`, `6` `NOT_FOUND`, `7` `TIMEOUT`, `130` `INTERRUPTED`.
- **Stdio policy defaults:** Controlled exclusively by profile-local `stdio-policy.json` (default `sensitiveActionPolicy: "read-only"`, `allowedToolCategories: ["read-only"]`, completely independent of `sync-local.json`). Default exposes 9 `read-only` tools and 5 resources; enabling `"file-transfer"` adds `scp_transfer_list`, `scp_transfer_get`, and `scp://status/transfers` (`11` tools, `6` resources). Mutations and unextracted runtime families fail closed (`POLICY_DENIED` / `CAPABILITY_UNAVAILABLE`), secrets/tokens (`mcpAuthToken`, `ccAgentToken`) are omitted/redacted, and `mcp serve --stdio` reserves `stdout` strictly for newline-delimited JSON-RPC (even on startup errors).

## Verification ledger (Linux `x86_64`)

| Check | Command | Observed result |
| --- | --- | --- |
| Workspace formatting | `cargo fmt --check` | Exited `0` |
| Workspace lint | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Exited `0`, `0` warnings |
| Core, MCP & CLI tests | `cargo test --locked -p scp-core -p scp-mcp -p scp-cli` | Exited `0`: `scp_core` (`9` in `slice1_core`), `scp_mcp` (`5` unit, `2` `ledger`, `5` in `slice1_standalone`), `scp-cli` (`9` in `cli_slice1`, `3` in `discovery`) |
| `scp-mcp` all features | `cargo test --locked -p scp-mcp --all-features` | Exited `0` (`12 passed; 0 failed`) |
| Release binary & dep purity | `cargo build --locked --release -p scp-cli && cargo tree --locked -p scp-cli -e normal,build` | Exited `0`; zero Tauri, GUI, or Axum/Hyper dependencies |
| Production `scpctl` stdio E2E | `node --test --test-reporter=spec experiments/cli-stdio-slice0/tests/slice1_production_stdio.mjs` | Exited `0` (`3 passed; 0 failed` across `2025-11-25`, `2026-07-28`, redaction, default-deny, NDJSON purity, EOF exit `0`, lock conflict exit `5`) |
| Slice 0 harness regression | `npm test --prefix experiments/cli-stdio-slice0` | Exited `0` (`10 passed; 0 failed`) |
| Desktop formatting (`src-tauri` edited files) | `rustfmt --edition 2021 --check` on modified `src-tauri/src/{models,server_mgmt,services,settings_backup,mcp}` files | Exited `0` |
| Desktop Tauri tests | `cargo test --manifest-path src-tauri/Cargo.toml` | Exited `0` (`126 passed; 0 failed`) |
| Frontend build & E2E | `npm run build && npx playwright test --reporter=list` | Exited `0` (`18 modules transformed`; Playwright `36 passed, 27 skipped, 0 failed`) |

## Explicit unrun checks & remaining scope

1. Windows and macOS native builds, `LockFileEx`, OS credential stores, PTY/ConPTY behavior, and elevated OpenVPN flows remain unverified on Windows/macOS runners.
2. GUI-free Linux CI runners without an unlocked D-Bus Secret Service daemon remain an open CI verification gate.
3. Runtime worker extraction (OpenVPN, Boundary, Caddy, SSH, SFTP, SCP), full mutation CLI/backup export-import workflows, and release packaging belong to Slices 2–5 in [`STANDALONE_CLI_STDIO_MCP_PLAN.md`](./STANDALONE_CLI_STDIO_MCP_PLAN.md).
