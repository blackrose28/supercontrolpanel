# SuperControlPanel MCP Tool Catalog

## Standalone stdio catalog (Slices 4–5)

`scpctl mcp serve --stdio` reuses the same `45`-operation / `6`-resource (`scp://*`) catalog under `stdio-policy.json`: `41` `Capability::Runtime` operations are callable when all categories and `"allow-local-policy"` are configured (`11` under default `read-only`, `15` with `file-transfer`), while `4` operations (`server_rdp_start`, `config_sync_pull`, `config_git_pull`, `config_git_push`) remain excluded with `CAPABILITY_UNAVAILABLE`. See [Standalone CLI Agent Guide](STANDALONE_CLI_AGENT_GUIDE.md), [Slice 4 report](STANDALONE_CLI_SLICE4.md), and [Slice 5 report](STANDALONE_CLI_SLICE5.md).

## Slice 6 qualification

Config now exposes the full `/mcp` URL, separate token copy, tested Inspector CLI 2.14.0 examples and runtime refresh/retry. [Slice 6](DESKTOP_STREAMABLE_HTTP_SLICE6.md) records exact client/platform evidence and remaining release gates. Standard/REST schemas, shared category/sensitivity policy, helper base-URL/token discovery and desktop command/event contracts are unchanged. Compatibility routes remain through at least one stable release containing `/mcp`, with removal requiring separately reviewed helper/consumer migration. Cross-platform and untested product-client support are not implied.


## Implemented catalog (Slice 5)

Standard `/mcp` adapts all 45 registry operations and six fixed resources (the original five plus `scp://status/transfers`). Discovery uses current category, sensitive-action and capability policy: at most 42 tools are visible; `config_sync_pull`, `config_git_pull` and `config_git_push` remain unavailable. Slice 5 adds `boundary_search_targets`, `detect_openvpn_executable`, `server_rdp_start`, `server_ssh_password_set` and `server_ssh_password_clear`. RDP has an interactive-desktop capability check at invocation; SSH credentials are write-only. See [Slice 5 contracts/policy](DESKTOP_STREAMABLE_HTTP_SLICE5.md). [Slice 4](DESKTOP_STREAMABLE_HTTP_SLICE4.md) adds status/transfer snapshot subscriptions and bounded transport execution. See [Slice 3](DESKTOP_STREAMABLE_HTTP_SLICE3.md) and the [per-operation test matrix](DESKTOP_STREAMABLE_HTTP_INVENTORY.md).

## Purpose

This document defines the proposed MCP tool surface for SuperControlPanel so AI agents can inspect state and operate the running app.

## Naming convention

Use stable snake_case names that describe the action clearly.

Examples:

- `app_get_overview`
- `function_status_list`
- `openvpn_start`
- `boundary_auth_start`

## Tool groups

## App and status

- `app_get_overview`
  - Returns app version, MCP status, active functions, and active sessions summary
- `function_status_list`
  - Returns all current function statuses
- `function_status_get`
  - Returns the status for one function id
- `events_recent`
  - Returns recent app/runtime events in summarized form

## OpenVPN

- `openvpn_get_status`
- `openvpn_get_config_summary`
- `openvpn_detect_executable`
- `openvpn_start`
- `openvpn_stop`

## Boundary

- `boundary_get_status`
- `boundary_get_config_summary`
- `boundary_auth_start`
- `boundary_auth_stop`
- `boundary_search_targets`

## Servers

- `servers_list`
- `server_get`
- `server_upsert`
- `server_delete`
- `server_tunnel_start`
- `server_tunnel_stop`
- `server_ssh_start`
- `server_ssh_stop`
- `server_rdp_start`

## Remote browser

- `remote_browser_open`
- `remote_browser_close`
- `remote_browser_list_children`
- `remote_browser_stat_path`
- `remote_browser_create_directory`
- `remote_browser_delete_path`

## SCP

- `scp_upload`
- `scp_download`
- `scp_start_upload`
- `scp_start_download`
- `scp_transfer_list`
- `scp_transfer_get`

## Config

- `config_get`
- `config_get_paths`
- `config_save`
- `config_save_partial`
- `sync_settings_get`
- `sync_settings_save_partial`

## Config sync and Git

- `config_sync_pull`
- `config_sync_push`
- `config_git_pull`
- `config_git_push`

## Caddy

- `caddy_get_status`
- `caddy_start`
- `caddy_stop`

## Recommended v1 scope

Start with the highest-value tools that match the current app’s runtime contract:

- `app_get_overview`
- `function_status_list`
- `openvpn_get_status`
- `openvpn_start`
- `openvpn_stop`
- `boundary_get_status`
- `boundary_auth_start`
- `boundary_auth_stop`
- `boundary_search_targets`
- `servers_list`
- `server_get`
- `server_tunnel_start`
- `server_tunnel_stop`
- `server_ssh_start`
- `server_ssh_stop`
- `config_get_paths`
- `config_sync_pull`
- `config_sync_push`

## Response shape

Prefer a standard result envelope:

```json
{
  "ok": true,
  "message": "Operation completed",
  "data": {},
  "warnings": [],
  "timestamp": 0
}
```

## Tool sensitivity classes

Each tool should be assigned one of these sensitivity classes:

- `read_only`
- `runtime_control`
- `write_config`
- `destructive`
- `secret_access`

### Examples

- `function_status_list` → `read_only`
- `openvpn_start` → `runtime_control`
- `server_upsert` → `write_config`
- `remote_browser_delete_path` → `destructive`

## Data handling rules

- Never return keyring secrets by default
- Redact passwords and OTP secrets in config responses
- Include stable ids and timestamps in action results
- Prefer idempotent behavior for repeated start/stop requests where possible
