# Phase 3 — Control (Low-Risk)

## Current implementation status

Phase 3 began from a stronger base than the original draft assumed, and part of the low-risk control slice is now already implemented:

- agent ↔ server websocket connection is already established
- telemetry/history/admin visibility is already implemented
- admin auth/session/csrf/rate-limiting hardening is already in place
- the current agent protocol implementation handles:
  - `welcome`
  - `ack`
  - `nack`
  - `heartbeat`
  - `status_snapshot`
  - `inventory`
  - `recent_logs`
  - `command_status`
  - `request_status_snapshot`
  - `request_inventory`
  - `request_recent_logs`
  - `request_restart_openvpn`
  - `request_restart_boundary_auth`
  - `request_restart_server_tunnel`
  - `request_restart_server_ssh`
  - `function-status`

Implemented control slices so far:

- `request_status_snapshot`
- `request_inventory`
- `request_recent_logs`
- `restart_openvpn`
- `restart_boundary_auth`
- `restart_server_tunnel`
- `restart_server_ssh`

Windows-specific restart behavior now implemented:

- first OpenVPN start still requires the elevated helper / UAC prompt
- later CC-triggered restarts reuse the already-running elevated helper
- helper restart is performed through a helper control file, not by relaunching the elevated helper
- helper status-file writes, reconnect cancellation, and log-tail reset behavior were hardened during restart work

That means the remaining Phase 3 work should focus on the **command lifecycle / audit trail layer** around the already-implemented safe commands, not on rebuilding connection/admin primitives.

## Recommended next slice

The safe restart set is now complete. The next slice should focus on making command execution more observable and durable:

1. add server-side command persistence
2. add a first-class command lifecycle model (`queued` → terminal state)
3. add an audit trail for who triggered each command and what happened
4. add idempotency/replay handling across reconnects

The current implementation has already proven:

- server → agent command delivery
- acknowledgement through the existing websocket flow
- low-risk data refresh commands
- multiple real state-changing control actions (`restart_openvpn`, `restart_boundary_auth`, `restart_server_tunnel`, `restart_server_ssh`)

## Design goals for the next slice

- keep the current device-scoped control endpoints working
- add durable server-side visibility into what was requested, when, and by whom
- make retries/reconnects safe without causing duplicate side effects
- expose enough lifecycle state for Admin UI and API clients without requiring raw log inspection
- keep the first implementation small: command persistence and audit for the current safe command set only

## Scope
- Execute non-interactive safe control commands via CC.

## Commands

### Already implemented
- `request_status_snapshot`
- `request_inventory`
- `request_recent_logs`
- `restart_openvpn`
- `restart_boundary_auth`
- `restart_server_tunnel`
- `restart_server_ssh`

### Candidate next commands
- none in the first safe restart set

## Command lifecycle
- States: `queued` → `dispatched` → `executing` → `succeeded` | `failed` | `canceled`.
- Transitions are idempotent and append-only in audit.
- Each command carries `{commandId, reqId?, submittedAt, actor, deviceId, payload}`.

### Lifecycle semantics

| State | Meaning | Entered by | Notes |
|---|---|---|---|
| `queued` | server accepted the command and persisted it | REST/admin layer | device may be offline |
| `dispatched` | server sent the command over the agent websocket | agent gateway | may be retried if no ack arrives |
| `executing` | agent accepted the command and started local work | agent via `command_status` | include `commandId` |
| `succeeded` | command reached a terminal success outcome | agent via `command_status` or server reconciliation | include outcome summary |
| `failed` | command reached a terminal error outcome | agent via `command_status` or server reconciliation | include stable reason code |
| `canceled` | command was intentionally canceled before success | server or agent | best-effort only in the first pass |

### Transition rules

- lifecycle is append-only; terminal rows are never rewritten into another terminal state
- duplicate delivery of the same lifecycle event must be harmless
- `queued` is written before websocket dispatch so Admin/API can show accepted work even if the device is offline
- `dispatched` means "sent", not "executed"
- `command_status{ state: "acknowledged" }` is the first explicit proof that the agent accepted the command envelope
- `command_status{ state: "executing" }` is the first explicit proof that local work started
- terminal transitions must store:
  - `finishedAt`
  - `outcome`
  - `reasonCode?`
  - `reasonDetail?` (human-readable, redacted)

