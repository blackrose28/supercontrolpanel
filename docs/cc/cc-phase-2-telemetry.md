# Phase 2 — Telemetry

## Scope
- Stream `pty-output`, `pty-exit`, `openvpn-log` to CC.
- Persist events in DB; expose SSE for Admin UI.

## Current implementation note

The current implementation diverges from the original Phase 2 storage plan:

- Implemented now:
  - agent forwarding for `function-status`, `pty-output`, `pty-exit`, `openvpn-log`
  - agent-side HTTP ingest batching at 15 ms / 16 KB max per batch
  - agent-side adaptive throttling with a 256 KB/s leaky-bucket soft cap and temporary cooldown on HTTP `429` / `5xx`
  - agent-side gzip compression for oversized HTTP ingest bodies (`> 8 KB`) sent to `POST /v1/logs`
  - agent-side chunking for oversized telemetry events, emitted as ordered chunk records with `{ chunkIndex, totalChunks }`
  - agent-side redaction before enqueue for structured secret fields and common password/OTP/token text patterns
  - agent retry/buffering queue in memory
  - durable queue scaffolding: app-config disk layout, manifest, active append-only segment, and partial-tail recovery
  - CC ingest endpoint: `POST /v1/logs`
  - persisted telemetry/history in SQLite
  - SSE stream: `GET /v1/logs/stream`
  - history/export endpoints: `GET /v1/logs`, `GET /v1/logs/export`
  - admin telemetry UI with filters, search, export, alerts, and runtime summaries
- Still deferred:
  - ClickHouse
  - persisted enqueue/send/ack on the disk-backed encrypted agent queue
  - negotiated WS gzip capability and server-side chunk reassembly
  - explicit `nack`/server backpressure protocol
  - full redaction matrix parity with the original Phase 2 design

Use this document as the target design, but treat SQLite + in-memory agent retry as the current Phase 2 baseline. The disk queue layout now exists on disk, but live telemetry replay still depends on the in-memory queue until persisted enqueue/send/ack is wired in.

## Deliverables
- Agent: batching implemented at 15 ms / 16 KB max batch size; tags `{functionId, sessionId}`.
- Durable agent queue: disk-backed, bounded (default 50 MB), AES-GCM encrypted at rest; drop-oldest policy beyond high-watermark; flush-on-reconnect with de-dupe by `(deviceId, seq)`.
  - Implementation checklist: `docs/cc/cc-phase-2-durable-queue-checklist.md`
- CC (ingest + storage):
  - Ingest: `POST /v1/logs` (internal), `GET /v1/logs/stream?deviceId=...` (SSE)
  - Current storage: SQLite (implemented)
  - Target storage: ClickHouse (deferred) — see "Storage schema (ClickHouse)" below
  - `GET /v1/logs` and export/filter/search are already implemented in the current server
- Compression/chunking: current reality is gzip-compressed HTTP ingest bodies > 8 KB plus agent-side chunk records for oversize telemetry events; negotiated WS gzip and server-side chunk reassembly remain deferred.
- Ordering: per `(deviceId, sessionId, type)`; reconnect replay up to N buffered events.
- Inventory publish: cadence 5m or on change; fields `{ serversListHash, serversCount }`.
## Checklists
- Agent: subscribe to Tauri events: `function-status`, `pty-output`, `pty-exit`, `openvpn-log`.
- Agent: retry/buffer queue implemented in memory; durable disk layout/manifest scaffold is implemented, but offline replay across process restarts is still pending.
- CC: ingest → durable store implemented with SQLite; backpressure handling still pending.
- CC: SSE/history fanout per-device implemented.
- Redaction parity with MCP (no secrets at-rest/in-flight).

## Backpressure and throttling
- Server backpressure
  - When ingest backlog exceeds thresholds (e.g., queue > 10k msgs or > 64MB), server responds with `nack { code: "rate_limited", retryAfterMs: 100..2000 }`.
  - SSE fanout backpressure: per-connection buffers capped; slow consumers are dropped with a retry hint.
- Agent adaptive throttling
  - Implemented baseline: local leaky-bucket with a 256KB/s soft cap and up to 5s burst budget, plus temporary cooldown when ingest returns HTTP `429` or `5xx`.
  - Deferred from the original design: protocol-level `nack { rate_limited, retryAfterMs }` handling and dynamic batch-window resizing tied to negotiated backpressure signals.
- Offline buffer policy: drop-oldest when disk queue near full; emit health warning and include dropped count in next `heartbeat`.
- Compression and chunking
  - Implemented baseline: gzip HTTP ingest bodies when the serialized batch exceeds 8 KB.
  - Implemented baseline: oversize telemetry events are split into sequential chunk records that carry `{ chunkIndex, totalChunks, chunked: true }`.
  - Deferred from the original design: `welcome.capabilities.gzip=true` / `chunking=true` negotiation on the WS path and server-side chunk reassembly into a single event view.
