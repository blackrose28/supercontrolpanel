# SuperControlPanel MCP Tool Schemas

## Purpose

This document defines concrete MCP tool input and output shapes for the planned SuperControlPanel MCP server.

These schemas are also the contract that `supercontrolpanelhelper` should map onto.

## Response envelope

The existing desktop custom transport uses `scp-core::mcp_dto::McpToolCallEnvelope`,
re-exported by desktop `mcp::dto` and `mcp`. It serializes all five fields below,
including empty warnings and null data. The DTO has Serialize only; it does not
validate arguments/results, sanitize values, authorize calls, read the clock, or
construct failures. Desktop dispatch retains its success message, transport warning
and injected millisecond clock; action/policy failures remain `Err(String)` outside
this success envelope. `ok: false` remains representable by the DTO but is not a
new dispatcher error path. This is the custom transport contract, not a standard
MCP result or a CLI capability.

The result shape is:

```json
{
  "ok": true,
  "message": "Operation completed",
  "data": {},
  "warnings": [],
  "timestamp": 0
}
```

### Fields

- `ok`: boolean success flag
- `message`: short human-readable summary
- `data`: tool-specific payload
- `warnings`: string array, always serialized (including when empty)
- `timestamp`: Unix epoch milliseconds

## Shared data shapes

### FunctionStatus

```json
{
  "functionId": "boundary-auth",
  "state": "running",
  "message": "Boundary login in progress",
  "boundaryTokenSource": "app",
  "sessionId": "abc123",
  "updatedAtMs": 0
}
```

### StartFunctionResult

```json
{
  "functionId": "openvpn-connect",
  "sessionId": "abc123"
}
```

### Server

```json
{
  "id": "server-1",
  "name": "Prod Jump",
  "host": "10.0.0.10",
  "os": "linux",
  "connectionMethod": "boundary",
  "sshPort": 22,
  "sshUser": "ubuntu",
  "boundaryTargetId": "ttcp_123",
  "boundaryListenPort": 2201,
  "services": [
    { "name": "app", "port": 443 }
  ]
}
```

### BoundaryTargetSummary

```json
{
  "id": "ttcp_123",
  "name": "prod-api",
  "targetType": "ssh",
  "description": "Production API host",
  "address": "10.0.0.10"
}
```

## Tool schemas

## `app_get_overview`

### Input

```json
{}
```

### Output `data`

```json
{
  "appVersion": "0.1.0",
  "activeFunctions": 2,
  "knownFunctionCount": 3
}
```

The existing custom dispatcher ignores all arguments, including non-object values.
`activeFunctions` counts stored statuses with a non-null session ID (including an
empty string), regardless of state. `knownFunctionCount` counts all stored statuses;
this is not a count of PTYs or a filtered list of visible UI cards. The version
comes from desktop package metadata. All three fields are always present. The
Serialize-only `scp-core::mcp_dto::McpAppOverviewResult` carries the result; hosts
retain count/version construction and policy enforcement. The older example's
`mcpEnabled`, `activeFunctionCount`, `activeSessionCount` and `functions` fields
were documentation drift, not fields removed from production.

## `function_status_list`

### Input

```json
{}
```

The custom dispatcher ignores arguments for this operation, including non-objects.

### Output `data`

```json
[]
```

A bare array of `FunctionStatus` objects, not an `items` wrapper. The shared
`scp-core::mcp_dto::McpFunctionStatusListResult` aliases a vector of the existing
runtime status type. Store iteration order is retained; no sorting/filtering is
introduced and no stable order is promised.

## `function_status_get`

### Input

```json
{
  "functionId": "boundary-auth"
}
```

`McpFunctionStatusGetArgs::from_args` preserves the custom dispatcher's existing
lookup: missing, null, non-string values and non-object inputs return
`functionId is required`. Any string is accepted without trimming, including empty
strings and control characters; extra fields are ignored. An absent store entry
returns `function status not found`. Policy runs before argument parsing.

### Output `data`

```json
{
  "functionId": "boundary-auth",
  "state": "idle",
  "message": "",
  "boundaryTokenSource": null,
  "sessionId": null,
  "updatedAtMs": 0
}
```

