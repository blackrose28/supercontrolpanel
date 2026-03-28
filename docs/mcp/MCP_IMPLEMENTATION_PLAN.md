# SuperControlPanel as MCP Server — Implementation Plan

## Goal

Expose the running SuperControlPanel desktop app as an MCP server so AI agents can inspect runtime state and operate app features through MCP tools while the app remains the main user-facing control surface.

## Primary use cases

- Check OpenVPN status
- Start or stop OpenVPN
- Check Boundary auth status
- Start or stop Boundary login
- Search Boundary targets
- List and manage servers
- Start or stop Boundary tunnels and SSH sessions
- Browse remote files
- Run SCP upload and download flows
- Read and update app configuration
- Run config sync and Git-backed config workflows

## Non-goals for v1

- Exposing the MCP server to non-local networks
- Returning passwords, OTP secrets, or other secrets from keyring storage
- Replacing the existing UI
- Making raw PTY streaming the primary MCP interaction model
- Adding a separate non-MCP automation IPC protocol

## Architecture

SuperControlPanel should embed an MCP server inside the existing Rust backend.

### Design principles

- The UI remains the primary operator experience
- MCP becomes an additional control surface
- Tauri commands and MCP tools must share the same backend service layer
- Existing command and event contracts should remain intact unless both frontend and backend are updated together

### Recommended layers

1. **Core services**
   - function status service
   - PTY/session service
   - OpenVPN service
   - Boundary service
   - server management service
   - remote browser and SCP service
   - config and sync service
   - Caddy service
2. **Adapters**
   - existing Tauri commands/events
   - new MCP tools/resources/prompts
3. **Transport**
   - localhost MCP transport
   - auth and policy enforcement

## Transport recommendation

Default to a localhost MCP transport that can connect to the already-running app instance.

### Recommended defaults

- Bind address: `127.0.0.1`
- Configurable port
- Disabled by default
- Protected by an auth token
- One embedded server per running app instance

## Automation surface strategy

MCP should be the single automation protocol for SuperControlPanel.

### Principle

- AI agents call the app through the MCP endpoint
- Future command-line automation should also call the same MCP endpoint
- Avoid inventing a second custom IPC protocol for helper tooling

### Planned v2 helper

Add a helper CLI such as `supercontrolpanelhelper` that acts as a thin MCP client to the already-running app.

Examples:

- `supercontrolpanelhelper boundary login start --hidden`
- `supercontrolpanelhelper boundary login stop`
- `supercontrolpanelhelper openvpn status --json`
- `supercontrolpanelhelper server tunnel start --server-id my-server`

The helper should:

- parse CLI arguments
- call the local MCP endpoint
- print structured output
- exit once the action is accepted or completed enough for scripting purposes

## Config page changes

Add a new **MCP Server** section on the Config page.

### Settings

- Enable MCP server
- Bind address
- Port
- Auth token
- Allowed tool categories
- Confirmation policy for sensitive actions
- Expose secrets to MCP: default off

### UI helpers

- Running/stopped/error status indicator
- Active client/session count if available
- Copyable MCP client connection snippet
- Clear explanation that MCP access controls the live desktop app

## Backend implementation strategy

### Step 1: service extraction

Refactor the current backend so Tauri commands become thin wrappers around shared services.

### Step 2: MCP adapter

Implement MCP handlers that call the same services used by Tauri commands.

### Step 3: state exposure

Expose structured status and configuration summaries via MCP tools and resources.

### Step 4: action coverage

Expose start/stop/mutate flows for each major app feature as MCP tools.

## MCP feature model

### Tools

Use tools for:

- status queries
- runtime controls
- config mutations
- server CRUD
- remote file operations
- SCP and sync actions

### Resources

Use resources for:

- function status snapshot
- config summary
- server list
- transfer history
- recent event summaries

### Notifications

Later phases can expose notifications for:

- function status changes
- transfer progress changes
- PTY exit events

### Prompts

Optional prompts can help agents use the tools effectively, for example:

- diagnose VPN status
- connect to a server by name
- search Boundary and create a server record

## Tool design rules

- Prefer high-level task-oriented tools in v1
- Avoid exposing raw PTY control in v1 unless truly needed
- Return stable JSON shapes
- Include timestamps and IDs where useful
- Redact secrets by default
- Make start actions idempotent where practical

## Response shape recommendation

Use a standard response envelope where practical:

```json
{
  "ok": true,
  "message": "Boundary auth started",
  "data": {},
  "warnings": [],
  "timestamp": 0
}
```

## Rollout strategy

1. Extract shared backend services
2. Add MCP config model and Config page UI
3. Implement MCP server skeleton
4. Ship read-only tools first
5. Add runtime control tools
6. Add remote browser, SCP, and sync tools
7. Add notifications, approvals, and hardening
8. Add `supercontrolpanelhelper` as a CLI wrapper over MCP tools

## Documentation rule

Detailed MCP planning and design docs should live under `docs/mcp/`, not in the repo root.

Related helper CLI design:

- `docs/mcp/MCP_HELPER_CLI_SPEC.md`
- `docs/mcp/MCP_TOOL_SCHEMAS.md`
- `docs/mcp/MCP_RUST_MODULE_PLAN.md`
