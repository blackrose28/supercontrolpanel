# Phase 3 — Control (Low-Risk)

## Scope
- Execute non-interactive start/stop commands via CC.

## Commands
- `start_openvpn_function`, `stop_openvpn_function`
- `start_boundary_auth`, `stop_boundary_auth`
- `start_server_tunnel`, `stop_server_tunnel`
- `start_server_ssh`, `stop_server_ssh`

## Command lifecycle
- States: `queued` → `dispatched` → `executing` → `succeeded` | `failed` | `canceled`.
- Transitions are idempotent and append-only in audit.
- Each command carries `{commandId, reqId?, submittedAt, actor, deviceId, payload}`.

## Idempotency and exactly-once
- Server enforces idempotency by `commandId` (UUID v7 recommended). Re-submission with the same `commandId` returns the existing record.
- Agent caches the last `M` handled `commandId` values (TTL 10 minutes) to ignore duplicates across reconnects.
- Side-effecting operations are guarded by local resource locks (e.g., only one OpenVPN start/stop at a time).

## Timeouts, cancellation, and retries
- Default execution timeout: 60 s (configurable per command); agent reports `failed` with `reason: timeout`.
- Cancellation: server may send `cancel_command {commandId}`; agent attempts best-effort cancel and reports `canceled`.
- Delivery retries: if agent does not `ack` dispatch within 5 s, server may re-dispatch; idempotency prevents double-execute.

## Reconciliation on reconnect
- On WS reconnect, agent posts a `status_snapshot` that includes currently running sessions and last seen `seq`.
- Server compares desired vs actual; any `dispatched`/`executing` commands with no terminal event older than T are marked `failed` with `reason: lost_agent` unless the agent resumes them.

## Validation and safety
- Strict schema validation per command; reject with explicit error codes: `invalid_params`, `unauthorized`, `conflict`, `rate_limited`.
- RBAC gates all commands; some commands may require Phase 6 approval before dispatch.

## CC
- `POST /v1/commands` enqueues; returns the created record.
- WS push to agent; await `ack`/`nack` for dispatch, then a terminal event.
- Full audit trail with before/after snapshots of relevant runtime state (secrets redacted).

## Agent
- Dispatcher maps control → existing Rust commands.
- Local resource locks ensure single in-flight action per resource type (e.g., one tunnel action per `serverId`).
- Structured error surfacing with stable `reason` codes.

## Acceptance Criteria
- Commands succeed/fail deterministically with stable `reason` codes.
- Duplicate commands do not cause duplicate effects (verified by re-submission and reconnect replay tests).
- Reconnect reconciliation closes out stale in-flight commands correctly.
- Audit list matches executed actions with lifecycle state transitions.

## Risks
- State drift after reconnect → mitigated by `status_snapshot` and terminalization of stale commands.