A bare `FunctionStatus`, not an `item` wrapper. The shared
`McpFunctionStatusGetResult` aliases the existing runtime type. All six fields
serialize, including null optional fields, and string values and the full u64
timestamp are retained. These Serialize-only argument/result contracts add no
execution handler, CLI operation or generated standard MCP JSON Schema.

## `openvpn_get_status`

### Input

```json
{}
```

All arguments are ignored, including non-object values. A supplied `functionId`
does not change the host's fixed `openvpn-connect` lookup.

### Output `data`

```json
{
  "functionId": "openvpn-connect",
  "state": "stopped",
  "message": "",
  "boundaryTokenSource": null,
  "sessionId": null,
  "updatedAtMs": 0
}
```

A bare `FunctionStatus`, or JSON `null` when no status is stored under the fixed
ID; there is no `status` wrapper or not-found error. The shared
`McpOpenVpnGetStatusResult` aliases `Option<FunctionStatus>`. All six status
fields serialize unchanged, including null optional fields, arbitrary strings
and the full u64 timestamp. The result type does not validate the stored
`functionId`, state or token source, or synthesize a stopped status. Policy and
locking remain host responsibilities; no CLI status command is added.

## `openvpn_start`

### Input

```json
{
  "hidden": false
}
```

### Output `data`

```json
{
  "result": {}
}
```

Where `result` is a `StartFunctionResult`.

## `openvpn_stop`

### Input

```json
{}
```

### Output `data`

```json
{
  "stopped": true
}
```

## `boundary_get_status`

### Input

```json
{}
```

All arguments are ignored, including non-object values. A supplied `functionId`
does not change the host's fixed `boundary-auth` lookup.

### Output `data`

```json
{
  "functionId": "boundary-auth",
  "state": "stopped",
  "message": "",
  "boundaryTokenSource": null,
  "sessionId": null,
  "updatedAtMs": 0
}
```

A bare `FunctionStatus`, or JSON `null` when no status is stored under the fixed
ID; there is no `status` wrapper or not-found error. The shared
`McpBoundaryGetStatusResult` aliases `Option<FunctionStatus>`. All six status
fields serialize unchanged, including null optional fields, arbitrary strings
and the full u64 timestamp. The result type does not validate the stored
`functionId`, state or token source, or synthesize a stopped status. Policy and
locking remain host responsibilities; no CLI status command is added.

## `boundary_auth_start`

### Input

```json
{
  "hidden": true
}
```

### Output `data`

```json
{
  "result": {}
}
```

Where `result` is a `StartFunctionResult`.

## `boundary_auth_stop`

### Input

```json
{}
```

### Output `data`

