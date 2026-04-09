# Control Plane Protocol (v1)

Envelope: `{ type, deviceId, seq, ts, payload }`

## Agent → CC (telemetry)
- `function-status`, `pty-output`, `pty-exit`, `openvpn-log`
- `heartbeat` (uptime, versions, platform)
- `status_snapshot`
- `inventory` (servers list hash)
- `recent_logs`
- `command_status` (explicit lifecycle update for a `commandId`)

## CC → Agent (control)
- current safe device-control messages:
  - `request_status_snapshot`
  - `request_inventory`
  - `request_recent_logs`
  - `request_restart_openvpn`
  - `request_restart_boundary_auth`
  - `request_restart_server_tunnel`
  - `request_restart_server_ssh`
- lower-level / broader command catalog also includes:
  - `start_openvpn_function`, `stop_openvpn_function`
  - `start_boundary_auth`, `stop_boundary_auth`
  - `start_server_tunnel`, `stop_server_tunnel`
  - `start_server_ssh`, `stop_server_ssh`
- PTY: `open_pty`, `write_pty`, `resize_pty`, `close_pty`
- SCP: `scp_upload`, `scp_download`
- Remote browser: `open_remote_browser`, `list_remote_browser_children`, `stat_remote_browser_path`, `create_remote_browser_directory`, `delete_remote_browser_path`, `close_remote_browser`

## Replies
- WebSocket replies: `ack` / `nack` with `reqSeq`, `status`, `error?`.
- HTTP telemetry ingest may also return `429` with a `nack`-shaped body such as `{ type: "nack", payload: { rate_limited: true, retryAfterMs } }`.

## Explicit command lifecycle frames

- Control commands carry `commandId`
- Agent lifecycle updates now use `command_status`
- Recommended states:
  - `acknowledged`
  - `executing`
  - `succeeded`
  - `failed`
  - `canceled`
- These explicit frames are now the preferred lifecycle source of truth; `function-status` and other telemetry remain useful supporting signals

## WebSocket
- URL: `wss /agent`, subprotocol `scp.cc.v1`.
- Auth: Bearer `<agentToken>` or mTLS.
- Backpressure: small-frame batching (10–50 ms or 8–16 KB).

## Mapping to App Contracts
- Names mirror existing Tauri commands/events 1:1; agent dispatches to `SessionStore`/`FunctionStore` and forwards `function-status`/`pty-output`/`pty-exit`/`openvpn-log`.
- For command execution tracking, the agent additionally emits `command_status` frames keyed by `commandId`.

## Versioning and Capabilities
- Protocol version: `scp.cc.v1` (WebSocket subprotocol).
- Feature negotiation: agent sends `hello{ agentVersion, supported: ["gzip", "chunking"] }`; server replies `welcome{ protocol: "scp.cc.v1", capabilities: { gzip: true, chunking: true } }`.
- Backward/forward compatibility: additive payload fields are tolerated; breaking changes bump protocol or `schemaVersion` per type.

## REST API versioning and deprecation

This section applies to the HTTP API exposed by `cc-server`, especially the mobile/admin API surface.

### Versioning model
- REST endpoints are versioned in the path: `/v1/...`, `/v2/...`.
- The first mobile/public contract is the `/v1` surface.
- Additive changes are allowed within a version:
  - adding optional response fields
  - adding optional request fields
  - adding new endpoints
- Breaking changes require a new major path version:
  - removing fields
  - renaming fields
  - changing field types
  - changing auth requirements
  - changing success/error envelope shape

### Compatibility rules
- Clients must ignore unknown response fields.
- Servers must reject invalid required inputs with stable error codes.
- Error responses should use:
  - `{ code, message, details? }`
- Browser-only auth endpoints and mobile-safe token endpoints may coexist, but each endpoint must document which auth modes it accepts.

### Deprecation policy
- Deprecations must be documented before removal.
- Minimum deprecation window for mobile/public REST endpoints: 6 months.
- Deprecated endpoints should return deprecation metadata when feasible:
  - `Deprecation: true`
  - `Sunset: <RFC 1123 date>`
  - optional docs link header to migration notes
- Migration notes must include:
  - old endpoint
  - replacement endpoint/version
  - schema/auth differences
  - removal date

### Auth-mode rules for REST
- Browser Admin UI:
  - cookie session + CSRF
- Mobile/Admin API:
  - `Authorization: Bearer <adminApiToken>`
  - no CSRF
  - current reality: bearer token may be either a server-minted DB-backed admin API token or the optional static bootstrap env token
- Agent ingest/auth:
  - `Authorization: Bearer <agentToken>`
