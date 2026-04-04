# Phase 1 — Bootstrap

## Goals
- Establish secure outbound WS connection from agent to CC.
- Stream `heartbeat` and `function-status` telemetry.
- Provide minimal Admin API + in-memory registry to view devices.

## Deliverables
- CC Server (default: NestJS)
  - WebSocket `wss /agent` (subprotocol `scp.cc.v1`, `Authorization: Bearer <agentToken>`)
  - HTTP Admin: `GET /v1/devices`, `GET /v1/devices/{id}`
  - In-memory stores for `devices`, `events`
  - Simple Admin page (server-rendered) listing devices + lastSeenAt
- Agent (this app)
  - New module `src-tauri/src/cc/` with `connection`, `protocol`, `telemetry`
  - Config keys in `sync-local.json`: `ccEnabled`, `ccServerUrl`, `ccAuthMode`, `ccAgentToken`, `ccDeviceId`
  - Heartbeat every 15s: `{ uptime, version, platform, appId }`
  - Forward `function-status` events to CC
  - Reconnect with jitter and exponential backoff
- Frontend
  - Config page: Control plane section (URL, token, enable toggle)
  - Status pill: "CC connected" with last sync time

## Protocol (v1 minimal)
- Envelope: `{ type, deviceId, seq, ts, payload }`
- Agent → CC: `heartbeat`, `function-status`
- CC → Agent: none in Phase 1 (control disabled)
- Acks: CC responds `{ type: 'ack', reqSeq }` for telemetry batches (optional)

## API Sketch
- `GET /v1/devices` → `[ { id, name, platform, version, lastSeenAt, status } ]`
- WS `/agent` → accepts frames above; authenticates token; updates lastSeenAt; stores last N events (ring buffer)

## Data Model (in-memory)
- `devices`: `Map<deviceId, { id, name, tags, platform, version, lastSeenAt, status }>`
- `events`: `Map<deviceId, CircularBuffer<Event>>(capacity=1000)`

## Acceptance Criteria
- When `ccEnabled=true` and valid `ccServerUrl`+`ccAgentToken`, agent connects within 5s and maintains heartbeats.
- Admin `GET /v1/devices` shows the device with updated `lastSeenAt` while connected.
- Toggling `ccEnabled` off closes connection within 2s.
- Losing network triggers reconnect with backoff; resumes within 60s after network returns.

## Risks & Mitigations
- Token leakage → store only in device-local `sync-local.json`; never sync to repo; redact in logs.
- Event flood → batch telemetry (10–50ms or 8–16KB) and drop-oldest ring buffer on CC.
- Clock skew → CC uses receive time for `lastSeenAt`.

## Next (Phase 2 Preview)
- Add `pty-output` and `openvpn-log` streaming; persist to Postgres/ClickHouse; SSE for admin UI.
