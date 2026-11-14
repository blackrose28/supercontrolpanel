# Standalone CLI and Stdio MCP Agent Guide

`scpctl` runs headless and GUI-free with the Super Control Panel desktop app closed. Both automation paths—short-lived shell invocations (`--json --non-interactive`) and persistent stdio MCP sessions (`scpctl mcp serve --stdio`)—share `scp-core` invariants, profile locking, SSH known-hosts checks, and atomic config/revision validation.

## Recommended agent loop

1. **Discover contracts dynamically:**
   ```bash
   scpctl schema --json
   scpctl schema --command scp.upload --json
   ```
   `scpctl schema --command` expects dotted command identifiers (for example `scp.upload`, `server.upsert`, `mcp.serve`). Currently `schema --json` exports `31` CLI commands, `45` MCP operation descriptors (`41` callable on standalone stdio + `4` excluded operations), and `6` `scp://*` resources.
2. **Probe profile and host readiness:**
   ```bash
   scpctl --config-dir ~/.config/scpctl --profile ops capabilities --json --non-interactive
   ```
   Check `data.keyring.state` (`"available"` vs `"unavailable"`), `data.availableOperations`, and `data.stdioPolicy`.
3. **Preview before mutating:**
   ```bash
   scpctl --config-dir ~/.config/scpctl --profile ops \
     server upsert --file server.json --preview --json --non-interactive
   ```
   Capture `data.revision` from the preview response.
4. **Apply with optimistic concurrency:**
   ```bash
   scpctl --config-dir ~/.config/scpctl --profile ops \
     server upsert --stdin --expected-revision "<REV>" --json --non-interactive < server.json
   ```
5. **Recover safely after timeouts or lost responses:**
   Inspect current state before retrying a timed-out mutation or transfer:
   - CLI: `server get --server-id <ID> --json`, `server list --json`, `status --json`, or re-run `--preview`.
   - Stdio MCP: `server_get`, `scp_transfer_list`, `scp_transfer_get`, or `scp://status/*` resources.
   Retrying with a stale `--expected-revision` deterministically fails with exit `5` (`CONFLICT`).

## Headless and non-interactive pre-provisioning

- **Isolated CLI profile:** Always keep standalone CLI state separate from any desktop profile directory. Configure with `--config-dir <DIR>` (or `SCPCTL_CONFIG_DIR`) and `--profile <NAME>` (or `SCPCTL_PROFILE`). Inside `--non-interactive` mode, missing target profiles are never auto-created for mutations (`scpctl capabilities --no-profile --json` probes without creating profile state).
- **Single-writer lock (`owner.lock`):** One stdio MCP host (`mcp serve --stdio`) holds `owner.lock` for the lifetime of the stdio session. Concurrent shell invocations or a second stdio server targeting the same profile exit cleanly with code `5` (`CONFLICT`, `reason: "profile_owner_conflict"`, `nextAction: "use_owner_connection"`).
- **SSH host verification and identity keys:** Headless `ssh` / `scp` / persistent SFTP operations run with `StrictHostKeyChecking=yes` (`BatchMode=yes`). Provision host keys ahead of time in `~/.ssh/known_hosts` or set `SCPCTL_SSH_KNOWN_HOSTS=<PATH>`. Optionally pass `SCPCTL_SSH_IDENTITY_FILE=<PATH>` for non-interactive SSH-key targets.
- **OS keyring vs GUI-free Secret Service:** `scpctl` uses `OsKeyringBackend` (Linux Secret Service over D-Bus, macOS Keychain, Windows Credential Manager). On headless Linux hosts without `org.freedesktop.secrets`, SSH-key targets, SCP/SFTP workflows, non-secret server/config CRUD, and `config sync push` still succeed. Keyring-dependent operations (`config secret set`, `server credentials set`, and `server delete` / MCP `server_delete`, which inspect and clear any stored SSH password slot fail-closed) exit `4` (`CAPABILITY_UNAVAILABLE`, `reason: "keyring_unavailable"`), and un-mocked `settings export` snapshot failures exit `1` (`OPERATION_FAILED`, `reason: "backup_snapshot_failed"`).