- Agent tokens and admin API tokens must never be interchangeable.
- Token lifecycle management for browser admins:
  - `GET /admin/api-tokens`
  - `POST /admin/api-tokens`
  - `POST /admin/api-tokens/:tokenId/revoke`
  - these lifecycle endpoints are browser-session only and are not part of the mobile SDK surface

## OpenAPI and SDK plan

### Goal
- Publish an OpenAPI spec for the mobile-safe/admin REST surface first.
- Treat browser-only HTML/session flows as documented separately, not as the mobile SDK contract.

### First OpenAPI slice
- `GET /v1/devices`
- `GET /v1/devices/:id`
- `GET /v1/device-summary`
- `GET /v1/logs`
- `GET /v1/logs/export`
- `GET /v1/log-settings`
- `POST /v1/devices/:id/request-status-snapshot`
- `POST /v1/devices/:id/request-inventory`
- `POST /v1/devices/:id/request-recent-logs`
- `POST /v1/devices/:id/request-restart-openvpn`
- `POST /v1/devices/:id/request-restart-boundary-auth`
- `POST /v1/devices/:id/request-restart-server-tunnel`
- `POST /v1/devices/:id/request-restart-server-ssh`

### Next OpenAPI slice after command persistence lands
- `GET /v1/commands`
- `GET /v1/commands/:commandId`
- `GET /v1/commands/:commandId/audit`

### Explicitly out of first OpenAPI slice
- browser HTML pages under `/admin/...`
- cookie-login/browser-session flows
- websocket `/agent`
- future remote PTY control endpoints until they are real and stable

### OpenAPI requirements
- Every mobile-safe endpoint must declare:
  - auth scheme: bearer admin API token
  - required scopes where applicable (`device:read`, `device:control`, `logs:read`, `logs:export`)
  - request schema
  - success schema
  - error schema
  - stability/version notes if non-final
- Reusable components:
  - `Error`
  - `DeviceSummary`
  - `TelemetryEvent`
  - `RuntimeSummary`
  - `RequestAccepted`
- SDK generation:
  - generate typed clients only from the mobile-safe OpenAPI spec
  - do not generate SDK bindings from browser-only endpoints
  - first targets: TypeScript for web/mobile shared code; optional Kotlin/Swift later if native clients need them

## Idempotency and Delivery Semantics
- Idempotency: control commands carry `commandId` (UUID); duplicates must return the same result; the agent caches recent `commandId`s (TTL 10m).
- Telemetry ordering: guaranteed per `(deviceId, sessionId, type)` keyed by `seq`; cross-stream interleaving allowed.
- Acknowledgements: batching allowed (10–50 ms or 8–16 KB); server `ack{ reqSeq }` / `nack{ reqSeq, retryAfterMs }`.
- Compression: gzip permitted when frames > 8 KB if `welcome.capabilities.gzip=true`.
- Chunking: oversize events include `{ chunkIndex, totalChunks }` if `welcome.capabilities.chunking=true`.

## API design and schema hygiene
This section specifies the message envelope, command/event/reply payload shapes, and validation conventions. All data is JSON. Unless noted, all fields are required.

### Envelope

All frames (WS or queued) are wrapped in a common envelope.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://supercontrolpanel.dev/schemas/cc/envelope.schema.json",
  "title": "CC Envelope v1",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "type": { "type": "string", "minLength": 1 },
    "deviceId": { "type": "string", "minLength": 1 },
    "seq": { "type": "integer", "minimum": 1 },
    "ts": { "type": "string", "format": "date-time" },
    "payload": { "type": ["object", "null"] },
    "commandId": { "type": "string", "format": "uuid" },
    "reqSeq": { "type": "integer", "minimum": 1 }
  },
  "required": ["type", "deviceId", "seq", "ts"]
}
```

- **type**: message kind (see catalogs below)
- **commandId**: present on CC → Agent control commands for idempotency
- **reqSeq**: carried on replies to point back to the original `seq`

Minimal example:

```json
{
  "type": "heartbeat",
  "deviceId": "dev-123",
  "seq": 42,
  "ts": "2026-04-05T03:30:00Z",
  "payload": { "uptimeSec": 3600, "versions": {"app": "2.1.0"} }
}
```

### Catalogs (message types)

- **Agent → CC (telemetry/events)**: `function-status`, `pty-output`, `pty-exit`, `openvpn-log`, `heartbeat`
- **Agent → CC (lifecycle)**: `command_status`
- **CC → Agent (control/commands)**: `start_openvpn_function`, `stop_openvpn_function`, `start_boundary_auth`, `stop_boundary_auth`, `boundary_search_targets`, `list_servers`, `upsert_server`, `delete_server`, `start_server_tunnel`, `stop_server_tunnel`, `start_server_ssh`, `stop_server_ssh`, `start_server_rdp`, `open_remote_browser`, `close_remote_browser`, `list_remote_browser_children`, `stat_remote_browser_path`, `create_remote_browser_directory`, `delete_remote_browser_path`, `scp_upload`, `scp_download`, `start_scp_upload`, `start_scp_download`, `list_function_statuses`, `get_app_config`, `save_app_config`, `detect_openvpn_executable`, `get_config_paths`, `sync_config_pull`, `sync_config_push`, `save_sync_settings`
- **Replies**: `ack`, `nack`

Example `command_status` payload:

```json
{
  "state": "failed",
  "outcome": null,
  "reasonCode": "ACTION_FAILED",
  "reasonDetail": "restart_server_ssh failed: ..."
}
```

### Common scalar types

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/common.scalars.json",
  "type": "object",
  "definitions": {
    "Id": { "type": "string", "minLength": 1 },
    "UUID": { "type": "string", "format": "uuid" },
    "Path": { "type": "string", "minLength": 1 },
    "Cols": { "type": "integer", "minimum": 1 },
    "Rows": { "type": "integer", "minimum": 1 }
  }
}
```

