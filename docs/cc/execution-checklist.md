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

**Last Updated:** 2026-04-11T00:00:00Z
**Status:** Phase 1 agent/server bootstrap is implemented end-to-end; Phase 2 core telemetry/admin slice is implemented; Phase 3 safe-control slice now includes durable command history, lifecycle/audit tracking, idempotent replay protection, reconnect reconciliation, and the current safe command set; Phase 4 interactive PTY is implemented and E2E-verified

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
- ✅ Extend CC protocol handling for server → agent messages
- ✅ Add command dispatcher in `src-tauri/src/cc/`
- ✅ Map safe server commands to existing local actions
- ✅ Emit lifecycle updates back to CC via explicit `command_status` frames:
  - acknowledged
  - executing
  - terminal result
- ✅ Add duplicate command suppression by `commandId` with TTL cache
- ✅ Add timeout handling for non-terminal commands

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
- ✅ Reconnect-related OpenVPN restart issues were addressed in the current helper-based implementation
- ✅ Audit trail matches actual executed actions

---

## Phase 4 — Interactive PTY

**Goal:** Add remote PTY control over CC with a live Admin terminal, safe session lifecycle handling, and PTY-specific hardening.

**Reference:** `docs/cc/cc-phase-4-interactive-pty.md`

**Phase 4 decisions locked in:**
- server-generated `sessionId` is the canonical remote PTY identity
- agent may keep a local PTY id internally, but must map it to `sessionId`
- remote PTY sessions are single-operator owned in Phase 4
- same-operator reattach is allowed; different-operator attach to a live owned PTY is rejected
- `pty_open` and `pty_attach` are distinct concepts; the first Admin UX may combine them as open-and-attach
- browser disconnect acts as detach, not immediate PTY close
- default reattach grace window: 30s
- default detached hard idle timeout: 5m
- agent ↔ cc-server uses the existing `/agent` websocket
- Admin UI ↔ cc-server uses a separate browser-facing PTY websocket
- browser-facing PTY transport uses WebSocket, not SSE
- audit is required for PTY open/attach/reject/detach/close/expire and recording consent

### Recommended implementation order
1. extend CC protocol and dispatch for remote PTY lifecycle
2. add agent-side PTY session mapping / multiplexing for CC-owned sessions
3. add server-side PTY session relay/state handling
4. add minimal Admin live terminal UI
5. add PTY hardening: backpressure, idle GC, scrollback replay, RBAC/recording

### Protocol / CC transport
- ✅ Add protocol messages for `open_pty`, `write_pty`, `resize_pty`, `close_pty`
- ✅ Define/align `pty-output` and `pty-exit` remote-session payload contracts with `{ sessionId }`
- ✅ Add PTY-specific correlation fields and stable error/nack handling for PTY control requests
- ✅ Keep `pty_open` and `pty_attach` distinct in the model while allowing open-and-attach in the first Admin UX

### Agent (Tauri/Rust)
- ✅ Add CC command handling for remote `open_pty`, `write_pty`, `resize_pty`, `close_pty`
- ✅ Map server-issued remote `sessionId` values onto the existing local PTY runtime/session store safely
- ✅ Forward PTY output/exit events for CC-owned sessions
- ✅ Add PTY multiplexing and slow-consumer backpressure handling
- ✅ Add detach-aware cleanup/GC for abandoned remote PTY sessions using the 30s grace / 5m idle policy

### CC Server
- ✅ Add durable `pty_sessions` state tracking between Admin and agent websocket
- ✅ Add authorization checks for PTY open/write/resize/close
- ✅ Add single-operator ownership enforcement plus attach/detach handling and session cleanup on disconnect
- ✅ Expose a browser-facing PTY websocket transport for Admin UI

### Admin / API
- ✅ Add minimal live terminal UI for a device PTY session
- ✅ Support terminal input, resize, close, and reconnect/reattach flows over the browser-facing PTY websocket
- ✅ Replay recent scrollback on attach
- ✅ Surface recording/privacy state when enabled
- ✅ Treat "Open terminal" as open-and-attach, while reconnect uses attach-only semantics

### PTY hardening
- ✅ Enforce UTF-8/input sanitization and disallowed control-sequence filtering
- ✅ Add keystroke and resize rate limits with `nack { rate_limited: true }`
- ✅ Add bounded scrollback with truncation notice
- ✅ Add RBAC + consent/audit behavior for record-enabled sessions
- ✅ Audit PTY open/attach/reject/detach/close/expire and recording consent without storing raw PTY input