```json
{
  "stopped": true
}

## `server_upsert`

### Input

```json
{
  "server": {}
}
```

Where `server` is a full `Server` object. An empty `id` creates a new server; a non-empty `id` updates an existing server.

### Output `data`

```json
[]
```

Where the returned value is the full updated server list.

## `server_delete`

### Input

```json
{
  "id": "srv-123"
}
```

### Output `data`

```json
[]
```

Where the returned value is the full updated server list.

## `config_save`

### Input

```json
{
  "config": {}
}
```

Where `config` is a full `AppConfig` object.

### Output `data`

```json
{}
```

Where the returned value is the saved `AppConfig`.

## `config_save_partial`

### Input

```json
{
  "patch": {
    "boundary": {
      "addr": "https://boundary.example.com"
    }
  }
}
```

The patch is merged into the current in-memory `AppConfig` shape before saving.

### Output `data`

```json
{}
```

Where the returned value is the saved `AppConfig`.

## `sync_settings_get`

### Input

```json
{}
```

### Output `data`

```json
{}
```

Where the returned value is the full device-local sync settings object from `sync-local.json`.

## `sync_settings_save_partial`

### Input

```json
{
  "patch": {
    "repoRoot": "/path/to/repository",
    "caddyFolder": "/path/to/caddy"
  }
}
```

`patch` must be a JSON object. Only `repoRoot`, `openvpnExecutablePath`,
`openvpnOvpnPath`, `boundaryCliPath` and `caddyFolder` are accepted; their existing
partial-update semantics are preserved. Omitted/null allowed fields do not change
stored values. Any other field rejects the entire patch before the save action,
even if null or mixed with allowed fields. Use desktop settings for MCP/CC policy,
tokens, listener, identity and enrollment controls. This restriction applies even
under `allow-with-token` and also to the Tauri MCP command; the desktop-local
`save_sync_settings` command retains the full persistence schema.

### Output `data`

```json
{}
```

Where the returned value is the full saved device-local sync settings object.

## `remote_browser_open`

`McpRemoteBrowserOpenArgs` reuses the shared server-ID argument contract: any
string `serverId` passes unchanged; missing, non-string or non-object arguments
fail with `serverId is required`. Extra fields are ignored. The bare result is
`McpRemoteBrowserOpenResult`, an alias of the shared `RemoteBrowseSession` wire
value (Serialize + Clone). It owns no connection. Desktop allocation, replacement
of prior browser sessions, credentials, SFTP/root resolution and errors remain
unchanged; policy still runs first. No CLI browser operation is enabled.

### Input

```json
{
  "serverId": "srv-123"
}
```

### Output `data`

```json
{
  "browserId": "browser-1",
  "rootPath": "/home/ubuntu"
}
```

## `remote_browser_close`

Shared `McpRemoteBrowserCloseArgs` accepts any string `browserId` unchanged,
including empty/whitespace/control text; other shapes fail with
`browserId is required`. Extras are ignored. `McpRemoteBrowserCloseResult`
serializes to exactly `{}` on success. Desktop exact-ID removal, absent-ID success,
locking/cleanup errors, policy and envelopes retain their existing behavior.

### Input

```json
{
  "browserId": "browser-1"
}
```

### Output `data`

```json
{}
```

## `remote_browser_list_children`

The list and stat tools share `scp-core::mcp_dto::McpRemoteBrowserReadArgs`.
`browserId` must be a string (including empty/whitespace/control text), otherwise
`browserId is required`. Missing, null or any other non-string `path` becomes
`""`; string paths are passed through unchanged. Extra arguments are ignored.
Desktop policy runs first. The host then looks up the persistent session and
normalizes the path relative to its root; this DTO does not validate paths.
Results use shared entry/node models with exactly the fields shown below and
lowercase kinds `file`, `directory`, `symlink`, `other`, `missing`.
The DTO preserves host enumeration order and duplicates; desktop enumeration
retains its existing filtering. Session/SFTP/path errors and the custom envelope
are unchanged. These types do not enable standalone browser execution.

### Input

```json
{
  "browserId": "browser-1",
  "path": "/home/ubuntu"
}
```

### Output `data`

```json
[
  {
    "name": "logs",
    "path": "/home/ubuntu/logs",
    "kind": "directory"
  }
]
```

## `remote_browser_stat_path`

### Input

```json
{
  "browserId": "browser-1",
  "path": "/home/ubuntu/app.log"
}
```

### Output `data`

```json
{
  "path": "/home/ubuntu/app.log",
  "kind": "file",
  "exists": true
}
```

## `remote_browser_create_directory`

### Input

```json
{
  "browserId": "browser-1",
  "path": "/home/ubuntu/new-folder"
}
```

### Output `data`

```json
{}
```

`McpRemoteBrowserCreateDirectoryArgs` requires a string `browserId` first,
then a string `path`. Missing, null or non-string values produce exactly
`browserId is required` or `path is required`, in that order; non-object inputs
fail on `browserId`. Empty, whitespace, Unicode and control text are accepted
unchanged and borrowed; extra fields are ignored. The desktop still performs
session lookup, root-relative path validation and SFTP mkdir with mode 0755.
`McpRemoteBrowserCreateDirectoryResult` aliases the existing close `{}` result.

## `remote_browser_delete_path`

### Input

```json
{
  "browserId": "browser-1",
  "path": "/home/ubuntu/old-folder",
  "recursive": true
}
```

### Output `data`

```json
{}
```

`McpRemoteBrowserDeletePathArgs` reuses the create-directory required-string
parser and error order. `recursive` preserves true/false; missing, null and all
non-boolean values become `None` without coercion. Its serialized argument value
includes `recursive: null` when absent; request parsing does not require the key.
`McpRemoteBrowserDeletePathResult` also aliases the close `{}` result. Desktop
execution still defaults absent recursion to false, looks up the exact browser
session before normalizing the path against its root, and opens SFTP. Recursive
removal and nonrecursive lstat/directory-versus-file deletion, errors, policy-first
dispatch and custom envelopes remain unchanged. These DTOs enable no CLI actions.

## `scp_upload`

### Input

```json
{
  "serverId": "srv-123",
  "localPath": "/tmp/report.txt",
  "remotePath": "/home/ubuntu/report.txt",
  "recursive": false
}
```

### Output `data`

```json
{
  "result": "Upload completed"
}
```

## `scp_download`

### Input

```json
{
  "serverId": "srv-123",
  "remotePath": "/home/ubuntu/report.txt",
  "localPath": "/tmp/report.txt",
  "recursive": false
}
```

### Output `data`

```json
{
  "result": "Download completed"
}
```

## `scp_start_upload`

### Input

```json
{
  "serverId": "srv-123",
  "localPath": "/tmp/reports",
  "remotePath": "/home/ubuntu/reports",
  "recursive": true
}
```

### Output `data`

```json
{
  "sessionId": "pty-123"
}
```

## `scp_start_download`

### Input

```json
{
  "serverId": "srv-123",
  "remotePath": "/home/ubuntu/reports",
  "localPath": "/tmp/reports",
  "recursive": true
}
```

### Output `data`

```json
{
  "sessionId": "pty-123"
}

