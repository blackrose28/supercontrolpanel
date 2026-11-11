# SuperControlPanel MCP Transport and Auth

## Implemented Slice 2 transport

One Axum listener now serves the SDK at exact `/mcp` and retains all six REST compatibility routes without redirects. It uses pinned rmcp 3.9.0/Axum 0.8.8 and SDK-managed legacy sessions. The implemented standard subset is `function_status_list` and `scp://status/functions`; both revisions (2025-11-25 and 2026-07-28) have official-client integration tests. Standard JSON/SSE handling belongs to the SDK. See [Slice 2](DESKTOP_STREAMABLE_HTTP_SLICE2.md) for scope and reproduction.

All routes require bearer auth, reject any Origin, and require Host `127.0.0.1:<port>`, `localhost:<port>`, or `[::1]:<port>`. Native requests omit Origin. Bodies are limited to 1 MiB. This is a local static-bearer deployment; OAuth discovery and general remote clients are not claimed. Listener status retains `transport: "http"`; active `implementationStage` is now `http-streamable-slice2`. Config copy/setup UX and lifecycle qualification remain later slices.

## Goal

Provide an MCP connection model that works well when the SuperControlPanel desktop app is already running and an AI agent wants to call into that live app instance.

This same MCP endpoint should also be the automation surface used by future command-line helper tooling.

## Recommended transport

Use an embedded MCP server reachable from the local machine.

### Why this fits SuperControlPanel

- The app is already a long-running desktop process
- The backend already holds live runtime state in memory
- AI agents should be able to query and control that live state
- It avoids spawning a separate process just to proxy the desktop app

## Recommended defaults

- Bind address: `127.0.0.1`
- Configurable port
- Disabled by default
- One auth token per app instance or per configured profile

## Lifecycle

- Turning on the Config page toggle starts the MCP server
- Turning it off stops the MCP server cleanly
- App shutdown should also stop the MCP server cleanly

## Helper CLI strategy

Do not add a second custom IPC protocol for one-shot command-line automation.

Instead, a future `supercontrolpanelhelper` command-line tool should act as a thin MCP client.

### Example direction

- `supercontrolpanelhelper boundary login start --hidden`
- `supercontrolpanelhelper openvpn status --json`
- `supercontrolpanelhelper server tunnel start --server-id abc`

### Why this is preferred

- one automation protocol
- one auth model
- one tool surface
- less backend duplication
- easier testing and maintenance

## Auth approach

Use a bearer-style token or equivalent shared secret for initial implementation.

### Requirements

- Token generated locally
- Token stored in device-local config, not synced by default
- Token shown in Config page with regenerate action
- Token required for all MCP calls unless a future read-only anonymous mode is explicitly designed

## Client setup UX

The Config page should show a copyable snippet containing:

- transport type
- localhost URL
- auth token placement

## Future transport options

If a future need appears, additional local transports can be evaluated later, but they should not replace the main design:

- one running app instance
- one local MCP endpoint
- optional helper CLI layered on top of MCP


## Concrete v1 auth decision

Use a single device-local bearer token stored in `sync-local.json`.

### v1 settings contract

- `mcpEnabled`
- `mcpBindAddress`
- `mcpPort`
- `mcpAuthToken`
- `mcpAllowedToolCategories`
- `mcpSensitiveActionPolicy`

### Defaults

- bind address: `127.0.0.1`
- port: `46739`
- enabled: `false`
- allowed categories: `read-only`, `runtime-control`
- sensitive action policy: `require-ui-approval`

### Token details

- token is generated locally
- token is device-local and not part of synced config
- legacy `sync-local.json` files missing the token will get one generated on load
- UI should later expose regenerate/copy actions without changing the storage contract


## Current implementation status

The current repo now includes a minimal in-process localhost HTTP listener.

### Current local HTTP endpoints

- `GET /health`
- `GET /mcp/status`
- `GET /mcp/tools`
- `GET /mcp/resources`
- `POST /mcp/tools/call`
- `POST /mcp/resources/read`

### Current auth behavior

- every HTTP request requires `Authorization: Bearer <mcpAuthToken>`
- token comes from device-local `sync-local.json` settings
- changing MCP settings through the Config page re-syncs the live runtime
- non-localhost bind addresses are currently rejected by the backend
- when sensitive-action policy is `require-ui-approval`, sensitive tool calls are rejected over HTTP until the policy is changed to `allow-with-token`

### Current tool coverage

- read-only tools: app overview, function status, OpenVPN status, Boundary status, server list/detail, config paths
- runtime-control tools: OpenVPN start/stop, Boundary auth start/stop, server tunnel start/stop, server SSH start/stop, Caddy start/stop
- mutation tools: server upsert/delete, full/partial app-config save, device-local sync-settings get/patch
- remote-browser and SCP tools: open/close/list/stat/mkdir/delete remote browser sessions plus direct and PTY-backed SCP upload/download starts and SCP transfer history lookup
- config-sync: shared config push remains available under sensitive policy; sync pull and git pull/push are unavailable remotely pending protected import/git review

### Important note

This is a **minimal localhost HTTP transport** for the current implementation phase. It is intentionally simpler than a full spec-complete MCP transport and should be treated as the repo's current working transport layer, not the final polish state.

### Slice 1 policy corrections

Discovery, tool calls, resources, and Tauri MCP adapters share the registry's fail-closed policy. Every launch/mutation is sensitive. MCP cannot edit listener/policy/CC identity or local launch settings. Only `repoRoot` is accepted in a device-local MCP settings patch. See [the complete Slice 1 contract](DESKTOP_STREAMABLE_HTTP_SLICE1.md) for protected fields, omitted diagnostics/secrets, stricter argument validation, and test evidence. Local desktop administration retains its authority.

### Current error behavior

- malformed requests and MCP call failures return a standardized JSON envelope
- envelope shape:
  - `ok: false`
  - `message`
  - `data: {}`
  - `warnings: []`
  - `timestamp`
