# Standalone CLI Slice 0: Linux x86_64 Feasibility Gate Report

Status: **Complete for Linux `x86_64` feasibility gate** (Windows, macOS, GUI-free Linux CI runner, interactive Browser OIDC, live privileged `openvpn` route tunnel, and implementation Slices 1–5 remain outside this gate).

- Report date: `2026-11-14` (Increment 1 baseline: `2026-11-11`, `ed2bda686`; Increment 2 baseline: `670a9ae` plus uncommitted Slice 0 workspace deliverables)
- Host target: Linux `x86_64`
- Observed toolchain & local binaries:
  - `rustc 1.94.0 (4a4ef493e 2026-03-02)`
  - `cargo 1.94.0 (85eff7c80 2026-01-15)`
  - Node `v24.14.0`, npm `11.12.1`
  - `/usr/bin/boundary 0.19.1` (`d363c002e743cf11b8aa8d45b51148c51006dee9`)

## Deliverables completed

1. **Increment 1 — Minimal dependency-pure workspace crates & ledger foundation (`ed2bda686`):**
   - Established standalone workspace crates `scp-core`, `scp-mcp`, and `scp-cli` with release CLI binary compilation and discovery scaffolding (`discovery.rs`).
   - Verified dependency purity: `cargo tree -p scp-cli -e normal,build` pulls only `scp-core`, `scp-mcp`, `serde v1.0.229`, `serde_json v1.0.154` (plus standard core/derive Rust crates), with zero Tauri, GTK/WebKit, HTTP server, or frontend dependencies.
   - Created `docs/mcp/STANDALONE_CLI_OPERATION_LEDGER.md` and automated ledger/policy tests (`crates/scp-mcp/tests/ledger.rs` plus `scp_mcp` and `scp-cli` unit tests).

2. **Increment 2 — Isolated platform spikes & `stdio` MCP harness (`670a9ae` + uncommitted changes):**
   - Added isolated spike crate `experiments/cli-stdio-slice0/` (excluded from the root workspace in `Cargo.toml`) pinning:
     - Rust: `rmcp v3.9.0` (`transport-io`, `server`, `macros`, `base64`), `keyring v3.6.3`, `dbus-secret-service v4.1.0`, `portable-pty v0.8.1`, `libc v0.2.191`, `tokio v1.54.1`, `tokio-util v0.7.20`.
     - Node/JS harness: `@modelcontextprotocol/client 2.4.0`, `@modelcontextprotocol/sdk 1.32.2`, `@modelcontextprotocol/inspector 2.14.0`.
   - Implemented spikes in `experiments/cli-stdio-slice0/src/`:
     - `profile_lock.rs`: Profile & owner lease lock behavior.
     - `keyring_spike.rs`: `NamespacedSecretStore::new` with Secret Service availability/lock pre-flight.
     - `supervisor_spike.rs`: Child process tracking and `Supervisor::shutdown` reaping.
     - `vpn_spike.rs` & `boundary_spike.rs`: Linux non-GUI VPN/Boundary state spikes.
     - `stdio_host.rs`: Bounded `stdio` MCP host (`StdioProbeHost`, `EofWatchReader`, `TimedAsyncWriter`, `AsyncRwTransport::new_server`, `probe_echo`, `probe_hold`, `probe://status`, `probe://active`).
     - `main.rs` & `lib.rs`.
   - Added acceptance tests and evidence:
     - `experiments/cli-stdio-slice0/tests/platform_spikes.rs` (6 integration tests).
     - `experiments/cli-stdio-slice0/tests/stdio.test.mjs` (10 end-to-end stdio compatibility & resilience tests).
     - `docs/mcp/evidence/CLI_STDIO_SLICE0.tap` (TAP v13 output for the 10 stdio tests: `1..10`, `# pass 10`, `# fail 0`).
   - Completed `docs/mcp/STANDALONE_CLI_OPERATION_LEDGER.md` with `## Local CLI-only command workflows` and `## Profile and platform decisions and Linux Slice 0 spike evidence`, updated `crates/scp-mcp/tests/ledger.rs`, and updated status headers in `docs/mcp/STANDALONE_CLI_STDIO_MCP_PLAN.md` and `docs/mcp/SETTINGS_BACKUP_STREAMABLE_HTTP_STANDALONE_CLI_PLAN.md`.

## Key technical findings & platform decisions