### Lifecycle source of truth

- primary source of truth: explicit agent-emitted `command_status` frames keyed by `commandId`
- fallback/compatibility source: existing telemetry such as `status_snapshot`, `inventory`, `recent_logs`, and `function-status`
- server inference from telemetry should remain a backstop for reconnect/reconciliation and older agents, not the preferred steady-state path

## Idempotency and exactly-once
- Server enforces idempotency by `commandId` (UUID v7 recommended). Re-submission with the same `commandId` returns the existing record.
- Agent caches the last `M` handled `commandId` values (TTL 10 minutes) to ignore duplicates across reconnects.
- Side-effecting operations are guarded by local resource locks (e.g., only one OpenVPN start/stop at a time).

### Idempotency keying

- external API clients may provide `commandId`; browser-admin-triggered commands can be server-generated
- idempotency should be scoped by `(deviceId, commandId)`
- if the same `commandId` is replayed with a different payload, reject with `409 conflict`
- replay of the same logical command should return the existing command record rather than creating a second one

## Timeouts, cancellation, and retries
- Default execution timeout: 60 s (configurable per command); agent reports `failed` with `reason: timeout`.
- Cancellation: server may send `cancel_command {commandId}`; agent attempts best-effort cancel and reports `canceled`.
- Delivery retries: if agent does not `ack` dispatch within 5 s, server may re-dispatch; idempotency prevents double-execute.

### First-pass simplification

- cancellation can stay API-internal/deferred if it is not needed for the current restart commands
- retries should initially cover only the `queued` → `dispatched` gap
- long-running interactive actions remain out of scope for this Phase 3 command-store slice

## Reconciliation on reconnect
- On WS reconnect, agent posts a `status_snapshot` that includes currently running sessions and last seen `seq`.
- Server compares desired vs actual; any `dispatched`/`executing` commands with no terminal event older than T are marked `failed` with `reason: lost_agent` unless the agent resumes them.

### Reconciliation policy

- if the device reconnects and the command already completed locally, the agent may emit the missing terminal update for the same `commandId`
- if the server cannot prove completion after the grace window, it terminalizes the command as `failed`
- reconciliation decisions should be auditable and distinguish:
  - `lost_agent`
  - `dispatch_timeout`
  - `execution_timeout`
  - `conflict`

## Validation and safety
- Strict schema validation per command; reject with explicit error codes: `invalid_params`, `unauthorized`, `conflict`, `rate_limited`.
- RBAC gates all commands; some commands may require Phase 6 approval before dispatch.

## Persistence model

The first durable command model can stay intentionally small.

### `commands`

Recommended fields:

- `id` — internal DB id
- `commandId` — external/idempotency id
- `deviceId`
- `commandType`
- `state`
- `actorType` (`admin_session`, `admin_api_token`, `system`)
- `actorId`
- `payloadJson` — redacted or safe-to-store payload only
- `payloadSummary` — short human-readable summary for lists
- `submittedAt`
- `dispatchedAt?`
- `executingAt?`
- `finishedAt?`
- `outcome?`
- `reasonCode?`
- `reasonDetail?`
- `reqSeq?`

### `command_audit`

Append-only rows:

- `id`
- `commandId`
- `ts`
- `eventType` (`created`, `dispatched`, `acknowledged`, `executing`, `succeeded`, `failed`, `canceled`, `reconciled`)
- `actorType?`
- `actorId?`
- `source` (`api`, `admin_ui`, `server`, `agent`, `reconciler`)
- `summary`
- `detailsJson` — redacted structured details

Audit entries should never store secrets, raw passwords, bearer tokens, or unredacted config bodies.

## Current CC / Admin surface
- existing device-scoped endpoints:
  - `POST /v1/devices/:id/request-status-snapshot`
  - `POST /v1/devices/:id/request-inventory`
  - `POST /v1/devices/:id/request-recent-logs`
  - `POST /v1/devices/:id/request-restart-openvpn`
  - `POST /v1/devices/:id/request-restart-boundary-auth`
  - `POST /v1/devices/:id/request-restart-server-tunnel`
  - `POST /v1/devices/:id/request-restart-server-ssh`
