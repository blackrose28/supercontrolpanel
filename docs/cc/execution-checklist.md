# CC Execution Checklist

Track implementation progress for the Control Plane. Each phase has concrete deliverables and acceptance criteria defined in the corresponding phase doc.

**Status Legend:**
- ⬜ Not started
- 🟨 In progress
- ✅ Complete
- 🔴 Blocked

---

## Phase 1 — Bootstrap

**Goal:** Establish secure outbound WS connection, stream telemetry, minimal Admin API.

**Reference:** `docs/cc/cc-phase-1-bootstrap.md`

### CC Server (NestJS)
- ✅ Project scaffold (NestJS + TypeScript)
- ✅ WebSocket `/agent` endpoint (subprotocol `scp.cc.v1`)
- ✅ In-memory stores: `devices`, `sessions`
- ✅ HTTP Admin API: `GET /v1/devices`, `GET /v1/devices/:id`, `GET /v1/health`
- ✅ Simple Admin page (server-rendered): device list with `id`, `name`, `lastSeenAt`, `status`
- ✅ Enrollment: `POST /v1/enroll` (bootstrap token → `{ deviceId, ccAgentToken }`)
- ✅ Duplicate-connection policy: evict old session when new connects with same `deviceId`

### Agent (Tauri/Rust)
- ✅ New module `src-tauri/src/cc/` with `connection.rs`, `protocol.rs`, `dispatcher.rs`
- ✅ Config in `sync-local.json`: `ccEnabled`, `ccServerUrl`, `ccAuthMode`, `ccAgentToken?`, `ccDeviceId?`
- ✅ WS client with reconnect (jittered exponential backoff: base 500ms, max 30s)
- ✅ Heartbeat every 15s: `heartbeat { uptimeSec, versions }`
- ✅ Initial `status_snapshot` on connect: `{ activeSessions, lastSeqByStream }`
- ✅ Forward `function-status` events to CC (dispatcher ready)

### Frontend (Vite/TypeScript)
- ✅ Config page: Control plane section (URL, token, enable toggle)
- ✅ Status pill: CC connection state

### Acceptance Criteria
- ✅ Secure WS connect succeeds with valid token; rejected with clear error when invalid
- ✅ Device appears in Admin list within 5s with `lastSeenAt` updating
- ✅ Heartbeat every 15s received; jitter/backoff respects limits
- ✅ Duplicate connection policy enforced; audit reflects eviction
- ✅ `status_snapshot` sent on connect; server stores and renders current state
- ✅ Telemetry forwarding (`function-status`) flows end-to-end

---

## Phase 2 — Telemetry

**Goal:** Stream PTY/OpenVPN logs to CC, persist in ClickHouse, expose SSE for Admin UI.

**Reference:** `docs/cc/cc-phase-2-telemetry.md`

### Agent
- ⬜ Subscribe to Tauri events: `function-status`, `pty-output`, `pty-exit`, `openvpn-log`
- ⬜ Batching (10–50 ms / 8–16 KB)
- ⬜ Durable disk-backed queue (AES-256-GCM encrypted, 128 MB cap, drop-oldest policy)
- ⬜ Adaptive throttling (256 KB/s cap, leaky-bucket)
- ⬜ Compression (gzip frames > 8 KB if negotiated)
- ⬜ Chunking (oversize events with `{chunkIndex, totalChunks}`)
- ⬜ Redaction (apply Phase 2 redaction matrix before enqueue)

### CC Server
- ⬜ Ingest: `POST /v1/logs` (internal)
- ⬜ ClickHouse setup: `cc_events_raw` table with partitioning and TTL
- ⬜ SSE: `GET /v1/logs/stream?deviceId=...`
- ⬜ Backpressure: `nack { rate_limited: true, retryAfterMs }` when saturated
- ⬜ Inventory: handle `inventory { serversListHash, serversCount }`

### Acceptance Criteria
- ⬜ Live logs appear in Admin UI within <250 ms P50
- ⬜ Backfill: reconnect replays buffered events without duplicates
- ⬜ Ordering holds per `(deviceId, sessionId, type)` under burst
- ⬜ Redaction: no secrets in SSE or stored rows
- ⬜ Disk queue survives process restarts

---

**Last Updated:** 2026-04-06T11:06:52Z
**Status:** Phase 1 complete; Phase 2 telemetry pending
