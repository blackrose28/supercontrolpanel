# SuperControlPanel MCP Rust Module Plan

## Purpose

This document proposes a Rust-side module layout for implementing:

- shared backend services
- the embedded MCP server
- the future `supercontrolpanelhelper` CLI

The goal is to keep business logic in one place and expose it through multiple adapters.

## Design principles

- business logic lives in shared services
- Tauri commands are thin adapters
- MCP tools are thin adapters
- helper CLI is a thin MCP client
- stateful runtime stores stay centralized

## Recommended top-level layout

```text
src-tauri/src/
├─ main.rs
├─ app_state.rs
├─ models/
│  ├─ mod.rs
│  ├─ common.rs
│  ├─ config.rs
│  ├─ function_status.rs
│  ├─ server.rs
│  ├─ boundary.rs
│  ├─ remote_browser.rs
│  └─ mcp.rs
├─ stores/
│  ├─ mod.rs
│  ├─ session_store.rs
│  ├─ function_store.rs
│  ├─ remote_browser_store.rs
│  └─ transfer_store.rs
├─ services/
│  ├─ mod.rs
│  ├─ config_service.rs
│  ├─ sync_service.rs
│  ├─ pty_service.rs
│  ├─ function_service.rs
│  ├─ openvpn_service.rs
│  ├─ boundary_service.rs
│  ├─ server_service.rs
│  ├─ remote_browser_service.rs
│  ├─ scp_service.rs
│  ├─ caddy_service.rs
│  └─ event_service.rs
├─ tauri_api/
│  ├─ mod.rs
│  ├─ commands.rs
│  └─ events.rs
├─ mcp/
│  ├─ mod.rs
│  ├─ server.rs
│  ├─ auth.rs
│  ├─ tools.rs
│  ├─ resources.rs
│  ├─ mapper.rs
│  └─ dto.rs
└─ helper/
   ├─ README.md
   └─ (future standalone helper crate or mode)
```

## Recommended responsibility split

## `models/`

Pure data structures shared across services and adapters.

### Suggested files

- `config.rs`
  - `AppConfig`
  - `BoundaryConfig`
  - `OpenVpnConfig`
  - MCP config structs
- `function_status.rs`
  - `FunctionStatus`
  - `StartFunctionResult`
- `server.rs`
  - `Server`
  - server service types
- `boundary.rs`
  - `BoundaryTargetSummary`
- `remote_browser.rs`
  - `RemoteBrowseSession`
  - `RemoteBrowseEntry`
  - `RemoteBrowseNodeInfo`
- `mcp.rs`
  - common MCP tool response envelope types

## `stores/`

Own long-lived in-memory runtime state.

### Suggested files

- `session_store.rs`
  - PTY sessions
  - session lookup
  - watcher registration
- `function_store.rs`
  - current function runtime states
  - status updates
- `remote_browser_store.rs`
  - active remote browser sessions
  - ownership/lifecycle tracking
- `transfer_store.rs`
  - SCP transfer history
  - transfer progress snapshots

## `services/`

This is the core business layer.

### `config_service.rs`

Responsibilities:

- load/save app config
- load/save sync-local config
- keyring integration
- config validation
- redact secret-bearing outputs for automation readers

### `sync_service.rs`

Responsibilities:

- config sync pull/push
- optional Git helper integration if kept in-app
- path validation and clear error reporting

### `pty_service.rs`

Responsibilities:

- open/write/resize/close PTYs
- PTY monitor threads
- PTY output forwarding
- PTY-child lifecycle ownership

### `function_service.rs`

Responsibilities:

- common function status updates
- status lookup/listing
- helper methods for start/stop state transitions
- shared function id conventions

### `openvpn_service.rs`

Responsibilities:

- build OpenVPN launch command
- start/stop OpenVPN function
- integrate with PTY/elevated helper behavior
- update status store

### `boundary_service.rs`

Responsibilities:

- Boundary auth start/stop
- token status detection
- Boundary target search
- app-token vs CLI-token source reporting

### `server_service.rs`

