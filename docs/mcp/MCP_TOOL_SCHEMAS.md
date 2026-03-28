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
{
  "synced": true,
  "direction": "pull"
}
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
  "synced": true,
  "direction": "push"
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
