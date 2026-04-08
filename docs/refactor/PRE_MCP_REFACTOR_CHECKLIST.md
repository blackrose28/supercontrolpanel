# SuperControlPanel Pre-MCP Refactor Checklist

## Goal

Complete the internal refactor work before adding MCP features so the MCP layer can be built on top of clean feature/service boundaries instead of a large `main.rs`.

## Refactor completion criteria

We should consider the refactor sufficiently complete for MCP work when:

- `src-tauri/src/main.rs` is mostly bootstrap, command wiring, and thin adapters
- shared models live outside `main.rs`
- runtime stores live outside `main.rs`
- PTY lifecycle helpers live outside `main.rs`
- config and keyring helpers live outside `main.rs`
- Boundary feature logic lives primarily in a `boundary_service`
- OpenVPN feature logic lives primarily in an `openvpn_service`
- server/remote-browser/SCP logic is split by responsibility enough that MCP tool handlers can call service functions directly

## Progress snapshot

### Completed

- [x] Extract shared model types from `main.rs`
- [x] Extract store/state structs from `main.rs`
- [x] Add `app_state.rs` placeholder
- [x] Start `config_service`
- [x] Start `function_service`
- [x] Extract PTY lifecycle helpers into `pty_service`
- [x] Extract OpenVPN utility helpers into `automation_service`
- [x] Extract Boundary env/token/keyring helper functions into `config_service`
- [x] Create `boundary_service.rs`
- [x] Extract `BoundaryTokenSummary`
- [x] Extract `BoundaryApiAuthenticateResponse`
- [x] Extract `BoundaryApiAuthenticateAttributes`
- [x] Extract `boundary_token_summary`
- [x] Extract `boundary_oidc_init_script`
- [x] Create `openvpn_service.rs`
- [x] Extract OpenVPN admin-helper runtime helpers
- [x] Extract `start_openvpn_admin_worker`
- [x] Extract `run_openvpn_automation`
- [x] Extract `prepare_openvpn_start`
- [x] Extract non-Windows PTY start/stop orchestration helpers

### Remaining before MCP

- [x] Reassess `main.rs` for safe cleanup-only leftovers after service extraction
- [x] Reduce `main.rs` Boundary runtime logic
- [x] Extract `run_boundary_oidc_automation` into `boundary_service`
- [x] Reassess server/browser/SCP split
- [x] Create `server_service.rs` and extract shared server/SCP helpers
- [x] Decide whether more server-side extraction is required before MCP
- [x] Update progress docs after each remaining refactor slice

## Recommended execution order

## Phase R1 — Boundary service bootstrap

- [x] Create `src-tauri/src/services/boundary_service.rs`
- [x] Move `BoundaryTokenSummary`
- [x] Move `boundary_token_summary`
- [x] Move `BoundaryApiAuthenticateResponse`
- [x] Move `BoundaryApiAuthenticateAttributes`
- [x] Move `boundary_oidc_init_script`

## Phase R2 — OpenVPN service bootstrap

- [x] Create `src-tauri/src/services/openvpn_service.rs`
- [x] Move `runtime_dir`
- [x] Move `openvpn_runtime_dir`
- [x] Move `write_openvpn_helper_script`
- [x] Move `escape_ps_single_quoted`
- [x] Move `set_openvpn_admin_session`
- [x] Move `get_openvpn_admin_session`
- [x] Move `emit_openvpn_log`

## Phase R3 — OpenVPN runtime cleanup

- [x] Reduce `start_openvpn_function` by pushing low-risk logic into service helpers
- [x] Reduce `stop_openvpn_function` by pushing low-risk logic into service helpers
- [x] Keep Tauri command shape unchanged
- [ ] Decide whether any Windows-specific start/stop logic still needs extraction

## Phase R4 — Boundary runtime cleanup

- [x] Reduce `start_boundary_auth` by pushing logic into service helpers
- [x] Reduce `stop_boundary_auth` by pushing logic into service helpers
- [x] Keep Tauri command shape unchanged

## Phase R5 — Reassess server/browser/SCP split

- [x] Review remaining `main.rs` responsibilities for servers
- [x] Decide that `server_service`, `remote_browser_service`, and `scp_service` should be extracted in phases
- [x] Create `server_service.rs` for shared server/SCP helpers
- [x] Extract server runtime start/stop command orchestration into `server_service`
- [x] Start `remote_browser_service` and route remote browser command handlers through it
- [x] Remove now-redundant remote browser helper bodies still left in `main.rs`
- [x] Extract SCP command handlers into `scp_service`

## Guardrails

- Do not change frontend/backend command names during refactor-only PRs
- Do not change event names during refactor-only PRs
- Prefer moving code first, behavior later
- Keep each PR compile-safe with `cargo check`
- Avoid mixing MCP implementation into refactor PRs

## Suggested milestone rule

Do not begin the MCP skeleton until at least:

- Boundary service bootstrap is done
- OpenVPN service bootstrap is done
- `main.rs` no longer owns most OpenVPN/Boundary feature logic directly


## Current status after SCP extraction

- `main.rs` is now primarily bootstrap, Tauri command wiring, and thin wrappers
- Boundary, OpenVPN, server, remote browser, and SCP logic all have service-module homes
- Remaining warnings are minor cleanup items and do not block the refactor milestone
- `cargo check` currently passes, but the tree still has a small set of warnings to clean up separately