## `scp_transfer_list`

All arguments, including non-object values, are ignored after the existing policy
check. Results clone the host history in its current order (new entries are
inserted first; history retains at most 32 entries); list adds no sorting,
filtering or deduplication. Store lock errors propagate unchanged.

### Input

```json
{}
```

### Output `data`

```json
[
  {
    "id": "scp-123",
    "title": "Upload",
    "detail": "Transferring… 42%",
    "state": "pending",
    "createdAtMs": 0,
    "updatedAtMs": 0,
    "progressPercent": 42,
    "source": "/tmp/report.txt",
    "destination": "/home/ubuntu/report.txt",
    "sessionId": "pty-123"
  }
]
```

## `scp_transfer_get`

After policy, `transferId` uses the existing string lookup: missing, null,
non-string and non-object arguments return `transferId is required`. Empty,
whitespace, Unicode and control strings are accepted unchanged; extra fields are
ignored. The host returns the first exact ID match or `transfer not found`;
store lock errors propagate unchanged.

Both transfer tools return the Serialize-only `ScpTransferEntry` model through
shared MCP result aliases. All ten fields are emitted. `progressPercent`,
`source`, `destination` and `sessionId` emit explicit nulls when absent; present
strings are unchanged. States are exactly `pending`, `success`, `error`.
Timestamps preserve `u64` values and progress preserves `u8` values without
DTO validation (including values above 100); hosts retain update semantics.
The existing custom envelope and execution policy remain unchanged.

### Input

```json
{
  "transferId": "scp-123"
}
```

### Output `data`

```json
{
  "id": "scp-123",
  "title": "Upload",
  "detail": "Transfer finished.",
  "state": "success",
  "createdAtMs": 0,
  "updatedAtMs": 0,
  "progressPercent": 100,
  "source": "/tmp/report.txt",
  "destination": "/home/ubuntu/report.txt",
  "sessionId": "pty-123"
}
```
```
```

## `boundary_search_targets`

### Input

```json
{
  "filter": "prod"
}
```

### Output `data`

```json
{
  "items": []
}
```

Where `items` is an array of `BoundaryTargetSummary`.

## `servers_list`

### Input

`{}` is conventional. All arguments are ignored, including filters and non-objects.

### Output `data`

A bare array of `Server` records, or `[]` for an empty inventory, with loader
order and duplicates preserved. `McpServersListResult` aliases `Vec<Server>`;
there is no `items` wrapper, filtering, sorting or deduplication.

## `server_get`

### Input

```json
{"serverId":"server-1"}
```