## CLI JSON envelope, exit codes, and payload contracts

With `--json`, `scpctl` writes a single JSON line to stdout and routes diagnostics/logs exclusively to stderr:

```json
{"ok":true,"schemaVersion":1,"runtimeId":"...","data":{},"error":null}
{"ok":false,"schemaVersion":1,"runtimeId":null,"data":null,"error":{"code":"CONFLICT","reason":"profile_owner_conflict","message":"...","retryable":false,"outcome":"not_started","nextAction":"use_owner_connection"}}
```

Deterministic process exit codes (from `CoreError::exit_code` in [`crates/scp-core/src/error.rs`](../../crates/scp-core/src/error.rs)):

| Exit code | Error code | Meaning |
| --- | --- | --- |
| `0` | — | Success |
| `1` | `OPERATION_FAILED` (or other runtime errors) | Runtime, snapshot, or backend failure |
| `2` | `INVALID_ARGUMENTS` | Validation or argument parse error |
| `3` | `POLICY_DENIED` | Local safety or stdio policy rejection |
| `4` | `CAPABILITY_UNAVAILABLE` | Interactive TTY, GUI host, keyring, or binary missing |
| `5` | `CONFLICT` | Revision mismatch, overwrite guard, or `owner.lock` busy |
| `6` | `NOT_FOUND` | Unknown server, transfer, function, or profile ID |
| `7` | `TIMEOUT` | Bounded operation or shutdown timeout expired |
| `130` | `INTERRUPTED` | Process interrupted via `SIGINT` / cancellation |

**Important CLI vs MCP shape differences:**
- **Server upsert:** CLI `scpctl server upsert --file/--stdin` accepts the flat server object (`{ id, name, host, ... }`); stdio MCP tool `server_upsert` wraps it in `{ server: { id, name, host, ... } }`.
- **Async SCP transfer IDs:** `scp_start_upload` and `scp_start_download` return `{ sessionId }`. Poll `scp_transfer_list` (or subscribe/read `scp://status/transfers`) matching `sessionId`, then fetch the transfer record via `scp_transfer_get({ transferId: match.id })`.
- **Interactive-only CLI commands:** `settings import --apply --interactive` and `server rdp --interactive` require an interactive terminal (or desktop UI). Under `--non-interactive`, both exit `4` with `CAPABILITY_UNAVAILABLE` (`server rdp --non-interactive --json` returns structured JSON; `settings import --apply --non-interactive` prints `CAPABILITY_UNAVAILABLE` on stderr and exits `4`).

## Stdio MCP policy, framing, and transport bounds

`scpctl mcp serve --stdio` reads `<profileDir>/stdio-policy.json` live on every `tools/list`, `tools/call`, and `resources/read` without restarting the process:

```json
{
  "sensitiveActionPolicy": "read-only",
  "allowedToolCategories": ["read-only"]
}
```

- **Default policy (when file is absent):** `sensitiveActionPolicy: "read-only"` with `allowedToolCategories: ["read-only"]` exposes **`11` read-only tools** and **`6` `scp://*` resources**. Adding `"file-transfer"` under `"read-only"` adds the **`4` remote SFTP browser read tools** (`remote_browser_open`, `remote_browser_close`, `remote_browser_list_children`, `remote_browser_stat_path`) for **`15` tools**.
- **Full local automation policy:** `"sensitiveActionPolicy": "allow-local-policy"` with `["read-only", "runtime-control", "config-mutation", "file-transfer"]` exposes **`41` tools** and all **`6` resources** (`scp://config/paths`, `scp://servers/list`, `scp://status/boundary`, `scp://status/functions`, `scp://status/openvpn`, `scp://status/transfers`).
- **Fail-closed & separation from `sync-local.json`:** Corrupt or unreadable `stdio-policy.json` immediately drops exposed tools to `0` and rejects calls with `POLICY_DENIED:`. `sync_settings_save_partial` exposes only `{ patch: { repoRoot } }` (`additionalProperties: false`), so attempts to pass desktop/policy fields such as `mcpEnabled` or `mcpSensitiveActionPolicy` are rejected at schema validation (`-32602 Invalid tool arguments`), CLI `settings patch` / core sync validation also forbids desktop service and token escalation, and `stdio-policy.json` stays independent of desktop HTTP policy in `sync-local.json`.
- **Excluded stdio operations (`4`):** `server_rdp_start`, `config_sync_pull`, `config_git_pull`, and `config_git_push` remain excluded on stdio and return `isError: true` with `CAPABILITY_UNAVAILABLE:` and host-neutral remediation (`open_desktop`, `use_cli_import`, `use_cli_git`).
- **Framing and resource bounds:** Transport is newline-delimited UTF-8 JSON-RPC over stdin/stdout (never write logs to stdout). Runtime bounds are configurable via env vars:
  - `SCP_STDIO_MAX_LINE_BYTES`: default `1048576` bytes (minimum `256`)
  - `SCP_STDIO_WRITE_TIMEOUT_MS`: default `5000` ms (minimum `50`)
  - `SCP_STDIO_MAX_WORKERS` (alias `SCP_STDIO_MAX_CONCURRENT`): default `16`, clamped `1..=64`
  - `SCP_STDIO_MAX_SUBSCRIPTIONS`: default `8`, clamped `1..=64`

