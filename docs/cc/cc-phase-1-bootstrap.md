# Phase 1 — Bootstrap

## Goals
- Establish secure outbound WS connection from agent to CC.
- Stream status and runtime telemetry.
- Provide minimal Admin API and device/session tracking to view devices.

## Current implementation note

This phase is only partially "Phase 1" in the original sense now because the paired `cc-server` repo moved ahead in some areas:

- Implemented now:
  - agent outbound WS connection with reconnect/backoff
  - `POST /v1/enroll`
  - `GET /v1/devices`, `GET /v1/devices/:id`, `GET /v1/health`
  - duplicate-connection eviction for the same `deviceId`
  - server-rendered admin devices page
  - persisted devices in SQLite, with live WS sessions still in memory
  - admin login/session + CSRF protection on the server
- real connected/disconnected state exposed through `get_cc_status` and polled by the Config UI
- Still incomplete in the app:
  - agent-side enrollment UX is not implemented in this repo; setup is still manual via `sync-local.json` or the Config form fields

## Deliverables
- CC Server (default: NestJS)
  - WebSocket `/agent` (subprotocol `scp.cc.v1`, localhost-only dev default)
  - HTTP Admin (Phase 1 minimal): `GET /v1/devices`, `GET /v1/devices/:id`, `GET /v1/health`
  - Device records persisted in SQLite; active WS sessions remain in memory
  - Simple Admin page (server-rendered) listing devices with columns: `id`, `name`, `lastSeenAt`, `status`
- Agent (this app)
  - New module `src-tauri/src/cc/` with `connection.rs`, `protocol.rs`, `dispatcher.rs`
  - Config in `sync-local.json`: `ccEnabled` (bool), `ccServerUrl` (string), `ccAuthMode` (`mtls`|`token`), `ccAgentToken?` (string), `ccDeviceId?` (string)
  - Heartbeat every 15s: `heartbeat { uptimeSec, versions }`
  - Initial `status_snapshot` on connect: `{ activeSessions, lastSeqByStream }`
  - Forward `function-status` events to CC (telemetry)
  - Reconnect with jittered exponential backoff (base 500ms, max 30s)
- Frontend
  - Config page: Control plane section (URL, token, enable toggle)
  - Status page shows CC as a normal runtime status card with live connected/disconnected state
## Enrollment and Device Identity

### Enrollment UX
- Bootstrap token flow (Phase 1): Admin generates a one-time bootstrap token scoped to org (expiry ≤ 24h). The agent accepts either paste-in token or QR scan of `ccServerUrl` + token.
- Token exchange: agent calls `POST /v1/enroll` with the bootstrap token and a proposed `deviceName`; server returns `{ ccAgentToken, deviceId }`.
- One-time code fallback: when QR/paste not possible, allow short code entry (server maps code → bootstrap token).

### Device identity and naming
- `deviceId`: UUID v7 generated server-side on first enrollment; persisted on the agent in `sync-local.json` as `ccDeviceId`.
- `deviceName`: default to OS hostname; sanitize to `[a-zA-Z0-9._-]{1,64}`; if taken, server appends `-N` to ensure uniqueness (client stores canonical name returned by server).
- `deviceAlias` (optional, admin-provided): stable human alias shown in Admin first; stored server-side and may be edited without changing `deviceName`.
  - Conflicts: alias must be unique within org; rejects with `ALIAS_TAKEN`.
  - Display order: Admin shows `alias (deviceName)` when alias exists; otherwise just `deviceName`.
- Re-enrollment: clearing `ccAgentToken` or `ccDeviceId` triggers re-enrollment; server treats a new enrollment as a distinct device record.

### Duplicate devices and concurrent connections
- Policy: single active WS per `deviceId`.
  - Same device reconnect: new connection takes over; server evicts the old session with reason `replaced_by_new_connection`.
  - Different device claiming same `deviceId`: reject with `nack { code: "device_id_conflict" }` and require re-enrollment.

### Sequence/offset resume semantics
- Agent maintains per-stream `seq` and a bounded offline buffer (see Phase 2 for sizes/policies).
- On connect, agent sends `status_snapshot { lastSeqByStream }`.
- Server responds with `resume_from { lastAckedByStream }`; agent replays buffered events with `seq > lastAcked` and ignores server-side duplicates keyed by `(deviceId, seq)`.

### Deliverables (Phase 1 scope)
- Implement `POST /v1/enroll` returning `{ deviceId, ccAgentToken }`.
- Add agent enrollment UI (Config page): `Enroll` button, paste/QR, and display of assigned `deviceName`. This remains deferred; current setup is manual.
- Persist `ccDeviceId`, `ccAgentToken` in `sync-local.json` after successful enrollment.
- Enforce single active connection per `deviceId` on the server.

## Acceptance Criteria (Phase 1)

- Secure WS connect succeeds with valid token; rejected with clear error when invalid/expired.
- Device appears in Admin list within 5s with `lastSeenAt` updating; status reflects WS up/down.
- Heartbeat every 15s received for connected devices; jitter/backoff respects limits on reconnect storms.
- Duplicate connection policy enforced: new connection evicts old for same `deviceId`; audit reflects eviction reason.
- `status_snapshot` sent on connect with `{ activeSessions, lastSeqByStream }`; server stores and renders current state.
- Telemetry forwarding (at least `function-status`) flows end-to-end and is visible in Admin logs/feed.

## Error Taxonomy (Phase 1)

| Code | Meaning | Agent action |
|------|---------|--------------|
| `BOOTSTRAP_TOKEN_INVALID` | Token not found, expired, or already used | Do not retry; surface error to user |
| `BOOTSTRAP_TOKEN_EXPIRED` | Token TTL exceeded | Same as above |
| `DEVICE_ID_CONFLICT` | Another device claims same `deviceId` | Prompt re-enrollment; clear `ccAgentToken` |
| `AUTH_FAILED` | Token/mTLS handshake failed | Retry with backoff; after 3 failures surface error |
| `CC_SERVER_UNREACHABLE` | WS connection refused/timeout | Exponential backoff; max 30s |
| `SESSION_EVICTED` | Server closed old session for this `deviceId` | Reconnect immediately (new session takes over) |
| `INVALID_PARAMS` | Malformed enroll/connect request | Surface error; do not auto-retry |

### Error surfacing rules
- Structured `{ code, message, details? }` in all error responses.
- Codes starting with `BOOTSTRAP_` are terminal; agent must prompt user.
- Codes starting with `AUTH_`, `CC_SERVER_`, `SESSION_` are retriable with backoff.
- `INVALID_PARAMS` is always terminal with explicit field-level message.