Serialize-only borrowed `McpServerGetArgs` preserves the existing
`.get("serverId").and_then(Value::as_str)` lookup: extra fields are ignored;
empty, whitespace, Unicode and control strings are accepted without normalization.
Missing/null/non-string IDs and non-object inputs fail with `serverId is required`.

### Output `data`

A bare `Server` object. `McpServerGetResult` aliases the existing shared inventory
model; there is no `item` wrapper. The host loads inventory and returns the first
exact ID match, or `server not found`. Loader errors still propagate.

Both result aliases serialize all ten existing camelCase fields: `id`, `name`,
`host`, `os` (`linux`/`windows`), `connectionMethod` (`ssh`/`boundary`), `sshPort`,
`sshUser`, `boundaryTargetId`, `boundaryListenPort`, and `services` (ordered
`{name, port}` records, including duplicates). Empty strings/arrays and zero ports
remain explicit. These aliases inherit the inventory model's existing derives;
they add no validation, wrappers or credential lookup. Both tools retain the
existing policy-first dispatch and custom success envelope. The desktop loader
retains its settings-maintenance gate and path creation/read behavior; it is not
replaced by the standalone read-only inventory reader.

## `server_tunnel_start`

### Input

```json
{
  "serverId": "server-1",
  "cols": 120,
  "rows": 30,
  "hidden": true
}
```

### Output `data`

```json
{
  "result": {}
}
```

Where `result` is a `StartFunctionResult`.

## `server_tunnel_stop`

### Input

```json
{
  "serverId": "server-1"
}
```

### Output `data`

```json
{
  "stopped": true
}
```

## `server_ssh_start`

### Input

```json
{
  "serverId": "server-1",
  "cols": 120,
  "rows": 30,
  "hidden": true
}
```

### Output `data`

```json
{
  "result": {}
}
```

Where `result` is a `StartFunctionResult`.

## `server_ssh_stop`

### Input

```json
{
  "serverId": "server-1"
}
```

### Output `data`

```json
{
  "stopped": true
}
```

## `config_get_paths`

Arguments are ignored, including extra fields and non-object values. Policy is
checked before the host callback runs. The resource `scp://config/paths` delegates
to this tool with `{}`; the helper sends `{}` and prints the result envelope.

### Input

```json
{}
```

### Output `data`

```json
{
  "appConfigPath": "/path/to/config.json"
}
```

The bare object uses `McpConfigGetPathsResult`, an alias of the Serialize-only
`scp-core::config::ConfigPaths` model. Its sole field is a host-supplied string;
the DTO does not trim, normalize, validate or resolve it. Desktop HTTP and Tauri
hosts resolve the app config directory, create it if needed, append `config.json`
and use the path's lossy string conversion. Resolution/directory-creation errors
propagate unchanged; policy and the custom response envelope remain host-owned.
This result does not describe CLI profile paths or initialize a CLI profile.

## `config_sync_pull`

### Input

```json
{
  "repoRoot": "/path/to/repo"
}
```

### Output `data`

```json
{}
```

## `config_sync_push`

### Input

```json
{
  "repoRoot": "/path/to/repo"
}
```

### Output `data`

```json
{
}
```

## `config_git_pull`

### Input

```json
{
  "repoRoot": "/path/to/repo"
}
```

### Output `data`

```json
{
  "result": "Already up to date."
}
```

## `config_git_push`

### Input

```json
{
  "repoRoot": "/path/to/repo"
}
```

### Output `data`

```json
{
  "result": "No changes to commit."
}
```

## Helper mapping notes

`supercontrolpanelhelper` should map commands to MCP calls directly.

### Examples

- `supercontrolpanelhelper status`
  - `app_get_overview`
- `supercontrolpanelhelper function list`
  - `function_status_list`
- `supercontrolpanelhelper boundary login start --hidden`
  - `boundary_auth_start` with `{ "hidden": true }`
- `supercontrolpanelhelper server tunnel start --server-id server-1`
  - `server_tunnel_start`

## Schema rules

- Keep field names camelCase in JSON payloads
- Prefer explicit booleans over inferred state
- Include ids and timestamps where useful
- Keep outputs small and structured
- Redact secrets in all outputs
