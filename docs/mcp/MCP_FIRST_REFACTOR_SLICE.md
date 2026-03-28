# SuperControlPanel MCP First Refactor Slice

## Purpose

This document proposes the safest first refactor steps for preparing `src-tauri/src/main.rs` for MCP support without trying to rewrite everything at once.

The goal is to:

- reduce risk
- keep the app working after each step
- extract shared structures before extracting behavior
- make later MCP work easier

## Guiding rule

Refactor in this order:

1. shared types
2. stores/state containers
3. pure helper functions
4. service boundaries
5. Tauri command wrappers
6. MCP adapters

Do **not** start by moving all runtime logic at once.

## Current risk profile

`src-tauri/src/main.rs` currently mixes:

- data models
- config models
- PTY runtime ownership
- function runtime state
- Boundary/OpenVPN/server logic
- Tauri command handlers
- event emission

That makes a full rewrite risky.

## Best first slice

The first refactor slice should extract code that is:

- widely reused
- relatively stable
- low in behavioral complexity
- easy to compile-check

## Slice 1 — Extract models only

### Goal

Move data definitions out of `main.rs` without changing behavior.

### Status

Completed.

### Recommended first extractions

Move these to `src-tauri/src/models/` first:

- `BoundaryTargetSummary`
- `PtyOutputPayload`
- `PtyExitPayload`
- `FunctionStatus`
- `StartFunctionResult`
- `OpenVpnLogPayload`
- `RemotePathKind`
- `RemoteBrowseSession`
- `RemoteBrowseEntry`
- `RemoteBrowseNodeInfo`
- `SshEndpoint`
- `AppConfig`
- `BoundaryAuthKind`
- `BoundaryOidcLoginMode`
- `BoundaryConfig`
- `BoundaryConfigForDisk`
- `AppRuntimeConfig`
- `OpenVpnConfig`
- `OpenVpnLaunchConfig`
- `HelperStatusFile`
- `OpenVpnConfigForDisk`
- `AppConfigForDisk`

### Why this first

- mostly type movement
- low logic risk
- immediately reduces `main.rs` size
- creates reusable shared DTOs for MCP later

### Important rule

Do not change field names or serde behavior in this step.

## Slice 2 — Extract stores/state containers

### Goal

Move runtime state holder structs into `src-tauri/src/stores/`.

### Recommended extractions

- `PtySession`
- `SessionStore`
- `FunctionStore`
- remote browser session store types
- SCP transfer tracking store types

### Why second

- these are foundational dependencies for later services
- extraction is still mostly structural
- behavior can remain unchanged

### Important rule

Keep method signatures the same where possible.

## Slice 3 — Extract config path and config disk helpers

### Goal

Move config-specific helper functions into a `config_service` or `config_helpers` module.

### Good candidates

- app config path resolution
- sync-local path resolution
- config load/save helpers
- keyring read/write helpers
- disk serialization helpers
- redaction helpers for future MCP use

### Why this is a good early behavior extraction

- relatively bounded area
- easy to test
- needed by UI, MCP, and helper flows

## Slice 4 — Extract function status helpers

### Goal

Create one place that owns status update behavior.

### Good candidates

- create/update/list function status helpers
- timestamp updates
- state/message normalization
- shared function id helpers

### Why

- many features depend on this
- MCP status tools will need it
- status changes should not stay scattered through `main.rs`

## Slice 5 — Extract PTY service

### Goal

Move PTY lifecycle code behind a service boundary.

### Good candidates

- open/write/resize/close logic
- child monitor startup
- PTY output forwarding
- watcher notification helpers

### Why not first

- more behavior-heavy
- threading and ownership make it riskier
- easier after models and stores are already extracted

## Slice 6 — Extract feature services one by one

After the groundwork above, extract feature logic in this order:

### 6A. OpenVPN service

Why first:

- already a distinct feature boundary
- core use case for MCP

### 6B. Boundary service

Why second:

- another major MCP use case
- clearer separation after status/config helpers exist

### 6C. Server service

Why third:

- depends on several shared types
- likely touches more features

### 6D. Remote browser and SCP services

Why later:

- more stateful
- more moving parts
- depends on server and PTY groundwork

### 6E. Caddy service

Can be extracted whenever convenient if it is already isolated enough.

## Concrete low-risk file creation order

Recommended file creation order:

1. `src-tauri/src/models/mod.rs`
2. `src-tauri/src/models/function_status.rs`
3. `src-tauri/src/models/config.rs`
4. `src-tauri/src/models/boundary.rs`
5. `src-tauri/src/models/remote_browser.rs`
6. `src-tauri/src/stores/mod.rs`
7. `src-tauri/src/stores/session_store.rs`
8. `src-tauri/src/stores/function_store.rs`
9. `src-tauri/src/services/mod.rs`
10. `src-tauri/src/services/config_service.rs`
11. `src-tauri/src/services/function_service.rs`
12. `src-tauri/src/services/pty_service.rs`

This order gives useful structure without forcing immediate large behavior moves.

## Minimal-risk implementation sequence

## Step A

Create `models/` modules and move type definitions only.

Success condition:

- [x] app compiles
- [x] no behavior changes

## Step B

Create `stores/` modules and move store structs only.

Success condition:

- app compiles
- PTY/session/function behavior unchanged

## Step C

Introduce `app_state.rs` to centralize owned stores.

Success condition:

- main bootstrap still works
- no feature behavior changes

## Step D

Move config load/save and keyring helpers into `config_service.rs`.

Success condition:

- Config page still works
- config sync behavior unchanged

## Step E

Move function status helper logic into `function_service.rs`.

Success condition:

- Status page still updates correctly
- OpenVPN/Boundary/server status cards remain accurate

## Step F

Move PTY lifecycle code into `pty_service.rs`.

Success condition:

- PTY open/write/resize/close still work
- `pty-output` and `pty-exit` behavior unchanged

## After first slice is complete

Once the steps above are done, the codebase should be ready for:

- adding `tauri_api/commands.rs`
- adding `mcp/` modules
- exposing read-only MCP tools first

## What not to do in the first slice

- do not rename public Tauri commands
- do not change event names
- do not redesign feature behavior
- do not mix MCP implementation with the first extraction pass
- do not migrate remote browser/SCP logic first

## Suggested checkpoints

After each step:

- build the Tauri app
- smoke test OpenVPN page
- smoke test Boundary page
- smoke test Servers page
- verify status cards still update

## Recommended first actual PR split

### PR 1

- [x] extract shared models

### PR 2

- extract stores and app state

### PR 3

- extract config and function status helpers

### PR 4

- extract PTY service

### PR 5

- begin MCP server skeleton on top of the new structure

## Related docs

- `docs/mcp/MCP_RUST_MODULE_PLAN.md`
- `docs/mcp/MCP_IMPLEMENTATION_PLAN.md`
- `docs/mcp/MCP_TOOL_SCHEMAS.md`
- `docs/mcp/MCP_PR1_RUST_SCAFFOLDING.md`