## MCP client configurations and runnable examples

### Generic `mcpServers` JSON config

```json
{
  "mcpServers": {
    "supercontrolpanel-stdio": {
      "command": "scpctl",
      "args": ["--config-dir", "/path/to/cli-config", "--profile", "ops", "mcp", "serve", "--stdio"],
      "env": {
        "SCPCTL_SSH_KNOWN_HOSTS": "/path/to/known_hosts"
      }
    }
  }
}
```

### Verified SDK and Inspector invocations

- **MCP Inspector CLI (`@modelcontextprotocol/inspector@2.14.0`, modern & legacy protocol eras):**
  ```bash
  npx @modelcontextprotocol/inspector@2.14.0 --cli --transport stdio \
    --protocol-era modern --format json --quiet \
    --method tools/call --tool-name server_get \
    --tool-args-json '{"serverId":"my-server"}' \
    -- scpctl --config-dir /path/to/cli-config --profile ops mcp serve --stdio
  ```
  Pass `--protocol-era legacy` for older MCP client handshake compatibility.

- **`@modelcontextprotocol/client@2.4.0` (modern TypeScript/Node client):**
  ```js
  import { Client } from '@modelcontextprotocol/client';
  import { StdioClientTransport } from '@modelcontextprotocol/client/stdio';

  const client = new Client(
    { name: 'ops-agent', version: '1.0.0' },
    { capabilities: {}, versionNegotiation: { mode: { pin: '2026-07-28' } } },
  );
  await client.connect(
    new StdioClientTransport({
      command: 'scpctl',
      args: ['--config-dir', cliRoot, '--profile', 'ops', 'mcp', 'serve', '--stdio'],
      stderr: 'pipe',
    }),
  );
  ```

- **`@modelcontextprotocol/sdk@1.32.2` (classic SDK client + resource subscriptions):**
  See [`experiments/cli-stdio-slice0/tests/slice4_production_stdio.mjs`](../../experiments/cli-stdio-slice0/tests/slice4_production_stdio.mjs) for live `stdio-policy.json` reload, `notifications/resources/updated` subscriptions, SFTP/SCP, Boundary, Caddy, and OpenVPN orchestration.

### Runnable end-to-end workflows in this repository

Both runnable examples provision an isolated temp profile, start the local `sshd_fixture_daemon`, exercise real SSH/SFTP/SCP operations against `scpctl`, and tear down cleanly:

- Shell automation (`--json --non-interactive`):
  ```bash
  node examples/standalone-cli/shell-agent-workflow.mjs \
    --scpctl ./target/release/scpctl \
    --sshd-fixture ./target/release/examples/sshd_fixture_daemon
  ```
- Stdio MCP workflow (`@modelcontextprotocol/client@2.4.0`):
  ```bash
  node examples/standalone-cli/stdio-mcp-agent-workflow.mjs \
    --scpctl ./target/release/scpctl \
    --sshd-fixture ./target/release/examples/sshd_fixture_daemon
  ```
