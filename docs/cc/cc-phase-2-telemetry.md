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

## Acceptance Criteria
- Live logs appear in Admin UI within <250 ms P50.
- Backfill: reconnect replays up to N buffered events.
- Queries by device/time/type return expected events.

## Risks
- Volume spikes from PTY spam → batching + rate limits.
- Storage costs → compression + retention (Phase 7 finalizes).
