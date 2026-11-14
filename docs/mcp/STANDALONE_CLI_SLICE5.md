# Standalone CLI Slice 5 Report (Release Packaging, Agent Guide & Qualification)

Status: **Complete on Linux `x86_64`** (headless CLI, GUI-free `dbus-run-session`, release packaging/tamper check, prefix installer, and shipped agent workflow examples verified; see environment-blocked items below for multi-OS CI runners, desktop Tauri/GTK packages, and container image build).

## Scope and deliverables

- **Release packaging and dependency/linkage purity:** [`scripts/package-scpctl.mjs`](../../scripts/package-scpctl.mjs) builds or ingests release `scpctl`, generates shell completions (`bash`, `zsh`, `fish`, PowerShell), verifies `cargo tree --locked -p scp-cli -e normal,build` excludes GUI/Tauri/D-Bus/HTTP crates and `ldd` excludes GUI/X11/Wayland/`libdbus-1` shared libraries on Linux, packages `scpctl-v<version>-<os>-<arch>.tar.gz` (`.zip` on Windows), and emits `SHA256SUMS`, `build-manifest.json`, and `scpctl-cargo-tree.txt`, plus `--verify <dir>` tamper verification.
- **Prefix installer:** [`scripts/install-scpctl.sh`](../../scripts/install-scpctl.sh) validates `SHA256SUMS` before unpacking into `--prefix` (`bin/scpctl` plus completion directories) and runs a `--version` sanity probe.
- **Container & CI automation:** [`Dockerfile.cli`](../../Dockerfile.cli) defines a minimal Ubuntu 22.04 headless build/verification stage without GTK, WebKit, or D-Bus dev headers; [`.github/workflows/standalone-cli.yml`](../../.github/workflows/standalone-cli.yml) adds a multi-OS `standalone-cli` matrix (`ubuntu-latest`, `macos-latest`, `windows-latest`) plus a separate `desktop-regression` Ubuntu job; [`package.json`](../../package.json) adds `cli:build`, `cli:release`, `cli:test`, and `cli:package`; [`.gitignore`](../../.gitignore) ignores `/dist-cli/`.
- **Agent guide & runnable workflow examples:** [`docs/mcp/STANDALONE_CLI_AGENT_GUIDE.md`](./STANDALONE_CLI_AGENT_GUIDE.md) documents the agent discover -> capabilities -> preview -> expected-revision -> lost-response recovery loop, headless pre-provisioning, deterministic exit codes, stdio MCP framing/policy bounds, Inspector/SDK/client configs, and runnable scripts in [`examples/standalone-cli/`](../../examples/standalone-cli/lib/agent-helpers.mjs):
  - [`examples/standalone-cli/shell-agent-workflow.mjs`](../../examples/standalone-cli/shell-agent-workflow.mjs)
  - [`examples/standalone-cli/stdio-mcp-agent-workflow.mjs`](../../examples/standalone-cli/stdio-mcp-agent-workflow.mjs)
- **Qualification tests & evidence:** [`crates/scp-cli/tests/cli_slice5.rs`](../../crates/scp-cli/tests/cli_slice5.rs), [`experiments/cli-stdio-slice0/tests/slice5_release_qualification.mjs`](../../experiments/cli-stdio-slice0/tests/slice5_release_qualification.mjs), and sanitized [`docs/mcp/evidence/CLI_SLICE5_RELEASE.txt`](./evidence/CLI_SLICE5_RELEASE.txt).

## Minimum host prerequisites

- **Required at runtime:** OpenSSH client binaries (`ssh`, `scp`, `sftp` on `PATH`), pre-provisioned host keys (`~/.ssh/known_hosts` or `SCPCTL_SSH_KNOWN_HOSTS`) for non-interactive `StrictHostKeyChecking=yes`, and standard C/TLS runtime libraries (Linux `x86_64` release artifact dynamically links only `libc`, `ld-linux-x86-64`, `libssl`, `libcrypto`, `libz`, `libgcc_s`, `libm`, `libpthread`, `libdl`).
- **Optional runtime tools:** `boundary`, `openvpn` (+ `sudo`), `caddy`, and platform RDP launcher when invoking those respective commands.
- **OS keyring behavior on GUI-free hosts:** Secret Service (`org.freedesktop.secrets` over D-Bus on Linux), Keychain (macOS), or Credential Manager (Windows) is required only for storing/clearing/exporting credential secrets. On headless Linux hosts without Secret Service (verified both with invalid `DBUS_SESSION_BUS_ADDRESS` and inside `dbus-run-session`), `capabilities`, SSH-key targets, `server upsert`/`list`/`get`, non-secret `config patch`/`sync push`, `scp`, and persistent SFTP browsing succeed cleanly; `config secret set`, `server credentials set`, and `server delete` / MCP `server_delete` (which inspects/clears the SSH password keyring slot fail-closed) return exit `4` (`CAPABILITY_UNAVAILABLE`, `reason: "keyring_unavailable"`), and un-mocked `settings export` returns exit `1` (`OPERATION_FAILED`, `reason: "backup_snapshot_failed"`).

## Capability matrix and coexistence with `supercontrolpanelhelper`

