# Phase 2 — Telemetry

## Scope
- Stream `pty-output`, `pty-exit`, `openvpn-log` to CC.
- Persist events in DB; expose SSE for Admin UI.

## Current implementation note

The current implementation now defines the Phase 2 storage plan:

- Implemented now:
  - agent forwarding for `function-status`, `pty-output`, `pty-exit`, `openvpn-log`
  - agent-side HTTP ingest batching at 15 ms / 16 KB max per batch
  - agent-side adaptive throttling with a 256 KB/s leaky-bucket soft cap and temporary cooldown on HTTP `429` / `5xx`
  - agent-side gzip compression for oversized HTTP ingest bodies (`> 8 KB`) sent to `POST /v1/logs`
  - agent-side chunking for oversized telemetry events, emitted as ordered chunk records with `{ chunkIndex, totalChunks }`
  - agent-side redaction before enqueue for structured secret fields and common password/OTP/token text patterns
  - agent retry/buffering queue now persists redacted records to disk and reads ready batches/retry metadata from the disk queue during send/retry/ack
  - durable queue now has app-config disk layout, manifest, append-only rotated segments, partial-tail recovery, atomic manifest rewrites, queue stats, compaction that removes fully acked old segments, AES-256-GCM encryption at rest, and bounded-capacity shedding with a 128 MB hard cap / 90% high-watermark
  - CC ingest endpoint: `POST /v1/logs`
  - persisted telemetry/history in SQLite
  - SSE stream: `GET /v1/logs/stream`
  - history/export endpoints: `GET /v1/logs`, `GET /v1/logs/export`
  - admin telemetry UI with filters, search, export, alerts, and runtime summaries
- Still deferred:
  - negotiated WS gzip capability and server-side chunk reassembly
  - full redaction matrix parity with the original Phase 2 design

Use this document as the target design for the current implementation. SQLite + the rotated encrypted disk queue are the intended Phase 2 baseline and storage path. Redacted telemetry records are now persisted on enqueue and used as the source of truth for ready-batch selection, retry metadata, and ack/drop. Rotation, compaction, encryption, bounded-capacity shedding, and validation coverage are now implemented.

## Deliverables
- Agent: batching implemented at 15 ms / 16 KB max batch size; tags `{functionId, sessionId}`.
- Durable agent queue: disk-backed, bounded (default 128 MB), AES-GCM encrypted at rest; drop-oldest for non-critical telemetry beyond the 90% high-watermark; flush-on-reconnect with de-dupe by `(deviceId, seq)`.
  - Implementation checklist: `docs/cc/cc-phase-2-durable-queue-checklist.md`
- CC (ingest + storage):
  - Ingest: `POST /v1/logs` (internal), `GET /v1/logs/stream?deviceId=...` (SSE)
  - Storage: SQLite (implemented and planned)
  - `GET /v1/logs` and export/filter/search are already implemented in the current server
- Compression/chunking: current reality is gzip-compressed HTTP ingest bodies > 8 KB plus agent-side chunk records for oversize telemetry events; negotiated WS gzip and server-side chunk reassembly remain deferred.
- Ordering: per `(deviceId, sessionId, type)`; reconnect replay up to N buffered events.
- Inventory publish: implemented on connect, every 5 minutes, and on detected server-list hash change; fields `{ serversListHash, serversCount }`.
## Checklists
- Agent: subscribe to Tauri events: `function-status`, `pty-output`, `pty-exit`, `openvpn-log`.
- Agent: redacted telemetry is persisted to the disk queue before it becomes sendable, and the live sender/retry path now reads ready records plus retry metadata from disk; segment rotation, compaction, AES-256-GCM at-rest encryption, and bounded-capacity shedding are implemented.
- Agent: queue validation now covers restart replay, offline growth/reconnect flush, per-stream ordering, corruption skip-and-continue behavior, auth-failure drop semantics, disk-pressure shedding, and disk-artifact secret checks.
- CC: ingest → durable store implemented with SQLite; HTTP ingest now returns `nack { rate_limited: true, retryAfterMs }` when saturated by rate limiting.
- CC: SSE/history fanout per-device implemented.
- Redaction parity with MCP is implemented for persisted telemetry, history/export, and SSE fanout (no secrets at-rest/in-flight).

## Backpressure and throttling
- Server backpressure
  - When ingest backlog exceeds thresholds (e.g., queue > 10k msgs or > 64MB), server responds with `nack { code: "rate_limited", retryAfterMs: 100..2000 }`.
  - SSE fanout backpressure: per-connection buffers capped; slow consumers are dropped with a retry hint.
- Agent adaptive throttling
  - Implemented baseline: local leaky-bucket with a 256KB/s soft cap and up to 5s burst budget, plus temporary cooldown when ingest returns HTTP `429` or `5xx`.
  - Implemented baseline: HTTP ingest returns `429` with a `nack` body carrying `rate_limited` + `retryAfterMs`, and the agent honors that server backoff hint.
  - Deferred from the original design: dynamic batch-window resizing tied to negotiated backpressure signals.
