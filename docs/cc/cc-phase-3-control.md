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
  - `request_status_snapshot`
  - `request_inventory`
  - `request_recent_logs`
  - `request_restart_openvpn`
  - `function-status`

Implemented control slices so far:

- `request_status_snapshot`
- `request_inventory`
- `request_recent_logs`
- `restart_openvpn`

Windows-specific restart behavior now implemented:

- first OpenVPN start still requires the elevated helper / UAC prompt
- later CC-triggered restarts reuse the already-running elevated helper
- helper restart is performed through a helper control file, not by relaunching the elevated helper
- helper status-file writes, reconnect cancellation, and log-tail reset behavior were hardened during restart work

That means the remaining Phase 3 work should focus on **additional safe commands** and optionally a fuller command/audit model, not on rebuilding connection/admin primitives.

## Recommended next slice

The next smallest safe vertical slices are:

1. `restart_boundary_auth`
2. `restart_server_tunnel`
3. `restart_server_ssh`
4. optionally add server-side command persistence and audit trail

The current implementation has already proven:

- server → agent command delivery
- acknowledgement through the existing websocket flow
- low-risk data refresh commands
- one real state-changing control action (`restart_openvpn`)

## Scope
- Execute non-interactive safe control commands via CC.

## Commands

### Already implemented
- `request_status_snapshot`
- `request_inventory`
- `request_recent_logs`
- `restart_openvpn`

### Candidate next commands
- `restart_boundary_auth`
- `restart_server_tunnel`
- `restart_server_ssh`

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

## Current CC / Admin surface
- existing device-scoped endpoints:
  - `POST /v1/devices/:id/request-status-snapshot`
  - `POST /v1/devices/:id/request-inventory`
  - `POST /v1/devices/:id/request-recent-logs`
  - `POST /v1/devices/:id/request-restart-openvpn`
- admin/devices already exposes buttons for these actions
- full generic `/v1/commands` persistence / audit is still deferred

## Agent
- Dispatcher maps control → existing Rust commands.
- Local resource locks ensure single in-flight action per resource type (e.g., one tunnel action per `serverId`).
- Structured error surfacing with stable `reason` codes.

## Acceptance Criteria
- implemented commands succeed/fail deterministically through existing telemetry/runtime reporting
- `request_status_snapshot`, `request_inventory`, and `request_recent_logs` surface results in admin telemetry/history
- Windows `restart_openvpn` restarts through the existing elevated helper without requiring a new UAC prompt once helper is already running
- helper restart continues surfacing updated logs and function state after restart

## Risks
- State drift after reconnect → mitigated by `status_snapshot` and terminalization of stale commands.

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