### Commands (CC → Agent)

Representative schemas; new commands should follow the same pattern.

- `start_openvpn_function`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/cmd.start_openvpn_function.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "command": { "type": "string", "minLength": 1 },
    "username": { "type": ["string", "null"] },
    "password": { "type": ["string", "null"] }
  },
  "required": ["command"]
}
```

- `start_boundary_auth`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/cmd.start_boundary_auth.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "cols": { "type": "integer", "minimum": 1 },
    "rows": { "type": "integer", "minimum": 1 }
  },
  "required": ["cols", "rows"]
}
```

- `start_server_ssh`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/cmd.start_server_ssh.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "serverId": { "type": "string", "minLength": 1 },
    "cols": { "type": "integer", "minimum": 1 },
    "rows": { "type": "integer", "minimum": 1 }
  },
  "required": ["serverId", "cols", "rows"]
}
```

- `scp_upload`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/cmd.scp_upload.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "serverId": { "type": "string", "minLength": 1 },
    "localPath": { "type": "string", "minLength": 1 },
    "remotePath": { "type": "string", "minLength": 1 },
    "recursive": { "type": "boolean", "default": false }
  },
  "required": ["serverId", "localPath", "remotePath"]
}
```

### Events (Agent → CC)

- `function-status`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/evt.function_status.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "functionId": { "type": "string" },
    "state": { "type": "string", "enum": ["idle", "running", "error"] },
    "message": { "type": ["string", "null"] },
    "boundaryTokenSource": { "type": ["string", "null"], "enum": ["app", "cli", null] },
    "sessionId": { "type": ["string", "null"] },
    "updatedAtMs": { "type": "integer", "minimum": 0 }
  },
  "required": ["functionId", "state", "updatedAtMs"]
}
```

- `pty-output`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/evt.pty_output.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "id": { "type": "string" },
    "data": { "type": "string" }
  },
  "required": ["id", "data"]
}
```

- `pty-exit`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/evt.pty_exit.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "id": { "type": "string" },
    "exitCode": { "type": ["integer", "null"] },
    "reason": { "type": "string", "enum": ["exit", "signal", "unknown"] }
  },
  "required": ["id", "reason"]
}
```

- `openvpn-log`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/evt.openvpn_log.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "functionId": { "type": "string" },
    "sessionId": { "type": "string" },
    "data": { "type": "string" }
  },
  "required": ["functionId", "sessionId", "data"]
}
```

- `heartbeat`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/evt.heartbeat.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "uptimeSec": { "type": "integer", "minimum": 0 },
    "versions": { "type": "object", "additionalProperties": { "type": "string" } }
  },
  "required": ["uptimeSec"]
}
```

### Replies

- `ack`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/reply.ack.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "reqSeq": { "type": "integer", "minimum": 1 },
    "status": { "type": "string", "enum": ["ok"] },
    "result": { "type": ["object", "array", "string", "number", "boolean", "null"] }
  },
  "required": ["reqSeq", "status"]
}
```