Responsibilities:

- list/upsert/delete servers
- start/stop tunnel
- start/stop SSH
- start RDP
- server lookup helpers

### `remote_browser_service.rs`

Responsibilities:

- open/close persistent remote browser session
- list/stat/create/delete remote paths
- enforce server/tunnel lifecycle rules

### `scp_service.rs`

Responsibilities:

- synchronous and PTY-backed SCP actions
- progress parsing
- transfer history updates
- path normalization and overwrite checks

### `caddy_service.rs`

Responsibilities:

- Caddy runtime controls
- Caddy status updates

### `event_service.rs`

Responsibilities:

- central event emission helpers
- Tauri event fan-out
- future MCP notification fan-out

## `tauri_api/`

Thin adapter layer for existing app commands/events.

### `commands.rs`

Contains the Tauri command functions, but they should mostly:

- parse inputs
- call services
- map errors
- return DTOs

### `events.rs`

Contains helpers for:

- `pty-output`
- `pty-exit`
- `function-status`
- `openvpn-log`

## `mcp/`

Adapter layer for automation.

### `server.rs`

Responsibilities:

- start/stop MCP server
- bind address/port
- lifecycle integration with app startup/shutdown

### `auth.rs`

Responsibilities:

- token validation
- auth middleware/helpers
- sensitive-tool policy checks

### `tools.rs`

Responsibilities:

- tool registration
- tool handlers
- call into shared services

### `resources.rs`

Responsibilities:

- resource registration
- snapshot/read handlers
- map internal state to automation-safe views

### `mapper.rs`

Responsibilities:

- shared conversion between service models and MCP response DTOs
- redaction helpers
- standardized response envelope construction

### `dto.rs`

Responsibilities:

- MCP tool input types
- MCP resource output types
- standardized success/error result shapes

## `app_state.rs`

Recommended purpose:

- central `AppState` struct for Tauri and services
- owns stores and service dependencies
- exposes shared handles used by Tauri and MCP adapters

Example contents:

- config paths
- `SessionStore`
- `FunctionStore`
- remote browser store
- transfer store
- event emitter handle
- MCP runtime state

## `main.rs`

Recommended role after refactor:

- initialize app state
- register Tauri commands
- wire event emission
- start MCP server if enabled
- keep bootstrap code only

Avoid keeping large blocks of business logic in `main.rs`.

## Helper CLI implementation options

## Option A: separate crate

Recommended long-term:

```text
helper-cli/
├─ Cargo.toml
└─ src/main.rs
```

Pros:

- clean separation
- easy distribution/testing
- no Tauri runtime dependency

## Option B: same crate, alternate mode

Example:

```bash
supercontrolpanel --helper ...
```

Pros:

- fewer crates

Cons:

- easier to accidentally couple helper code to desktop runtime concerns

Recommended choice: separate helper crate if helper scope grows.

## Suggested implementation order

1. introduce `models/` and `stores/` modules
2. extract service modules from `main.rs`
3. add `tauri_api/commands.rs` wrappers
4. add `mcp/` adapter layer
5. add MCP config/state bootstrap in `main.rs`
6. later add helper CLI crate that consumes MCP

## Migration strategy

Refactor incrementally.

### Stage 1

- move shared structs out of `main.rs`
- keep behavior unchanged

### Stage 2

- move OpenVPN, Boundary, server, SCP logic into services

### Stage 3

- make Tauri commands delegate to services

### Stage 4

- add MCP handlers on top of the same services

### Stage 5

- add helper CLI over MCP

## Testing implications

This structure should make it easier to test:

- service logic directly
- MCP tool handlers separately
- helper CLI command mapping separately

## Related docs

- `docs/mcp/MCP_IMPLEMENTATION_PLAN.md`
- `docs/mcp/MCP_TOOL_CATALOG.md`
- `docs/mcp/MCP_TOOL_SCHEMAS.md`
- `docs/mcp/MCP_HELPER_CLI_SPEC.md`
- `docs/mcp/MCP_FIRST_REFACTOR_SLICE.md`
