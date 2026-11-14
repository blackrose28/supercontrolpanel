# Standalone CLI stdio MCP Slice 0 fixture

Disposable platform-spike and `stdio` MCP protocol feasibility crate, isolated from
desktop `src-tauri/` and production workspace crates (`scp-core`, `scp-mcp`, `scp-cli`).
No desktop settings, real credentials, `/usr/bin/sudo` calls, or desktop keyring
entries are read or modified by these tests.

## Subcommands & probe surface

- Platform spike CLI helpers (exercised by `tests/platform_spikes.rs`):
  `hold-lock`, `keyring-probe`, `supervisor-daemon`, `vpn-lock`, `boundary-probe`.
- Disposable MCP `stdio` server (`serve-stdio`): exposes only `probe_echo`,
  `probe_hold`, `probe://status`, and `probe://active` over `rmcp 3.9.0`
  (`stdio_pipes` + `AsyncRwTransport`). No HTTP listener or Tauri runtime exists.

## Run from repository root

```sh
cargo build --locked --manifest-path experiments/cli-stdio-slice0/Cargo.toml
cargo test --locked --manifest-path experiments/cli-stdio-slice0/Cargo.toml
npm ci --prefix experiments/cli-stdio-slice0 --ignore-scripts --no-audit --no-fund
npm test --prefix experiments/cli-stdio-slice0
```

Requirements verified on Linux `x86_64`: `rustc 1.94.0`, `cargo 1.94.0`, `node v24.14.0`,
`npm 11.12.1`, `/usr/bin/boundary 0.19.1`, `dbus-run-session`, `gnome-keyring-daemon`,
and `secret-tool`. Keyring and Boundary isolation tests run inside ephemeral
`dbus-run-session` sessions with private temporary `XDG_DATA_HOME` / `XDG_RUNTIME_DIR`.
VPN tests inject disposable scripts in `tempdir` instead of invoking host `sudo` or
`openvpn`.

## Manual Inspector CLI check

```sh
node experiments/cli-stdio-slice0/node_modules/@modelcontextprotocol/inspector/clients/launcher/build/index.js \
  --cli --transport stdio --protocol-era modern --method tools/list --format json --quiet \
  -- experiments/cli-stdio-slice0/target/debug/scp-cli-stdio-slice0 serve-stdio
```

Use `--protocol-era legacy` for the `2025-11-25` handshake path.
