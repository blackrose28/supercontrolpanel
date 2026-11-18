---
name: scpctl-mcp-usage
description: Operate a running scpctl stdio MCP server (SuperControlPanel standalone CLI) safely - list/inspect servers, browse remote files over SFTP, run SCP transfers, manage server config and secrets, control OpenVPN/Boundary/Caddy/SSH sessions. Use when scpctl MCP tools (servers_list, remote_browser_*, scp_*, server_upsert, ...) are available and the user wants to use them. For installing or configuring the server, use scpctl-setup.
---

# Using the scpctl stdio MCP server

Tools are exposed per the profile's `stdio-policy.json` (11 read-only by default; 15 with `file-transfer`; 41 with `allow-local-policy` and all categories). If a tool you need is missing, do not work around it: tell the user which policy category it needs (see inventory) and point to `scpctl-setup`.

## Rules

- Never output secrets. Tool results redact them, but do not ask for, store or echo passwords/tokens/passphrases. Passwords are write-only (`server_ssh_password_set`).
- Mutations and anything that connects out need the user's confirmation first: say what will change (ids, paths, host), then act.
- Prefer read-only tools to gather facts before acting. After a timeout or lost response, inspect state (`server_get`, `servers_list`, `scp_transfer_list`) before retrying.
- Stay inside this profile. Never edit `stdio-policy.json` to escalate yourself.

## Tool inventory

| Category | Tools |
| --- | --- |
| read-only | `app_get_overview`, `function_status_list`, `function_status_get {functionId}`, `openvpn_get_status`, `boundary_get_status`, `servers_list`, `server_get {serverId}`, `config_get_paths`, `sync_settings_get`, `detect_openvpn_executable`, `boundary_search_targets {filter?, limit?}` |
| file-transfer | `remote_browser_open {serverId}` -> `browserId`; `remote_browser_list_children {browserId, path?}`; `remote_browser_stat_path`; `remote_browser_close {browserId}`; `remote_browser_create_directory`, `remote_browser_delete_path {recursive?}` (mutating); `scp_upload` / `scp_download {serverId, localPath, remotePath, recursive?}` (blocking); `scp_start_upload` / `scp_start_download` (async); `scp_transfer_list`, `scp_transfer_get {transferId}` |
| runtime-control | `openvpn_start/stop`, `boundary_auth_start/stop`, `server_tunnel_start/stop {serverId}`, `server_ssh_start/stop {serverId}`, `caddy_start {folder}`/`caddy_stop` |
| config-mutation | `server_upsert`, `server_delete`, `server_ssh_password_set/clear {serverId, password}`, `config_save`, `config_save_partial`, `sync_settings_save_partial`, `config_sync_push {repoRoot}` |

Always unavailable over stdio (return `CAPABILITY_UNAVAILABLE`): `server_rdp_start`, `config_sync_pull`, `config_git_pull`, `config_git_push`. Settings import/export is CLI-only (`scpctl settings ...`, needs a TTY for apply).

Resources: `scp://status/functions|openvpn|boundary`, `scp://servers/list`, `scp://config/paths`, and `scp://status/transfers` (needs `file-transfer`). Subscribe with `resources/subscribe` for change notifications.

## Argument shapes that are easy to get wrong

- `server_upsert` takes `{ "server": { id, name, host, os, connectionMethod ("ssh"|"boundary"), sshPort, sshUser, services:[] ... } }` (the CLI takes the flat object).
- `server_delete` takes `{ "id": "<serverId>" }`, not `serverId`. Server ids look like `srv-<uuid>`; ids with path characters are rejected.
- `config_save_partial` / `sync_settings_save_partial` take `{ "patch": {...} }`; unknown keys and desktop-authority fields (e.g. `mcpEnabled`) are rejected with `-32602 Invalid tool arguments`. Schemas are strict: extra arguments fail too.
- `scp_start_*` return `{ sessionId }`, not a transfer id. Poll `scp_transfer_list`, find the entry whose `sessionId` matches, then call `scp_transfer_get { transferId: entry.id }`.
- `server_upsert` / `server_delete` return the full resulting server list.
- Local paths must be absolute; relative local paths are rejected.
- `sync_settings_save_partial` accepts any `repoRoot` string without checking it; existence is only checked at `config_sync_push`.

## Workflows

**Inventory:** `app_get_overview` -> `servers_list` -> summarize by `connectionMethod`. `boundary` servers need a running tunnel/Boundary login; `ssh` servers connect directly.

**Browse remote files:** `remote_browser_open` -> `remote_browser_list_children` (start at `/` or home) -> `remote_browser_close` when done. Close the browser when switching servers or finishing; each browser is one persistent SFTP session.

**Transfer:** confirm direction, paths and overwrite with the user. Use `scp_start_*` for large or recursive transfers and poll as above; use blocking `scp_upload`/`scp_download` for small files. Boundary-routed servers need a running tunnel (`server_tunnel_start`) first.

**Change config:** `servers_list`/`server_get` to read the current state -> show the user the intended change -> `server_upsert` or `config_save_partial` -> re-read to verify. Config secrets are never written to disk by these tools; they live in the OS keyring.

**Runtime sessions:** `server_ssh_start`/`server_tunnel_start` -> check `function_status_get` (ids: `openvpn-connect`, `boundary-auth`, `caddy-run`, `server-tunnel-<id>`, `server-ssh-<id>`) -> stop when done.

## Errors

Tool failures return `isError: true` with a text starting `<CODE>: ...`; protocol-level problems are JSON-RPC errors.

| Result | Meaning / next step |
| --- | --- |
| `NOT_FOUND` | Unknown server/transfer/function id; re-list |
| `INVALID_ARGUMENTS` | Bad or protected field, missing file, bad id; fix input, do not retry unchanged |
| `POLICY_DENIED` | Category not allowed by policy, or policy file corrupt; ask user (scpctl-setup) |
| `CAPABILITY_UNAVAILABLE` | Often untrusted SSH host key (`host_key_untrusted`), missing keyring/binary, or an excluded op. The MCP text is generic and has no reason code. Ask the user to run the same operation via the CLI with `--json` to see `reason` (they must stop this MCP server first because of `owner.lock`, or use a second profile), or check `known_hosts` |
| `CONFLICT` | Revision/overwrite conflict, or `profile_owner_conflict`; re-read state |
| `-32602 Invalid tool arguments` / `Unknown tool` | Schema violation or tool not exposed by policy; check `tools/list` |
| `-32002` | Unknown resource or policy denied for resource |
| No reply to a malformed JSON line | Known behavior: the server ignores unparseable lines instead of returning a parse error; always send valid JSON-RPC |

If a result is unexpected, call `tools/list` to confirm the current policy before concluding a tool is broken.
