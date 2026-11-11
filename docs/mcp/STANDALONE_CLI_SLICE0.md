# Standalone CLI Slice 0: first increment

Date: 2026-11-11. Source baseline: `ed2bda686` plus this uncommitted increment. Host: Linux x86_64. Toolchain: rustc `1.99.1 (58cf8f9c3 2026-10-13)`, cargo `1.99.1 (5f94df478 2026-08-27)`. Slice 0 remains **in progress**; this report does not close its feasibility gate.

## Delivered boundary

The root Cargo workspace contains `scp-core`, `scp-mcp` and `scp-cli`, with a committed root lockfile. Desktop is explicitly excluded and retains `src-tauri/Cargo.toml`, its separate lockfile, target directory, build script, features and existing npm/Tauri commands. The HTTP experiment retains its own workspace. Desktop consumes `scp-mcp` as a path dependency.

The registry, public DTOs, schema validator and five existing tests moved into `scp-mcp`. Desktop modules re-export the same items so consumers keep their existing paths. Host error classification is public across the crate boundary; its safe messages and behavior are unchanged. `authorize` remains desktop policy logic; a future standalone execution host must supply its own reviewed local authority and capability checks. Importing this crate or listing its metadata does not authorize execution.

`scp-core` begins the profile-free shell result envelope. `scpctl schema --json` emits exactly one versioned envelope with nullable runtime ID, command metadata, and a stable sorted catalog of all 45 operations and six resource mappings. Input/output schemas reuse the desktop registry. Catalog entries explicitly report `available: false`, `reason: runtime_not_extracted`. No profile, credential store, child, HTTP endpoint, desktop token discovery or runtime is opened. This metadata catalog is not a live stdio MCP tool list.

The binary supports `--help` and `--version`. Other commands/arguments return usage exit 2; JSON shell errors carry code, message, retryable, reason, nextAction and outcome. Attempts to start unsupported MCP mode produce diagnostics on stderr and no shell JSON on stdout. Runtime commands, profile flags, `schema --command`, completion, secret input and stdio are not yet implemented. `scpctl` remains a provisional name; release naming checks are outstanding.

The [operation ledger](STANDALONE_CLI_OPERATION_LEDGER.md) records extraction dispositions, proposed shell mappings, ownership, interaction, readiness/completion, preview and retry obligations, named future gates, resource mappings and platform blockers. Its coverage test fails when a registry operation/resource lacks a ledger entry. Named future tests are not presented as implemented tests.

## TDD and executed checks

CLI subprocess RED: all three discovery tests failed against an empty main: missing output, incorrectly successful unsupported commands and absent help/version. GREEN: the same tests pass with production CLI code and closed stdin. Ledger RED: failed on absent ledger; GREEN: every current operation/resource is covered. Existing schema/policy tests moved intact, except a test import now uses the package root.

| Command | Observed result |
| --- | --- |
| `cargo test --locked -p scp-core -p scp-mcp -p scp-cli` | 9 passed: 3 CLI subprocess, 5 shared policy/schema, 1 ledger coverage |
| `cargo build --locked --release -p scp-cli` | Passed; independent `target/release/scpctl` |
| `cargo tree -p scp-cli -e normal,build` | Only workspace packages plus serde/serde_json and their Rust dependencies; no Tauri, GTK/WebKit, Axum, rmcp, frontend assets or tauri-build |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 126 passed plus helper target; 5 former desktop tests now run in scp-mcp; official HTTP SDK JSON/SSE and legacy tests passed; existing unused/dead-code warnings |
| `npm run build` | Passed |
| `npx playwright test` | 36 passed, 27 skipped (default configuration) |
| `cargo fmt --all -- --check`, `git diff --check` | Passed |
| Ripwire impact/uses/affected and quality delta | Registry reaches HTTP/Tauri callers; re-exports preserve paths; zero blocking quality regressions. Relocated symbols appear as new-code findings, including test/macro reachability and existing DTO shape duplication. No quality suppressions added. |

Root lockfile pins serde `1.0.229` and serde_json `1.0.154`. No stdio SDK/client is selected or qualified in this increment. Existing desktop tests retain rmcp `3.9.0` and the Node clients already pinned by Work 2; their HTTP success is not stdio evidence.

The release graph/build proves dependency isolation on this machine, which already has GUI development packages. It does **not** substitute for building/installing on a GUI-free runner. No native Windows/macOS, keyring/elevation, Boundary external-state, network workflow, controlling-terminal/PTY launcher, real stdio client, or signal/EOF runtime cleanup qualification was performed. Browser tests are the default suite, not fresh native desktop smoke.

## Next increment

Characterize profile paths and config/backup recovery seams before extracting loaders. Implement canonical private profiles, cross-process exclusive owner leases, stable credential namespaces and recovery-before-load with RED/GREEN process tests. Run native keyring/process/elevation/Boundary spikes and retain unavailable capabilities where evidence is missing. Only after these gates, add selected-profile safe server/status queries and the minimal read-only stdio host. Do not enable runtime families or reuse desktop authority merely to make discovery appear functional.