- **`rmcp` EOF shutdown & buffered drain (`stdio_host.rs`):** `rmcp 3.9.0`'s `serve_inner` waits `5s` (`QuitReason::Closed`) before cancelling `serve_loop_ct` on `stdin` EOF. Wrapping `stdin` with `EofWatchReader`, applying a `150ms` drain grace, and signalling cancellation bounds worker-saturated EOF shutdowns (`<= 2s` verified in harness) while allowing already-buffered fast requests to flush.
- **Client discovery probe clean exit:** `@modelcontextprotocol/client 2.4.0` opens a sibling `StdioClientTransport` for `server/discover` and immediately closes `stdin`, causing `ServiceExt::serve` to return `ServerInitializeError::ConnectionClosed`. Treating that case cleanly allows exit `0` instead of erroring on discovery probes.
- **`stdout` NDJSON purity & slow/stalled readers:** Keeping subprocess PTY+pipe output off `stdout`, configuring `AsyncRwTransport::new_server(...).with_max_line_length(...)`, bounding concurrent tool workers with a `Semaphore` (`SCP_STDIO_MAX_WORKERS`), and guarding writes with `TimedAsyncWriter` (`SCP_STDIO_WRITE_TIMEOUT_MS`) prevents `stdout` corruption, recovers from malformed JSON / `-32600` / `-32022` / `-32602` / oversized-frame requests, and aborts stalled readers (`400ms` write timeout verified `<= 2.5s` including `Supervisor::shutdown` child reap).
- **Linux keyring lock pre-flight (`keyring_spike.rs`):** `keyring 3.6.3` triggers `/usr/libexec/gcr-prompter` when Secret Service is locked. Checking `dbus_secret_service::Collection::is_locked()` detects locked or unavailable stores in `< 2s` so non-interactive callers can fail or fall back explicitly instead of hanging on a GUI prompt.

## Verification ledger (Linux x86_64, run after final edits)

| Check | Command | Observed result |
| --- | --- | --- |
| Workspace crates & ledger | `cargo test --locked -p scp-core -p scp-mcp -p scp-cli` | `10 passed; 0 failed` (`3` CLI discovery, `5` `scp_mcp` unit tests, `2` `ledger.rs` tests) |
| Release standalone CLI build | `cargo build --locked --release -p scp-cli` | Exited `0` |
| Standalone CLI dep tree | `cargo tree -p scp-cli -e normal,build` | Only `scp-core`, `scp-mcp`, `serde v1.0.229`, `serde_json v1.0.154` (+ Rust core/derive crates) |
| Spike crate dep check | `cargo tree --manifest-path experiments/cli-stdio-slice0/Cargo.toml -e normal,build \| rg "axum\|tauri\|hyper\|webkit\|gtk"` | Exited `0` (`NO_HTTP_OR_GUI_DEPS` — no HTTP or GUI runtime deps) |
| Spike crate formatting & lint | `cargo fmt --manifest-path experiments/cli-stdio-slice0/Cargo.toml -- --check && cargo clippy --locked --manifest-path experiments/cli-stdio-slice0/Cargo.toml --all-targets -- -D warnings` | Exited `0`, 0 warnings |
| Platform spikes A–E | `cargo test --locked --manifest-path experiments/cli-stdio-slice0/Cargo.toml` | `6 passed; 0 failed` in `platform_spikes.rs` |
| Stdio MCP harness | `npm test --prefix experiments/cli-stdio-slice0` | `10 passed; 0 failed` across `2025-11-25` legacy & `2026-07-28` modern SDKs, Inspector CLI `2.14.0` (modern + legacy), `server/discover` EOF, cancellation, stdout NDJSON purity & error recovery, worker saturation + EOF reap, and stalled writer timeout (`docs/mcp/evidence/CLI_STDIO_SLICE0.tap` captured `# pass 10`, `# fail 0`) |
| Desktop Tauri tests | `cargo test --manifest-path src-tauri/Cargo.toml` | `126 passed; 0 failed` (`supercontrolpanelhelper` 0 tests; with `npm ci --prefix experiments/mcp-http-slice0 --ignore-scripts --no-audit --no-fund` installed) |
| Frontend build & E2E | `npm run build && npx playwright test` | `npm run build` exited `0` (`18 modules transformed`); Playwright `36 passed, 27 skipped` |
| Code quality gate | `ripwire . --quality-delta --legend=compact` | `0` gating rows in `crates/`, `Cargo.toml`, or `experiments/cli-stdio-slice0/` (`preexisting-worse="4" gating="4"` only from pre-existing untracked `cc-server/` vs `src/settingsBackupUi.ts::getRequired`; `.ripwire_quality_acks` untouched) |

## Explicit unrun checks & remaining scope

1. **Non-Linux targets:** Windows and macOS native spikes (keyring/keychain, elevation helpers, PTY/Boundary process behavior) have not been executed on Windows or macOS hosts.
2. **GUI-free headless Linux CI runner:** Headless execution without an unlocked desktop Secret Service session remains to be validated in CI.
3. **Privileged/interactive external integrations:** Interactive Browser OIDC Boundary auth and live privileged `openvpn` route-modifying tunnel establishment were not executed against live infrastructure in Slice 0.
4. **Implementation Slices 1–5:** Production extraction, owner lease in production crates, runtime families, complete CLI shell & backup, full stdio workflows, and release packaging remain planned in [`STANDALONE_CLI_STDIO_MCP_PLAN.md`](./STANDALONE_CLI_STDIO_MCP_PLAN.md).