- Ordering and replay
  - Ordering guaranteed per `(deviceId, sessionId, type)` by `seq`.
  - On reconnect, replay up to `N` buffered events; server de-duplicates by `(deviceId, seq)`.

## Storage schema (ClickHouse)
Target: ClickHouse for high-volume append-only telemetry with fast time-range queries and fanout.

```sql
-- Raw events (normalized envelope + selected payload fields)
CREATE TABLE IF NOT EXISTS cc_events_raw (
  ts DateTime64(3, 'UTC') CODEC(Delta, ZSTD(6)),
  deviceId String CODEC(ZSTD(6)),
  type LowCardinality(String) CODEC(ZSTD(6)),
  sessionId String CODEC(ZSTD(6)),
  seq UInt64 CODEC(Delta, ZSTD(6)),
  schemaVersion UInt16 CODEC(LZ4),
  functionId String CODEC(ZSTD(6)),
  payload_json JSON CODEC(ZSTD(6)), -- redacted at ingest
  size_bytes UInt32 CODEC(LZ4)
)
ENGINE = MergeTree()
PARTITION BY toYYYYMMDD(ts)
ORDER BY (deviceId, ts, type, sessionId, seq)
TTL ts + INTERVAL 30 DAY DELETE  -- adjust per org in Phase 7
SETTINGS index_granularity = 8192;

-- Optional narrow tables for hot-path queries (example: PTY output)
CREATE TABLE IF NOT EXISTS cc_events_pty_output (
  ts DateTime64(3, 'UTC') CODEC(Delta, ZSTD(6)),
  deviceId String CODEC(ZSTD(6)),
  sessionId String CODEC(ZSTD(6)),
  seq UInt64 CODEC(Delta, ZSTD(6)),
  data String CODEC(ZSTD(6))
)
ENGINE = MergeTree()
PARTITION BY toYYYYMMDD(ts)
ORDER BY (deviceId, ts, sessionId, seq)
SETTINGS index_granularity = 8192;

-- Materialized view to project hot fields from raw into pty_output (optional, Phase 2+)
CREATE MATERIALIZED VIEW IF NOT EXISTS mv_pty_output
TO cc_events_pty_output AS
SELECT
  ts,
  deviceId,
  JSON_VALUE(payload_json, '$.id') AS sessionId,
  seq,
  JSON_VALUE(payload_json, '$.data') AS data
FROM cc_events_raw
WHERE type = 'pty-output';
```

Notes
- Partition by day on `ts`; order by `(deviceId, ts, type, sessionId, seq)` for efficient per-device/session scans.
- Redaction occurs before insert; `payload_json` stores already-redacted payloads.
- Retention via TTL (Phase 7 can move older partitions to object storage tiering if enabled).
- SSE fanout queries use `cc_events_raw` by time+device, with server-side buffering for burst smoothing.

## Backpressure and throttling
- Server backpressure: when ingest/DB is saturated, respond with `nack { reqSeq, rate_limited: true, retryAfterMs }` on control paths and slow SSE fanout; log a structured event.
- Agent adaptive throttling: leaky-bucket per device (default 256KB/s soft cap for telemetry; bursts up to 1MB over 5s). When `nack.rate_limited=true` or socket write backs up, agent increases batch window (up to 100ms) and reduces send rate.
- Queue near-full policy: disk-backed queue maintains a high-watermark (e.g., 64MB). When >90%, drop-oldest for non-critical telemetry; never drop `function-status`/terminal events.
- Health signals: agent emits `telemetry_health { queueBytes, droppedCount, throttled: true/false }` periodically (min interval 10s when degraded) to aid diagnostics.
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
- Backfill: reconnect replays from current in-memory queue; durable queue layout exists, but replay across agent restarts is still pending until persisted enqueue/send/ack lands.
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
- Limits: default max on-disk size 128 MB; drop-oldest when 90% full and backpressure active.
- Failure behavior: if decryption fails (corrupt), skip record, count, and continue; emit `nack { code: "QUEUE_CORRUPT" }` metric and raise alert threshold if consecutive.
- Privacy: secrets already redacted at source; encryption here defends device compromise at-rest; does not replace transport TLS.

## Test plan (Phase 2 Telemetry)

## Test plan (Phase 2 Telemetry)

- Backpressure/adaptive throttling
  - Induce server `nack { rate_limited: true, retryAfterMs }` and verify agent backs off, reduces batch size, and recovers.
  - Verify no data loss: post-recovery, all queued events are delivered in order.
- Durable queue replay
  - Kill agent mid-burst; on restart, verify disk queue replays without duplicates (keyed by `(deviceId, seq)`).
  - Corrupt one queued record; verify decrypt/verify fails, record is skipped with audit, rest proceed.
- Ordering and chunking
  - Interleave two PTY sessions; verify per-stream ordering preserved, cross-stream interleave allowed.
  - Send oversized events; verify gzip and `{chunkIndex, totalChunks}` chunking behaviors and server reassembly.
- Redaction parity
  - Inject payloads containing secrets; verify redaction at agent and server logs; ClickHouse sample queries show redacted fields only.
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
