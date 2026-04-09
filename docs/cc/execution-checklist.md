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
- ✅ Device records are persisted in SQLite; live WS sessions remain in-memory
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
- ✅ CC connection state is shown on the Status page as a normal runtime card

### Acceptance Criteria
- ✅ Secure WS connect succeeds with valid token; rejected with clear error when invalid
- ✅ Device appears in Admin list within 5s with `lastSeenAt` updating
- ✅ Heartbeat every 15s received; jitter/backoff respects limits
- ✅ Duplicate connection policy enforced; audit reflects eviction
- ✅ `status_snapshot` sent on connect; server stores and renders current state
- ✅ Telemetry forwarding (`function-status`) flows end-to-end

---

## Phase 2 — Telemetry

**Goal:** Stream PTY/OpenVPN logs to CC, persist telemetry, expose history + SSE for Admin UI.

**Reference:** `docs/cc/cc-phase-2-telemetry.md`

### Agent
- ✅ Subscribe to Tauri events: `function-status`, `pty-output`, `pty-exit`, `openvpn-log`
- ✅ Batching (15 ms / 16 KB max batch to `POST /v1/logs`)
- ✅ Retry/buffer queue now persists and reads from the rotated encrypted disk queue; compaction, rotation, encryption, and bounded-capacity shedding are implemented (validation remains tracked in `docs/cc/cc-phase-2-durable-queue-checklist.md`)
- ✅ Adaptive throttling (256 KB/s soft cap via leaky-bucket; extra cooldown on HTTP 429/5xx)
- ✅ Compression (gzip HTTP ingest bodies > 8 KB to `POST /v1/logs`; WS-negotiated gzip still deferred)
- ✅ Chunking (oversize telemetry events split into ordered chunk records with `{chunkIndex, totalChunks}`)
- ✅ Redaction (structured secret fields + common password/OTP/token text patterns scrubbed before enqueue)

### CC Server
- ✅ Ingest: `POST /v1/logs`
- ✅ Durable storage implemented with SQLite as the chosen Phase 2 telemetry store
- ✅ SSE: `GET /v1/logs/stream?deviceId=...`
- ✅ History API: `GET /v1/logs`, `GET /v1/logs/export`
- ✅ Admin telemetry UI: filters, search, export, alerts, runtime summaries
- ✅ Backpressure: `nack { rate_limited: true, retryAfterMs }` when saturated
- ✅ Inventory summary path exists; full inventory cadence/request flow is implemented

### Acceptance Criteria
- ✅ Live logs appear in Admin UI end-to-end
- ✅ Backfill: reconnect and restart replay now come from the rotated encrypted disk queue, with validation coverage in `docs/cc/cc-phase-2-durable-queue-checklist.md`
- ✅ Ordering holds per `(deviceId, sessionId, type)` in the agent queue validation suite
- ✅ Redaction: no secrets in SSE or stored rows
- ✅ Disk queue survives process restarts (see `docs/cc/cc-phase-2-durable-queue-checklist.md`)

---

**Last Updated:** 2026-04-09T00:00:00Z
**Status:** Phase 1 agent/server bootstrap is implemented end-to-end; Phase 2 core telemetry/admin slice is implemented; Phase 3 safe-control slice now includes durable command history, lifecycle/audit tracking, idempotent replay protection, reconnect reconciliation, and the current safe command set; Phase 4 interactive PTY remains design-only

---

## Phase 3 — Core Control

**Goal:** Add safe server → agent control commands on top of the existing CC connection.

**Reference:** `docs/cc/cc-phase-3-control.md`

### Recommended implementation order
1. request fresh `status_snapshot`
2. request fresh `inventory`
3. one low-risk start/stop command mapped to an existing Tauri command
4. command persistence + lifecycle read APIs
5. audit trail + idempotency + reconnect reconciliation

### CC Server
- ✅ Add command store (`commands`) with lifecycle state:
  - `queued`
  - `dispatched`
  - `executing`
  - `succeeded`
  - `failed`
  - `canceled`
- ✅ Add `POST /v1/commands`
- ✅ Add `GET /v1/commands`
- ✅ Add `GET /v1/commands/:id`
- ✅ Add `GET /v1/commands/:id/audit`
- ✅ Device-scoped control endpoints now create durable command rows and dispatch the current safe command set over `/agent`
- ✅ Make existing device-scoped control endpoints create durable command rows and return `{ command }`
- ✅ Track command audit entries with actor, device, payload summary, lifecycle timestamps, outcome
- ✅ Add idempotent command replay/conflict handling by `commandId`
- ✅ Reconcile stale in-flight commands after reconnect/lost agent

### Agent (Tauri/Rust)
- 🟨 Extend CC protocol handling for server → agent messages
- 🟨 Add command dispatcher in `src-tauri/src/cc/`
- 🟨 Map safe server commands to existing local actions
- 🟨 Emit lifecycle updates back to CC via explicit `command_status` frames:
  - acknowledged
  - executing
  - terminal result
- ✅ Add duplicate command suppression by `commandId` with TTL cache
- ⬜ Add timeout handling for non-terminal commands

### Admin / API
- ✅ Add a minimal HTTP/admin flow for safe control testing
- ✅ Show latest outcomes through command history, audit entries, and telemetry/runtime views
- ✅ Show recent command history and audit entries as a first-class per-device list

### First safe command set
- ✅ `request_status_snapshot`
- ✅ `request_inventory`
- ✅ `request_recent_logs`
- ✅ `restart_openvpn`
- ✅ `restart_boundary_auth`
- ✅ `restart_server_tunnel`
- ✅ `restart_server_ssh`

### Acceptance Criteria
- ✅ Server can trigger implemented safe commands and see deterministic outcomes through lifecycle/audit state
- ✅ Agent executes current safe commands reliably under retry/reconnect conditions
- ✅ Each accepted control request returns a durable `commandId`
- ✅ Explicit `command_status` lifecycle updates flow end-to-end for the current safe command set
- ✅ Duplicate `commandId` does not produce duplicate side effects
- 🟨 Reconnect-related OpenVPN restart issues were addressed in the current helper-based implementation
- ✅ Audit trail matches actual executed actions