- Offline buffer policy: drop oldest non-critical telemetry when the disk queue projects above the 90% high-watermark; emit a local `cc-telemetry-health` signal and queue-pressure logs when shedding occurs.
- Compression and chunking
  - Implemented baseline: gzip HTTP ingest bodies when the serialized batch exceeds 8 KB.
  - Implemented baseline: oversize telemetry events are split into sequential chunk records that carry `{ chunkIndex, totalChunks, chunked: true }`.
  - Deferred from the original design: `welcome.capabilities.gzip=true` / `chunking=true` negotiation on the WS path and server-side chunk reassembly into a single event view.
- Ordering and replay
  - Ordering guaranteed per `(deviceId, sessionId, type)` by `seq`.
  - On reconnect, replay up to `N` buffered events; server de-duplicates by `(deviceId, seq)`.

## Storage schema (SQLite)
Target: SQLite as the primary append-oriented telemetry store for this project, with indexed time-range queries and straightforward export/backup flows.

Recommended logical shape:

- one `logs` table containing the normalized envelope fields already used by the current server
- indexed lookups on device/time/type for history and SSE catch-up queries
- redacted payload JSON stored as text
- retention handled by scheduled deletion/export jobs rather than engine TTL features

Suggested indexing goals:

- `(deviceId, ts)` for per-device timelines
- `(deviceId, type, ts)` for filtered telemetry views
- `(deviceId, sessionId, seq)` for ordered session replay

Notes
- redaction occurs before insert; stored payloads must already be redacted
- retention is enforced by application jobs described in Phase 7
- if higher volume ever requires a different store later, that should be treated as a separate migration project rather than an implicit Phase 2 assumption

## Backpressure and throttling
- Server backpressure: when HTTP ingest is saturated by rate limiting, respond with `429` and a `nack { rate_limited: true, retryAfterMs }` body; slow SSE consumers may still be dropped with a retry hint.
- Agent adaptive throttling: leaky-bucket per device (default 256KB/s soft cap for telemetry; bursts up to 1MB over 5s). When the server returns `nack.rate_limited=true`, the agent honors `retryAfterMs` and delays the next send attempt accordingly.
- Queue near-full policy: disk-backed queue uses a 128 MB hard cap and a 90% high-watermark. When projected usage crosses the watermark, it drops the oldest non-critical telemetry first; it never intentionally sheds `function-status`, `pty-exit`, or terminal lifecycle events.
- Health signals: current implementation emits a local Tauri `cc-telemetry-health` event and queue-pressure log entries when records are shed. A periodic server-visible degraded heartbeat remains deferred.
- Corruption handling: if one persisted record cannot be decoded or decrypted, the agent skips that record, logs the issue, and continues reading later records from the queue.
- Retry with jitter: exponential backoff (base 250ms, cap 5s) for transient HTTP ingest errors; dedupe by `(deviceId, seq)` prevents duplicates on re-send.


## Event schema and versioning
- Envelope v1: `{ type, deviceId, seq, ts, payload }`.
- `payload` per type has a `schemaVersion` field; server stores raw and normalized forms.
- Additive-only changes within a minor version; breaking changes bump `schemaVersion`.

## Ordering and delivery
- Ordering is guaranteed per `(deviceId, sessionId, type)` based on monotonically increasing `seq`.
- Interleaving across different `sessionId` streams is allowed.
- On reconnect, the agent may replay up to `N` buffered events; duplicates detected by `(deviceId, seq)`.

## Compression and size limits
- Current implementation: agent may gzip HTTP ingest bodies when aggregate payload > 8 KB.
- Deferred design: WS-level gzip still depends on `welcome.capabilities.gzip=true`.
- Per-event soft limit: 16 KB; oversize payloads are split into chunk records with `{ chunkIndex, totalChunks }`.

## Inventory publishing
- Agent publishes `inventory` with `{ serversListHash, serversCount }` on connect and every 5 minutes or when the hash changes.
- Server can request a full inventory snapshot via a Phase 3 control when hashes mismatch persistently.

## Acceptance Criteria
- Live logs appear in Admin UI end-to-end. Current implementation verified manually.
- Backfill: reconnect and restart replay now come from the persisted rotated encrypted disk queue, and queue validation covers replay, corruption skip, auth-failure drop semantics, and disk-pressure shedding.
- Queries by device/time/type return expected events and respect redaction rules.
- Ordering holds per `(deviceId, sessionId, type)` under burst conditions.

## Risks
- Volume spikes from PTY spam → batching + rate limits.
- Storage costs → compression + retention (Phase 7 finalizes).

## Implemented admin/server additions pulled forward

These items were implemented earlier than the original phased docs suggested:

- persisted device enrollment and agent tokens
- persisted runtime summaries per device
- admin login/logout with cookie session
- CSRF protection
- hashed agent tokens and hashed admin session tokens at rest
- security headers and request logging
- endpoint rate limiting

## SLOs (Phase 2 Telemetry)

