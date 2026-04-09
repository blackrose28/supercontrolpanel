# Phase 2 Durable Queue Checklist

Track the implementation of the agent-side durable disk-backed telemetry queue as a sequence of small, verifiable steps.

This checklist is scoped to the Tauri/Rust agent in `SuperControlPanel`. It does not cover the server-side SQLite telemetry store work.

## Goal

Replace the current in-memory-only retry queue with a disk-backed encrypted queue that:

- survives app restarts
- preserves replay order
- keeps already-redacted payloads only
- supports bounded storage with drop policy
- flushes safely after reconnect

## Current baseline

- Telemetry is already redacted before enqueue.
- Telemetry is already chunked, compressed, batched, and throttled before ingest.
- Retry/backoff exists, but the queue is memory-only.
- Reconnect replay works only within the current app process lifetime.

## Recommended execution order

1. introduce queue abstraction without behavior change
2. add disk queue file layout and manifest
3. persist enqueue path to disk
4. read/send/ack from disk
5. compact old segments
6. add encryption at rest
7. add bounded-capacity drop policy
8. verify restart replay, corruption handling, and degraded behavior

## Checklist

### 1. Queue abstraction
- [x] Extract a queue interface from `src-tauri/src/cc/telemetry.rs`
- [x] Keep current in-memory queue behind that interface as the first implementation
- [x] Define queue record shape:
  - `localId`
  - `seq`
  - `body`
  - `createdAt`
  - `attempt`
  - `nextAttemptAt`
- [x] Keep sender logic working with no behavior change before disk persistence is added

### 2. Disk queue layout
- [x] Create durable queue directory under the app config dir
- [x] Add manifest file for:
  - active segment
  - read cursor
  - ack cursor
  - total bytes
  - key id / format version
- [x] Add append-only segment file format
- [x] Pick initial segment rotation size
  - recommended: `4 MB` or `8 MB`
- [x] Add startup recovery for partially written tail records

### 3. Persisted enqueue
- [x] Write redacted telemetry records to disk before they are eligible for sending
- [x] Preserve current enqueue ordering semantics
- [x] Ensure chunked events are written as separate ordered records
- [x] Ensure only already-redacted payloads are written to disk
- [x] Keep `recent_log_events()` behavior intact for current CC control flows

### 4. Read/send/ack flow
- [x] Read ready records from disk into batch assembly
- [x] Keep existing batching/throttling/compression behavior on the send path
- [x] Mark records acked only after successful `/v1/logs` response
- [x] Persist retry metadata needed after restart
- [x] Ensure unauthorized responses still drop records intentionally

### 5. Compaction and rotation
- [x] Rotate segments when size threshold is reached
- [x] Delete fully acked old segments
- [x] Keep manifest/cursor updates crash-tolerant
- [x] Add queue stats helper:
  - queued records
  - queued bytes
  - oldest queued age

### 6. Encryption at rest
- [x] Generate random queue encryption key
- [x] Store key in OS keyring
- [x] Encrypt each record with AES-256-GCM
- [x] Store nonce + ciphertext + tag per record
- [x] Version the on-disk format for future migration
- [x] Define behavior if keyring is unavailable
  - recommended initial policy: fail closed for durable mode and fall back explicitly to in-memory only

### 7. Capacity and drop policy
- [x] Add maximum on-disk queue size
  - implemented default: `128 MB`
- [x] Add high-watermark behavior
  - implemented: degraded shedding starts at `90%`
- [x] Drop oldest non-critical telemetry first when near full
- [x] Never drop:
  - `function-status`
  - `pty-exit`
  - terminal lifecycle signals
- [x] Emit degraded-health signal/log when dropping occurs
  - current implementation emits a local `cc-telemetry-health` Tauri event and writes a queue-pressure log line

### 8. Validation
- [x] Restart test: queue survives app restart and replays remaining records
- [x] Offline test: queue grows while server is unavailable, then flushes on reconnect
- [x] Ordering test: per `(deviceId, sessionId, type)` ordering still holds
- [x] Corruption test: one bad record is skipped without blocking the queue
- [x] Auth failure test: `401` / `403` still drop queued records as intended
- [x] Disk full test: degraded policy triggers without crashing the app
- [x] Verify no unredacted secret reaches disk artifacts

## Suggested file split

- `src-tauri/src/cc/telemetry.rs`
  - orchestration only
- `src-tauri/src/cc/telemetry_queue.rs`
  - queue trait
  - in-memory queue
  - shared queue record types
- `src-tauri/src/cc/telemetry_queue_disk.rs`
  - manifest
  - segment IO
  - recovery
- `src-tauri/src/cc/telemetry_queue_crypto.rs`
  - keyring + AES-GCM helpers

## Exit criteria

Mark the durable queue hole complete only when all of these are true:

- queue contents survive full app restart
- pending records replay automatically after reconnect
- disk artifacts contain only redacted payloads
- queue is bounded and compacts old acknowledged data
- corruption and disk-pressure paths are handled explicitly
