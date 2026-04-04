# Phase 2 — Telemetry

## Scope
- Stream `pty-output`, `pty-exit`, `openvpn-log` to CC.
- Persist events in DB; expose SSE for Admin UI.

## Deliverables
- Agent: batching (10–50 ms / 8–16 KB), tags `{functionId, sessionId}`.
- CC: `POST /v1/logs` (internal), `GET /v1/logs/stream?deviceId=...` (SSE), `GET /v1/logs` (query).
- Storage: Postgres (Timescale or partitioned) or ClickHouse (preferred for volume).

## Checklists
- Agent: subscribe to Tauri events: `function-status`, `pty-output`, `pty-exit`, `openvpn-log`.
- Agent: offline ring buffer (bounded), drop-oldest policy.
- CC: WS → ingestor → durable store; backpressure handling.
- CC: SSE fanout per-device and per-session.
- Redaction parity with MCP (no secrets at-rest/in-flight).

## Event schema and versioning
- Envelope v1: `{ type, deviceId, seq, ts, payload }`.
- `payload` per type has a `schemaVersion` field; server stores raw and normalized forms.
- Additive-only changes within a minor version; breaking changes bump `schemaVersion`.

## Ordering and delivery
- Ordering is guaranteed per `(deviceId, sessionId, type)` based on monotonically increasing `seq`.
- Interleaving across different `sessionId` streams is allowed.
- On reconnect, the agent may replay up to `N` buffered events; duplicates detected by `(deviceId, seq)`.

## Compression and size limits
- Agent may gzip frames when aggregate payload > 8 KB; server advertises support via `welcome.capabilities.gzip=true`.
- Per-event soft limit: 16 KB; oversize payloads are chunked with `{chunkIndex, totalChunks}`.

## Inventory publishing
- Agent publishes `inventory` with `{ serversListHash, serversCount }` on connect and every 5 minutes or when the hash changes.
- Server can request a full inventory snapshot via a Phase 3 control when hashes mismatch persistently.

## Acceptance Criteria
- Live logs appear in Admin UI within <250 ms P50.
- Backfill: reconnect replays up to N buffered events without duplicates.
- Queries by device/time/type return expected events and respect redaction rules.
- Ordering holds per `(deviceId, sessionId, type)` under burst conditions.

## Risks
- Volume spikes from PTY spam → batching + rate limits.
- Storage costs → compression + retention (Phase 7 finalizes).
