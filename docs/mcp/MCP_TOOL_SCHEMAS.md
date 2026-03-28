# SuperControlPanel MCP Tool Schemas

## Purpose

This document defines concrete MCP tool input and output shapes for the planned SuperControlPanel MCP server.

These schemas are also the contract that `supercontrolpanelhelper` should map onto.

## Response envelope

All tools should prefer a consistent result shape:

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
- `warnings`: optional string array
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
  "mcpEnabled": true,
  "activeFunctionCount": 2,
  "activeSessionCount": 2,
  "functions": []
}
```

## `function_status_list`

### Input

```json
{}
```

### Output `data`

```json
{
  "items": []
}
```

Where `items` is an array of `FunctionStatus`.

## `function_status_get`

### Input

```json
{
  "functionId": "boundary-auth"
}
```

### Output `data`

```json
{
  "item": {}
}
```

Where `item` is a `FunctionStatus`.

## `openvpn_get_status`

### Input

```json
{}
```

### Output `data`

```json
{
  "status": {}
}
```

Where `status` is a `FunctionStatus`.

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

### Output `data`

```json
{
  "status": {}
}
```

Where `status` is a `FunctionStatus`.

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
    "mcpEnabled": true,
    "mcpPort": 46739
  }
}
```

The patch follows the same partial-update semantics as the UI/backend `save_sync_settings` command.

### Output `data`

```json
{}
```

Where the returned value is the full saved device-local sync settings object.

## `remote_browser_open`

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

Where `items` is an array of `Server`.

## `server_get`

### Input

```json
{
  "serverId": "server-1"
}
```

### Output `data`

```json
{
  "item": {}
}
```

Where `item` is a `Server`.

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
