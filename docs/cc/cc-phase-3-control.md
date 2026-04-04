# Phase 3 — Control (Low-Risk)

## Scope
- Execute non-interactive start/stop commands via CC.

## Commands
- `start_openvpn_function`, `stop_openvpn_function`
- `start_boundary_auth`, `stop_boundary_auth`
- `start_server_tunnel`, `stop_server_tunnel`
- `start_server_ssh`, `stop_server_ssh`

## Agent
- Dispatcher maps control → existing Rust commands.
- Idempotency via `{reqId, seq}` cache (5–10 min TTL).
- Timeouts + structured error surfaces.

## CC
- `POST /v1/commands` to enqueue.
- WS push to agent; await `ack`/`nack`.
- Audit each command and result.

## Acceptance Criteria
- Commands succeed/fail deterministically with reason.
- Duplicate command guarded by idempotency.
- Audit list matches executed actions.

## Risks
- State drift after reconnect → resync heartbeat includes running sessions.