- admin/devices already exposes buttons for these actions
- server-targeted restart actions use the selected device inventory rather than raw server-id text entry:
  - tunnel restart picks from Boundary-backed servers on that device
  - SSH restart picks from the full server inventory on that device
- generic `/v1/commands` now supports the current safe command set in addition to the stable device-scoped endpoints

## Proposed API shape for the lifecycle slice

Keep the current device-scoped endpoints as the stable UX entrypoint, but make them create durable command records internally.

### Existing endpoints keep working

- `POST /v1/devices/:id/request-status-snapshot`
- `POST /v1/devices/:id/request-inventory`
- `POST /v1/devices/:id/request-recent-logs`
- `POST /v1/devices/:id/request-restart-openvpn`
- `POST /v1/devices/:id/request-restart-boundary-auth`
- `POST /v1/devices/:id/request-restart-server-tunnel`
- `POST /v1/devices/:id/request-restart-server-ssh`

Recommended response shape:

```json
{
  "ok": true,
  "command": {
    "commandId": "018f6b7e-....",
    "deviceId": "dev-123",
    "commandType": "restart_server_ssh",
    "state": "queued",
    "submittedAt": "2026-04-09T08:00:00Z"
  }
}
```

### New read endpoints

- `GET /v1/commands?deviceId=...&state=...&limit=...`
- `GET /v1/commands/:commandId`
- `GET /v1/commands/:commandId/audit`

`POST /v1/commands` is now available for API-driven submission of the current safe command set. The device-scoped endpoints remain the preferred admin/browser UX entrypoint.

## Agent wire contract for the lifecycle slice

The current websocket command messages stay as-is, and lifecycle reporting is now standardized around explicit `command_status` frames.

### Minimum lifecycle signals from agent

- `command_status { state: "acknowledged" }`
- `command_status { state: "executing" }` when local work starts
- one terminal `command_status` tied to the same `commandId`

Recommended payload shape:

```json
{
  "type": "command_status",
  "deviceId": "dev-123",
  "seq": 42,
  "ts": "2026-04-09T08:00:05Z",
  "commandId": "018f6b7e-....",
  "payload": {
    "state": "executing",
    "outcome": null,
    "reasonCode": null,
    "reasonDetail": null
  }
}
```

### Suggested mapping in the first pass

- `command_status{ state: "acknowledged" }` => append audit entry
- `command_status{ state: "executing" }` => mark `executing`
- `command_status{ state: "succeeded"|"failed"|"canceled" }` => mark terminal state and append audit
- plain `ack` / `nack` can still exist for websocket delivery mechanics, but they are no longer the preferred lifecycle API
- if explicit lifecycle messages are missing, the server may still derive terminal success/failure from existing safe-command telemetry as a compatibility bridge

## Admin UI behavior for command history

First pass should avoid a heavy workflow tool and instead add a simple per-device command list:

- latest commands table on the device details page
- columns:
  - submitted time
  - actor
  - command
  - payload summary
  - state
  - finished time
  - reason / outcome
- click-through to audit trail entries for the selected command
- command history should complement, not replace, telemetry/log views

## Agent
- Dispatcher maps control → existing Rust commands.
- Local resource locks ensure single in-flight action per resource type (e.g., one tunnel action per `serverId`).
- Structured error surfacing with stable `reason` codes.

## Acceptance Criteria
- implemented commands succeed/fail deterministically through existing telemetry/runtime reporting
- `request_status_snapshot`, `request_inventory`, and `request_recent_logs` surface results in admin telemetry/history
- Windows `restart_openvpn` restarts through the existing elevated helper without requiring a new UAC prompt once helper is already running
- helper restart continues surfacing updated logs and function state after restart
- each accepted command gets a durable `commandId` and visible lifecycle state
- agent emits explicit `command_status` updates for acknowledged/executing/terminal states
- duplicate `commandId` replays do not create duplicate command records or duplicate side effects
- per-command audit entries show actor, timestamps, and terminal outcome
- device detail UI can show recent command history without requiring log spelunking

