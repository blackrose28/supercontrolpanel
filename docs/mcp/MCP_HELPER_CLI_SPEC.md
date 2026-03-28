# SuperControlPanel Helper CLI Spec

## Purpose

`supercontrolpanelhelper` is a planned command-line helper that talks to the already-running SuperControlPanel app through its local MCP endpoint.

The helper exists for:

- scripting
- shell automation
- easy AI-agent invocation
- one-shot operational commands

It should not introduce a separate automation protocol.

## Design principles

- The running app’s MCP endpoint is the single automation API
- The helper is a thin CLI wrapper over MCP tools
- The helper should not implement business logic on its own
- The helper should be safe for scripting and automation

## High-level flow

1. Parse CLI arguments
2. Discover or read local MCP connection settings
3. Connect to the running app’s MCP endpoint
4. Call the mapped MCP tool
5. Print structured output
6. Exit with a meaningful status code

## Connection model

The helper should connect to the same local MCP endpoint configured by the app.

### Expected inputs

- localhost endpoint URL
- auth token
- optional timeout

### Discovery options

Possible ways to obtain connection settings:

- read from app-managed local config
- accept explicit CLI flags
- allow environment variable overrides

## Command design

Prefer a nested command tree with clear verbs.

## Proposed commands

### App and status

- `supercontrolpanelhelper status`
- `supercontrolpanelhelper function list`
- `supercontrolpanelhelper function get --function-id <id>`

### OpenVPN

- `supercontrolpanelhelper openvpn status`
- `supercontrolpanelhelper openvpn start`
- `supercontrolpanelhelper openvpn stop`

### Boundary

- `supercontrolpanelhelper boundary status`
- `supercontrolpanelhelper boundary login start`
- `supercontrolpanelhelper boundary login start --hidden`
- `supercontrolpanelhelper boundary login stop`
- `supercontrolpanelhelper boundary search-targets --filter "<text>"`

### Servers

- `supercontrolpanelhelper server list`
- `supercontrolpanelhelper server get --server-id <id>`
- `supercontrolpanelhelper server tunnel start --server-id <id>`
- `supercontrolpanelhelper server tunnel stop --server-id <id>`
- `supercontrolpanelhelper server ssh start --server-id <id>`
- `supercontrolpanelhelper server ssh stop --server-id <id>`

### Config sync

- `supercontrolpanelhelper config sync pull`
- `supercontrolpanelhelper config sync push`

## Detailed command tree

```text
supercontrolpanelhelper
├─ status
├─ function
│  ├─ list
│  └─ get --function-id <id>
├─ openvpn
│  ├─ status
│  ├─ start
│  ├─ stop
│  └─ detect-executable
├─ boundary
│  ├─ status
│  ├─ login
│  │  ├─ start [--hidden]
│  │  └─ stop
│  └─ search-targets [--filter <text>]
├─ server
│  ├─ list [--filter <text>]
│  ├─ get --server-id <id>
│  ├─ tunnel
│  │  ├─ start --server-id <id>
│  │  └─ stop --server-id <id>
│  ├─ ssh
│  │  ├─ start --server-id <id>
│  │  └─ stop --server-id <id>
│  └─ rdp
│     └─ start --server-id <id>
├─ config
│  ├─ paths
│  └─ sync
│     ├─ pull
│     └─ push
└─ mcp
   ├─ ping
   └─ info
```

## Global flags

Recommended global flags:

- `--json`
- `--timeout-ms <n>`
- `--endpoint <url>`
- `--token <token>`
- `--quiet`
- `--verbose`

## Argument conventions

- Use `--server-id` for stable server identifiers
- Use `--function-id` for stable function identifiers
- Use `--filter` for search/filter text
- Use kebab-case for command names and flags
- Use JSON output for automation-friendly integration

## Human-readable output examples

### `supercontrolpanelhelper status`

```text
App: SuperControlPanel
MCP: connected
Functions: 3 active
- openvpn-connect: running
- boundary-auth: idle
- server-tunnel-prod: running
```

### `supercontrolpanelhelper openvpn start`

```text
OpenVPN start requested successfully.
Function: openvpn-connect
Session: abc123
```

## MCP mapping

Each helper command should map directly onto one MCP tool.

### Examples

- `supercontrolpanelhelper status`
  - `app_get_overview`
- `supercontrolpanelhelper openvpn status`
  - `openvpn_get_status`
- `supercontrolpanelhelper openvpn start`
  - `openvpn_start`
- `supercontrolpanelhelper boundary login start`
  - `boundary_auth_start`
- `supercontrolpanelhelper server tunnel start --server-id <id>`
  - `server_tunnel_start`

## Output modes

Support two output styles:

### Human-readable

Default concise output intended for terminal use.

### JSON

Use `--json` for script- and agent-friendly output.

Example:

```bash
supercontrolpanelhelper boundary login start --hidden --json
```

Example response:

```json
{
  "ok": true,
  "message": "Boundary login started",
  "data": {
    "functionId": "boundary-auth",
    "sessionId": "abc123"
  },
  "warnings": [],
  "timestamp": 0
}
```

## Exit codes

Recommended exit codes:

- `0` success
- `1` helper usage or local runtime error
- `2` MCP connection or auth failure
- `3` MCP tool returned an operational failure
- `4` requested app feature is unavailable or disabled

## Hidden mode

Support `--hidden` for actions where automation should not steal focus unnecessarily.

### Intended behavior

- do not force app window focus unless needed
- start the requested action in the app
- return once the app confirms the action has been accepted or started

This is especially useful for:

- `boundary login start --hidden`

## Safety rules

- Reuse the MCP auth/token model
- Do not print secrets
- Prefer explicit, actionable error messages
- Preserve destructive-action protections defined by the app/MCP layer

## Non-goals

- Do not duplicate backend logic in the helper
- Do not create a second custom IPC protocol
- Do not make the helper depend on undocumented internal state

## Suggested implementation shape

- small dedicated binary or helper mode
- shared MCP client code
- thin command-to-tool mapping layer
- optional shared output formatting utilities

## Suggested implementation phases

### Phase H1

- implement global MCP connection flags
- implement `status`
- implement `openvpn status/start/stop`
- implement `boundary status/login start/login stop`

### Phase H2

- implement `server list/get`
- implement `server tunnel start/stop`
- implement `server ssh start/stop`

### Phase H3

- implement `config paths`
- implement `config sync pull/push`
- add polished human-readable output
- add final exit-code normalization

## Documentation impact

When implemented, document:

- installation/location of the helper
- command examples
- JSON mode
- how it discovers the local MCP endpoint