### Acceptance Criteria
- ✅ Admin can open a remote PTY and interact with it end-to-end over CC
- ✅ `write_pty` / `resize_pty` / `close_pty` work reliably under reconnect conditions
- ✅ PTY output and exit events are correlated correctly by `sessionId`
- ✅ Same-operator reattach works within the grace window without spawning a new PTY
- ✅ Different-operator attach to an owned live PTY is rejected deterministically
- ✅ Slow-consumer/backpressure behavior is bounded and visible
- ✅ Idle/abandoned PTY sessions are cleaned up automatically
- ✅ Scrollback replay works and truncates with notice
- ✅ RBAC/recording/privacy behavior is enforced and auditable


---

## Phase 5 — File Operations

**Goal:** Add durable, auditable remote file operations over CC, including transfer progress/history and remote-browser-backed path workflows.

**Reference:** `docs/cc/cc-phase-5-file-ops.md`

### Recommended implementation order
1. align protocol/contracts for file ops and remote browser messages
2. add durable transfer/session state on cc-server
3. wire agent-side SCP/SFTP execution + progress/lifecycle reporting
4. add Admin UI for remote browser + transfer queue/history
5. add reliability/safety hardening: overwrite policy, atomic writes, quotas, tunnel precheck, integrity checks

### Protocol / CC transport
- ✅ Add protocol messages for `scp_upload`, `scp_download`, `start_scp_upload`, `start_scp_download`
- ✅ Add protocol messages for remote browser lifecycle (`open_remote_browser`, `list_remote_browser_children`, `stat_remote_browser_path`, `create_remote_browser_directory`, `delete_remote_browser_path`, `close_remote_browser`)
- ✅ Define stable transfer progress / terminal-status payloads and reason codes
- ✅ Define overwrite / follow-symlink / rate-limit / checksum flags in the contract

### Agent (Tauri/Rust)
- ✅ Add CC command handling for SCP upload/download and remote browser operations
- ✅ Reuse existing SCP + persistent SFTP browser session implementation safely for CC-owned sessions
- ✅ Emit transfer lifecycle + progress updates back to CC with stable transfer identifiers
- ✅ Add Boundary tunnel precheck before Boundary-backed SCP work
- ✅ Add retry/resume behavior for long-running transfers
  - ✅ timeout handling
  - ✅ minimal automatic retry for transient start/pre-transfer failures
  - ✅ true resumable interrupted-transfer support with validated resume offset/state

### CC Server
- ✅ Add durable transfer/session storage for file ops and remote browser state
- ✅ Add authorization checks for file ops and browser mutations
- ✅ Add transfer history / progress relay between Admin UI and agent websocket
- ✅ Enforce quota / rate-limit behavior with stable reason codes for the current Phase 5 slice
- ✅ Audit browser mutations and transfer lifecycle without storing secrets
- ✅ Persist resume metadata and recovery state for interrupted transfers

### Admin / API
- ✅ Add remote browser UI over CC for choosing upload/download paths
- ✅ Add transfer queue/history UI with running, success, and failure states
- ✅ Surface overwrite confirmations, tunnel-precheck failures, and stable error reasons
- ✅ Support upload/download initiation from browser selections and manual-path fallback
- ✅ Add resume-aware transfer UX for interrupted transfers

### File-op hardening
- ✅ Enforce path normalization and invalid-target rejection across platforms
- ✅ Add overwrite policy support (`prompt`, `force`, `skip`)
- ✅ Add atomic upload temp-path + rename flow
- ✅ Add optional checksum verification reporting
- ✅ Add symlink-handling policy and audit notes

### Acceptance Criteria
- ✅ Upload/download works end-to-end over CC with visible continuous progress
- ✅ Transfer history survives refresh/reconnect and reflects terminal outcomes deterministically
- ✅ Remote browser operations reflect remote FS changes after refresh
- ✅ Boundary-backed SCP is rejected clearly when the tunnel is not running
- ✅ Atomic upload flow prevents partial destination files on success path
- ✅ Overwrite / checksum / path-safety behavior is enforced and auditable
- ✅ Interrupted transfers expose truthful retry/resume state and resume only when validated state is trustworthy