## Risks
- State drift after reconnect → mitigated by `status_snapshot` and terminalization of stale commands.
- false-positive success inference if terminal state is derived only from telemetry → reduced now that `command_status` frames are emitted explicitly

## Logs pagination and filters

### Logs pagination and filters
- Endpoint: `GET /v1/logs`
- Query params:
  - `deviceId?` (string) — exact match; multiple allowed (comma-delimited) up to 50
  - `sessionId?` (string) — exact match; multiple allowed (comma-delimited)
  - `type?` (string) — in {`function-status`,`pty-output`,`pty-exit`,`openvpn-log`,`heartbeat`}; multiple allowed (comma-delimited)
  - `from?` (RFC3339) — inclusive lower bound on `ts`
  - `to?` (RFC3339) — exclusive upper bound on `ts`; must be > `from`
  - `q?` (string) — optional substring match on `payload.data` where applicable (server-side LIKE/ILIKE); redaction still applies
  - `limit?` (int) — page size; default 200; min 1; max 1000
  - `cursor?` (opaque string) — base64url-encoded position `{ ts, id }`; mutually exclusive with `from` when used for forward iteration
  - `sort?` (enum) — `asc` or `desc` (by `ts`, tie-break by `id`); default `desc`
- Pagination model: cursor-based
  - Response includes `nextCursor?` when more results exist; clients pass it back to continue
  - Cursors encode stable tuple `(ts, id)` to avoid duplicates/omissions across inserts
  - When `sort=desc`, pagination continues backwards in time; for `asc`, forwards
- Response shape:
```json
{
  "items": [
    {"id":"...","deviceId":"...","sessionId":"...","type":"pty-output","ts":"...","payload":{ /* redacted fields per policy */ }}
  ],
  "nextCursor": "base64url(...)",
  "count": 200
}
```
- Limits and safeguards:
  - Hard cap: 10,000 items per series per minute per tenant; rate-limit with `429` and `Retry-After`
  - Server enforces `to - from` window ≤ 7 days; larger windows require pagination via cursors
  - `q` filtering is best-effort text search on selected fields only; not a full-text engine
  - Redaction occurs before search/match; queries cannot reveal secrets
- Examples:
  - Latest 200 PTY lines for a device: `GET /v1/logs?deviceId=dev-1&type=pty-output&limit=200`
  - Walk back in time: `GET /v1/logs?deviceId=dev-1&type=pty-output&limit=200&cursor=...` (reuse `nextCursor`)
  - Time-bounded window asc: `GET /v1/logs?deviceId=dev-1&from=2026-04-01T00:00:00Z&to=2026-04-01T01:00:00Z&sort=asc`
- Acceptance criteria:
  - Stable pagination: no duplicates or gaps when iterating via `nextCursor`
  - Filters compose correctly (`deviceId` + `type` + time window); `q` does not bypass redaction
  - Performance: P95 query latency ≤ 500 ms for 1h window, 200 item page on typical tenant volumes
  - Backpressure: returns 429 under load with actionable `Retry-After`
## Logs API test plan

### Logs API test plan

- Pagination
  - Forward pagination (`cursor` forward): returns `items` length ≤ `limit`, `nextCursor` present when more available; stable ordering by `ts, seq`.
  - Backward pagination (`cursor` with direction=prev): returns previous page; `prevCursor` present when older data exists.
  - Boundary conditions: empty result when no matches; cursor invalid/expired → `400 invalid_cursor`.
- Filters
  - By `deviceId`, `sessionId`, `type`: exact-match returns only matching events.
  - Time window: `from`/`to` inclusive; crossing day partitions returns correct union.
  - Text search: `q` matches substring in redacted `payload` (no secrets leak).
- Performance/SLOs
  - P50/P95 latency within targets under normal load; degraded path under backpressure returns `429` with `retryAfterMs`.
- Redaction
  - All responses redact sensitive fields as per Phase 2 redaction matrix; verify representative types.
- Consistency
  - Ingested events appear in paginated queries within freshness target (Phase 2 SLOs).
- Error handling
  - Invalid params (bad ISO date, negative limit, unsupported type) → `400 invalid_params` with details.
- Examples
  - Include 3 example requests/responses covering: simple page, filtered by `deviceId`+time window, text search with pagination.