| Capability / Surface | Standalone CLI (`scpctl --json`) | Standalone Stdio MCP (`scpctl mcp serve --stdio`) | Desktop HTTP MCP (`/mcp/*`) + `supercontrolpanelhelper` |
| --- | --- | --- | --- |
| Requires GUI Desktop app open | No (must stay on isolated CLI profile) | No (holds `owner.lock` for stdio session) | Yes (attaches to running Tauri desktop session) |
| Discovery & status | `31` CLI commands (`schema`, `capabilities`, `status`) | `41` tools + `6` `scp://*` resources under permissive local policy (`11` read-only default; `15` with `file-transfer`) | Desktop HTTP MCP routes (`GET /health`, `/mcp/*`) via localhost bearer token |
| Interactive GUI / desktop-reviewed ops | `server rdp --interactive`, `settings import --apply --interactive` require TTY/GUI | `4` ops excluded (`server_rdp_start`, `config_sync_pull`, `config_git_pull`, `config_git_push` -> `CAPABILITY_UNAVAILABLE`) | Full desktop UI interactive/approval workflows (`require-ui-approval` & Browser OIDC popup) |
| Isolation & authorization | OS keyring + filesystem profile dir + `--expected-revision` optimistic concurrency | `stdio-policy.json` live-reloaded & fail-closed; independent of `sync-local.json` | Desktop `sync-local.json` bearer token + UI approval policy |

**Coexistence and migration guidance:**
- Keep `SCPCTL_CONFIG_DIR` separate from desktop configuration dirs (`scpctl` rejects desktop config paths/symlinks with exit `3` `POLICY_DENIED`).
- Use `supercontrolpanelhelper` when automating a live Super Control Panel desktop session over localhost HTTP MCP (`GET/POST /mcp/*`). Use `scpctl` / `scpctl mcp serve --stdio` on headless servers, CI runners, containers, or local agent loops when the desktop app is closed.
- Move configuration and encrypted secrets between desktop and standalone CLI profiles using portable `.scpbak` backups (`settings export` -> `settings import --apply --interactive`) or shared Git `app-config/` sync (`config sync push` + desktop/interactive review pull).

## Verification ledger (`d52c431` + Slice 5 working tree)

All commands executed locally on Ubuntu 20.04.1 LTS (`Linux 5.4.0 x86_64`), `rustc 1.100.0`, `cargo 1.100.0`, `Node v26.13.0`:

| Check | Result | Scope / Notes |
| --- | --- | --- |
| `cargo fmt --check` | Passed | Verified clean after formatting `cli_slice5.rs` |
| `cargo clippy --locked -p scp-core -p scp-mcp -p scp-cli --all-targets --all-features -- -D warnings` | Passed | Zero warnings across all standalone workspace targets |
| `cargo test --locked -p scp-core -p scp-mcp -p scp-cli` | **89 passed, 0 failed** | `scp-core` `44`, `scp-mcp` (`--all-features` workspace graph) `18`, `scp-cli` `27` (including `2` new `cli_slice5` tests) |
| `cargo test --locked -p scp-mcp --all-features` | **18 passed, 0 failed** | Unittests `5`, `ledger` `2`, `slice1_standalone` `6`, `slice4_standalone_stdio` `5` |
| `cargo test --locked -p scp-mcp` (default features) | **16 passed, 0 failed** | Unittests `5`, `ledger` `2`, `slice1_standalone` `5`, `slice4_standalone_stdio` `4` |
| `cargo test --locked -p scp-mcp --test ledger` | **2 passed, 0 failed** | Operation/resource parity ledger invariants |
| Release build + `scripts/package-scpctl.mjs` + `--verify dist-cli` | Passed | Produced `scpctl-v0.1.0-linux-x86_64.tar.gz` (`dependencyIsolationPassed: true`, `dynamicLinkageIsolationPassed: true`) |
| Installed-prefix Node E2E (`scripts/install-scpctl.sh` prefix + `slice1`, `slice4`, `slice5`) | **11 passed, 0 failed** | `slice1_production_stdio` (`3`), `slice4_production_stdio` (`5`, Inspector 2.14.0 modern/legacy + Client 2.4.0 + SDK 1.32.2), `slice5_release_qualification` (`3`, tamper rejection, `dbus-run-session` GUI-free smoke, both shipped workflow examples) |

## Environment-blocked items (not executed locally)

1. **`src-tauri` desktop build/tests:** Blocked locally by missing system `glib-2.0.pc` (`gtk-3`/`webkit2gtk` dev libraries not installed in this headless container); covered by `.github/workflows/standalone-cli.yml` `desktop-regression` job on `ubuntu-latest`.
2. **Root `npm run build` / Playwright desktop E2E:** Blocked locally because root `node_modules/` is absent (`experiments/cli-stdio-slice0/node_modules/` is installed for stdio/MCP qualification).
3. **Slice 0 desktop D-Bus experiment (`experiments/cli-stdio-slice0` / `libdbus-sys`):** Blocked locally by missing `dbus-1.pc` pkg-config headers (`libdbus-1-dev`).
4. **`Dockerfile.cli` local `docker build`:** Docker daemon is present, but a container image build pulling remote `ubuntu:22.04` / Rust toolchains over the network was intentionally not executed locally.
5. **macOS (`aarch64`/`x86_64`) & Windows native runners:** Verified in `.github/workflows/standalone-cli.yml` workflow definition; not executed natively from Linux `x86_64`.
6. **Live privileged `openvpn` route tunnel & interactive Browser OIDC popup:** Require kernel TUN/privileged network interface + live IdP browser session; PTY/subprocess/validation paths remain covered with fake fixtures and mocks.