- `nack`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/reply.nack.schema.json",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "reqSeq": { "type": "integer", "minimum": 1 },
    "status": { "type": "string", "enum": ["error"] },
    "error": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "code": { "type": "string", "minLength": 1 },
        "message": { "type": "string", "minLength": 1 },
        "details": { "type": ["object", "null"] }
      },
      "required": ["code", "message"]
    }
  },
  "required": ["reqSeq", "status", "error"]
}
```

### Conventions and hygiene

- **Extensibility**: prefer adding optional fields over breaking changes. Use `additionalProperties: false` by default; when extension points are needed, add a namespaced map (e.g., `ext: { "vendor.x": ... }`).
- **Timestamps**: RFC 3339 UTC (`format: date-time`). For high-frequency events, include `updatedAtMs` as integer epoch millis in payloads.
- **Ids**: use stable strings; when a true UUID is required, declare `format: uuid`.
- **Security**: redact credentials in all telemetry/events. Replies that include config must omit secrets or replace them with `"***"`.
- **Idempotency**: commands include `commandId` (UUID). Duplicate `commandId` for the same logical action must be safe to retry.
- **Error taxonomy**: use structured error codes (e.g., `BOUNDARY_AUTH_FAILED`, `PTY_SPAWN_ERROR`, `SFTP_NOT_CONNECTED`).
- **Compatibility**: protocol is versioned via WS subprotocol `scp.cc.v1`; breaking changes require `v2` and feature negotiation via a capabilities message.

## API design/schema

This section defines the canonical schemas for Control Plane transport. REST endpoints (`/v1/*`) are described via OpenAPI; WebSocket frames use JSON Schema with a common envelope.

### Envelope

All WS frames share the same envelope. `type` determines the `payload` schema.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://supercontrolpanel.dev/schemas/cc/envelope.json",
  "type": "object",
  "required": ["type", "deviceId", "seq", "ts", "payload"],
  "additionalProperties": false,
  "properties": {
    "type": { "type": "string", "minLength": 1 },
    "deviceId": { "type": "string", "minLength": 1 },
    "seq": { "type": "integer", "minimum": 0 },
    "ts": { "type": "string", "format": "date-time" },
    "reqSeq": { "type": "integer", "minimum": 0, "description": "Present in replies (ack/nack) to link request sequence." },
    "payload": { "type": "object" }
  }
}
```

### Event payloads (Agent → CC)

- `function-status`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/events/function-status.json",
  "type": "object",
  "required": ["functionId", "state", "updatedAtMs"],
  "additionalProperties": false,
  "properties": {
    "functionId": { "type": "string" },
    "state": { "type": "string", "enum": ["idle", "starting", "running", "stopping", "error"] },
    "message": { "type": "string" },
    "boundaryTokenSource": { "type": "string", "enum": ["app", "cli"], "nullable": true },
    "sessionId": { "type": "string" },
    "updatedAtMs": { "type": "integer", "minimum": 0 }
  }
}
```

- `pty-output`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/events/pty-output.json",
  "type": "object",
  "required": ["id", "data"],
  "additionalProperties": false,
  "properties": {
    "id": { "type": "string" },
    "data": { "type": "string" }
  }
}
```

- `pty-exit`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/events/pty-exit.json",
  "type": "object",
  "required": ["id", "exitCode", "reason"],
  "additionalProperties": false,
  "properties": {
    "id": { "type": "string" },
    "exitCode": { "type": "integer" },
    "reason": { "type": "string" }
  }
}
```

- `openvpn-log`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/events/openvpn-log.json",
  "type": "object",
  "required": ["functionId", "sessionId", "data"],
  "additionalProperties": false,
  "properties": {
    "functionId": { "type": "string" },
    "sessionId": { "type": "string" },
    "data": { "type": "string" }
  }
}
```

- `heartbeat`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/events/heartbeat.json",
  "type": "object",
  "required": ["uptimeSec", "versions"],
  "additionalProperties": false,
  "properties": {
    "uptimeSec": { "type": "integer", "minimum": 0 },
    "versions": {
      "type": "object",
      "required": ["agent", "rust", "tauri"],
      "properties": {
        "agent": { "type": "string" },
        "rust": { "type": "string" },
        "tauri": { "type": "string" }
      },
      "additionalProperties": true
    }
  }
}
```

### Control commands (CC → Agent)

- `start_openvpn_function`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/start_openvpn_function.json",
  "type": "object",
  "required": ["commandId", "command", "username", "password"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "command": { "type": "string" },
    "username": { "type": "string" },
    "password": { "type": "string" }
  }
}
```

- `stop_openvpn_function`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/stop_openvpn_function.json",
  "type": "object",
  "required": ["commandId"],
  "additionalProperties": false,
  "properties": { "commandId": { "type": "string", "format": "uuid" } }
}
```

- `start_boundary_auth` / `stop_boundary_auth`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/start_boundary_auth.json",
  "type": "object",
  "required": ["commandId", "cols", "rows"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "cols": { "type": "integer", "minimum": 1 },
    "rows": { "type": "integer", "minimum": 1 }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/stop_boundary_auth.json",
  "type": "object",
  "required": ["commandId"],
  "additionalProperties": false,
  "properties": { "commandId": { "type": "string", "format": "uuid" } }
}
```

- Server management (`serverId` is a stable UUID)

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/start_server_tunnel.json",
  "type": "object",
  "required": ["commandId", "serverId", "cols", "rows"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "serverId": { "type": "string", "format": "uuid" },
    "cols": { "type": "integer", "minimum": 1 },
    "rows": { "type": "integer", "minimum": 1 }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/stop_server_tunnel.json",
  "type": "object",
  "required": ["commandId", "serverId"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "serverId": { "type": "string", "format": "uuid" }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/start_server_ssh.json",
  "type": "object",
  "required": ["commandId", "serverId", "cols", "rows"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "serverId": { "type": "string", "format": "uuid" },
    "cols": { "type": "integer", "minimum": 1 },
    "rows": { "type": "integer", "minimum": 1 }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/stop_server_ssh.json",
  "type": "object",
  "required": ["commandId", "serverId"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "serverId": { "type": "string", "format": "uuid" }
  }
}
```

- Remote browser session (SFTP-backed while modal open)

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/open_remote_browser.json",
  "type": "object",
  "required": ["commandId", "serverId"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "serverId": { "type": "string", "format": "uuid" }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/close_remote_browser.json",
  "type": "object",
  "required": ["commandId", "browserId"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "browserId": { "type": "string" }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/list_remote_browser_children.json",
  "type": "object",
  "required": ["commandId", "browserId", "path"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "browserId": { "type": "string" },
    "path": { "type": "string" }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/stat_remote_browser_path.json",
  "type": "object",
  "required": ["commandId", "browserId", "path"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "browserId": { "type": "string" },
    "path": { "type": "string" }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/create_remote_browser_directory.json",
  "type": "object",
  "required": ["commandId", "browserId", "path"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "browserId": { "type": "string" },
    "path": { "type": "string" }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/delete_remote_browser_path.json",
  "type": "object",
  "required": ["commandId", "browserId", "path"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "browserId": { "type": "string" },
    "path": { "type": "string" },
    "recursive": { "type": "boolean", "default": false }
  }
}
```

- SCP transfers

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/scp_upload.json",
  "type": "object",
  "required": ["commandId", "serverId", "localPath", "remotePath"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "serverId": { "type": "string", "format": "uuid" },
    "localPath": { "type": "string" },
    "remotePath": { "type": "string" },
    "recursive": { "type": "boolean", "default": false }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/scp_download.json",
  "type": "object",
  "required": ["commandId", "serverId", "remotePath", "localPath"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "serverId": { "type": "string", "format": "uuid" },
    "remotePath": { "type": "string" },
    "localPath": { "type": "string" },
    "recursive": { "type": "boolean", "default": false }
  }
}
```

- Progress-aware SCP (PTY-backed)

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/start_scp_upload.json",
  "type": "object",
  "required": ["commandId", "serverId", "localPath", "remotePath"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "serverId": { "type": "string", "format": "uuid" },
    "localPath": { "type": "string" },
    "remotePath": { "type": "string" },
    "recursive": { "type": "boolean", "default": false }
  }
}
```

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/commands/start_scp_download.json",
  "type": "object",
  "required": ["commandId", "serverId", "remotePath", "localPath"],
  "additionalProperties": false,
  "properties": {
    "commandId": { "type": "string", "format": "uuid" },
    "serverId": { "type": "string", "format": "uuid" },
    "remotePath": { "type": "string" },
    "localPath": { "type": "string" },
    "recursive": { "type": "boolean", "default": false }
  }
}
```

### Replies

- `ack` / `nack`

```json
{
  "$id": "https://supercontrolpanel.dev/schemas/cc/replies/ack.json",
  "type": "object",
  "required": ["reqSeq", "status"],
  "additionalProperties": false,
  "properties": {
    "reqSeq": { "type": "integer", "minimum": 0 },
    "status": { "type": "string", "enum": ["ok", "error"] },
    "error": { "type": "string" }
  }
}
```

### Examples

- Envelope + `function-status` event

```json
{
  "type": "function-status",
  "deviceId": "agent-2a1c",
  "seq": 42,
  "ts": "2026-04-05T10:15:30.000Z",
  "payload": {
    "functionId": "server-ssh-3b7a",
    "state": "running",
    "message": "SSH session active",
    "sessionId": "pty-9f1e",
    "updatedAtMs": 1743848130000
  }
}
```