## SLOs (Phase 2 Telemetry)

- Ingest latency (Agent → CC ingest): P50 < 150 ms, P95 < 500 ms
- Admin SSE end-to-end (Agent → CC → Admin): P50 < 250 ms, P95 < 800 ms
- Backpressure recovery (from `nack rate_limited` to normal): < 5 s P95
- Ordering correctness violations per 1M events: 0 for same (deviceId, sessionId, type)
- Dropped events due to size limits: 0 (must chunk instead)


## Agent queue encryption
- At-rest encryption: AES-256-GCM per-record with unique 96-bit nonce; MAC verified on read.
- Key source: random 256-bit key stored via OS keyring; key alias `scp.cc.telemetry.queue`; fallback to local encrypted file only if keyring unavailable.
- Rotation: generate new key on admin request; re-encrypt lazily on compaction; old key kept until compaction completes (max 24h) then wiped.
- Format: append-only segments with small index; each record `{nonce, ciphertext, tag}`; header includes key id and segment salt.
- Limits: default max on-disk size 128 MB; begin shedding oldest non-critical telemetry when projected usage exceeds 90% of that budget.
- Failure behavior: if decryption fails (corrupt), skip the record, log the corruption locally, and continue reading the rest of the queue. Server-visible corruption metrics/alerts remain deferred.
- Privacy: secrets already redacted at source; encryption here defends device compromise at-rest; does not replace transport TLS.

## Test plan (Phase 2 Telemetry)

## Test plan (Phase 2 Telemetry)

- Backpressure/adaptive throttling
  - Induce server `nack { rate_limited: true, retryAfterMs }` and verify agent backs off according to `retryAfterMs` and recovers.
  - Verify no data loss: post-recovery, all queued events are delivered in order.
- Durable queue replay
  - Kill agent mid-burst; on restart, verify disk queue replays without duplicates (keyed by `(deviceId, seq)`).
  - Corrupt one queued record; verify decrypt/verify fails, record is skipped with audit, rest proceed.
- Ordering and chunking
  - Interleave two PTY sessions; verify per-stream ordering preserved, cross-stream interleave allowed.
  - Send oversized events; verify gzip and `{chunkIndex, totalChunks}` chunking behaviors and server reassembly.
- Redaction parity
  - Inject payloads containing secrets; verify redaction at agent and server logs; SQLite history/export queries show redacted fields only.
- SSE fanout SLOs
  - Under 2× normal traffic, P95 latency within targets; backpressure signals sent before breach and recovery verified.
- Inventory publishing
  - Hash change triggers publish; stable state publishes every 5m; server can request full snapshot on mismatch.


## Redaction matrix
| Event type       | Sensitive fields                                  | Redaction rule                                  | Notes |
|------------------|----------------------------------------------------|--------------------------------------------------|-------|
| function-status  | message (may include creds), boundaryTokenSource   | Drop secrets; redact tokens/passwords as `***`   | Only stable reason codes/messages allowed; avoid echoing raw CLI lines that may contain secrets. |
| pty-output       | data                                               | Best-effort redact passwords/OTP prompts/echoes  | Heuristics for common prompts ("Password:", "OTP:"); never log raw stdin from secret prompts. |
| pty-exit         | reason                                             | None                                             | No secrets normally present. |
| openvpn-log      | data                                               | Drop credentials; redact private IPs if policy   | Follow OpenVPN verbosity; strip auth lines; optionally mask IPs per org policy. |
| inventory        | serversListHash, counts                            | None                                             | Hash only (no server details in Phase 2). |

- Redaction enforcement points:
  - Agent: apply prompt/echo masking before enqueue.
  - Server ingest: second-pass regex redaction; reject frame if redaction fails for known patterns.
- Redaction tests: cover password/OTP prompts, boundary auth logs, and OpenVPN connect output.

## Redaction matrix

| Event type       | Potentially sensitive fields                 | Redaction rule                                  |
|------------------|----------------------------------------------|-------------------------------------------------|
| pty-output       | passwords, OTPs, tokens in terminal output   | inline redact via agent filter; drop secrets    |
| pty-exit         | none                                         | n/a                                             |
| openvpn-log      | credentials, IPs, tokens                     | redact credentials/tokens; keep non-PII IPs     |
| function-status  | messages may include paths/usernames         | redact usernames if present in error strings    |
| inventory        | server usernames, paths                      | redact passwords/tokens; allow usernames/paths  |

- Redaction implementation: apply deterministic regex-based filters in agent before enqueue; server-side secondary redaction on ingest as safety net.
- Current agent baseline:
  - structured keys such as `password`, `otpSecret`, `oidcPassword`, `oidcOtpSecret`, `mcpAuthToken`, `ccAgentToken`, `token`, `secret`, and `authorization` are replaced with `[redacted]`
  - string payloads are scrubbed for common `Bearer ...`, `password: ...`, `otp: ...`, `token: ...`, and `secret: ...` patterns before enqueue
- Never log secrets at server; audits/events store redacted forms only.
