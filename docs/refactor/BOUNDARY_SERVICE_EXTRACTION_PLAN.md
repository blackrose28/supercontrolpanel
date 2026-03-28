# Boundary Service Extraction Plan

## Goal

Extract the remaining Boundary-specific logic from `src-tauri/src/main.rs` into a focused `src-tauri/src/services/boundary_service.rs` module before MCP work begins.

## Status

Core Boundary service bootstrap is now complete.

Completed:

- [x] Created `src-tauri/src/services/boundary_service.rs`
- [x] Moved `BoundaryTokenSummary`
- [x] Moved `BoundaryApiAuthenticateResponse`
- [x] Moved `BoundaryApiAuthenticateAttributes`
- [x] Moved `boundary_token_summary`
- [x] Moved `boundary_oidc_init_script`

Remaining follow-up:

- [x] Reassess remaining Boundary runtime orchestration in `main.rs`
- [x] Extract additional `start_boundary_auth` / `stop_boundary_auth` helper seams where worthwhile
- [x] Keep behavior unchanged while shrinking `main.rs`

## Why this matters

Boundary is one of the first MCP feature areas we want to expose:

- auth status
- auth start/stop
- target search

If Boundary logic remains scattered through `main.rs`, the future MCP layer will be harder to implement and harder to test.

## Primary targets

## Target A — `BoundaryTokenSummary`

Move:

- `BoundaryTokenSummary`

Status:

- Done.

Reason:

- it belongs to Boundary domain logic
- it is only meaningful in the context of Boundary auth state

## Target B — `boundary_token_summary`

Move:

- `boundary_token_summary(app: &AppHandle) -> Result<BoundaryTokenSummary, String>`

Status:

- Done.

Reason:

- it is effectively already a service method
- it combines config loading, CLI resolution, token selection, command execution, and auth interpretation

### Risks

- changing token-state semantics on the Status page
- changing CLI env handling accidentally

### Safe extraction rule

- preserve exact behavior first
- keep the same return semantics
- only relocate and rewire calls

## Target C — `BoundaryApiAuthenticateResponse`

Move:

- `BoundaryApiAuthenticateResponse`
- `BoundaryApiAuthenticateAttributes`

Status:

- Done.

Reason:

- these are Boundary HTTP/OIDC response models
- they belong next to Boundary auth logic

## Target D — `boundary_oidc_init_script`

Move:

- `boundary_oidc_init_script(cfg: &BoundaryConfig) -> String`

Status:

- Done.

Reason:

- it is Boundary-specific
- it is tightly related to automated OIDC behavior

### Why this one is riskier

- large embedded JS string
- easy to break interpolation or escaping
- brittle selector heuristics
- not well validated by compilation alone

### Safe extraction rule

- first move it as-is
- do not rewrite or simplify the script in the same PR
- only consider later splitting it into its own template/helper file

## Suggested file target

Created:

```text
src-tauri/src/services/boundary_service.rs
```

Current contents include:

- `BoundaryTokenSummary`
- `boundary_token_summary`
- `BoundaryApiAuthenticateResponse`
- `BoundaryApiAuthenticateAttributes`
- `boundary_oidc_init_script`

## Extraction order

### Step 1

Create `boundary_service.rs` and move the response structs plus `BoundaryTokenSummary`.

Status:

- Done.

### Step 2

Move `boundary_token_summary` with no behavior changes.

Status:

- Done.

### Step 3

Move `boundary_oidc_init_script` unchanged.

Status:

- Done.

### Step 4

Update `main.rs` imports and call sites.

Status:

- Done.

### Step 5

Run `cargo check`.

Status:

- Done.

## Non-goals for this extraction

- do not redesign the OIDC script
- do not change Boundary auth flow
- do not start MCP work in the same PR
- do not merge unrelated OpenVPN/server cleanup into this extraction

## Success criteria

- `cargo check` passes
- Boundary status behavior remains unchanged
- automated OIDC setup behavior remains unchanged
- `main.rs` becomes smaller and more focused

## Follow-up after this plan

Completed follow-up after the bootstrap:

1. `start_boundary_auth` planning extracted
2. `stop_boundary_auth` orchestration extracted
3. Boundary automation lifecycle helpers extracted

Completed optional follow-up:

- [x] moved `run_boundary_oidc_automation` into `boundary_service`

Next focus:

- server / remote browser / SCP phased extraction work
