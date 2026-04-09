# Plan review checklist

Use this checklist to harden and complete the CC design before implementation. We’ll work items one by one and check them off.

## Device identity and enrollment
- [x] Define enrollment UX (bootstrap token, QR, one-time code) and device naming rules (see `docs/cc/cc-phase-1-bootstrap.md` → "Enrollment UX" and "Device identity and naming")
- [x] Prevent duplicate devices and concurrent connections for same device (evict vs reject policy) (see "Duplicate devices and concurrent connections")
- [x] Specify deviceId generation, persistence, and re-enrollment behavior (see "Device identity and naming")
- [x] Sequence/offset resume semantics across reconnects (how telemetry restarts or resumes) (see "Sequence/offset resume semantics")

## Agent offline durability
- [x] Decide on disk-backed queue for telemetry/acks (survive restarts, long offline) — see `docs/cc/cc-phase-2-telemetry.md` → "Deliverables" and "Agent queue encryption"
- [x] Backpressure strategy when queue is near full (drop-oldest, block, or degrade) — see `docs/cc/cc-phase-2-telemetry.md` → "Backpressure and throttling"
- [x] Size limits and encryption-at-rest for local queue — see `docs/cc/cc-phase-2-telemetry.md` → "Agent queue encryption"
## Protocol and versioning
- [x] Feature negotiation and protocol capability flags (hello/welcome exchange; see `docs/cc/cc-protocol.md` → "Versioning and Capabilities")
- [x] Backward/forward compatibility policy and deprecation window (see `docs/cc/cc-protocol.md` → "Versioning and Capabilities")
- [x] Idempotency keys for control commands and at-least-once vs exactly-once semantics (see "Idempotency and Delivery Semantics" in `docs/cc/cc-protocol.md`)
- [x] Document replay behavior on reconnect (see "Ordering and delivery" in `docs/cc/cc-phase-2-telemetry.md` and "Reconciliation on reconnect" in `docs/cc/cc-phase-3-control.md`)
## Security and authorization
- [x] Admin UI authN (SSO vs username/password), session mgmt, CSRF/CORS (see `docs/cc/cc-phase-6-security.md` → "Controls" → "AuthN (Admin UI)")
- [x] Tenant isolation at every API boundary (multi-org scoping) and PII handling (see `docs/cc/cc-phase-6-security.md` → "Controls" → "AuthZ (RBAC)" and Phase 2 redaction matrix)
- [x] Audit log integrity (WORM/append-only, hashing/attestation) and export (see `docs/cc/cc-phase-6-security.md` → "Audit and integrity")
- [x] Secret redaction in PTY/log streams; prompt/echo masking rules (see Phase 2 redaction matrix and `docs/cc/cc-phase-6-security.md` → "Controls" → "Secrets management")
## Control command lifecycle (Phase 3)
- [x] State machine: queued → dispatched → executing → succeeded/failed (deterministic reason codes) (see `docs/cc/cc-phase-3-control.md` → "Command lifecycle")
- [x] Timeouts, cancellation, retry with jitter; max concurrency and resource locks (see `docs/cc/cc-phase-3-control.md` → "Timeouts, cancellation, and retries" + "Agent")
- [x] Idempotent replays across reconnects; reconciliation on agent resume (see `docs/cc/cc-phase-3-control.md` → "Idempotency and exactly-once" + "Reconciliation on reconnect")
- [x] Define per-command schema and validation; reject invalid/unsafe params with explicit errors (see `docs/cc/cc-phase-3-control.md` → "Validation and safety" and `docs/cc/cc-protocol.md` → "API design and schema hygiene")
## Telemetry at scale (Phase 2)
- [x] Server backpressure (nack + retryAfterMs) and agent adaptive throttling (see `docs/cc/cc-phase-2-telemetry.md` → "Backpressure and throttling")
- [x] Event schema versioning and per-type size limits; compression policy (gzip/chunking) (see `docs/cc/cc-phase-2-telemetry.md` → "Event schema and versioning" and "Compression and size limits")
- [x] Ordering guarantees per `(deviceId, sessionId, type)`; interleaving across streams (see `docs/cc/cc-phase-2-telemetry.md` → "Ordering and delivery")
- [x] Inventory publishing (`serversListHash`, cadence and diff/apply strategy) (see `docs/cc/cc-phase-2-telemetry.md` → "Inventory publishing")
## Interactive PTY hardening (Phase 4)
- [x] Input encoding, bracketed paste, mouse reporting; Windows ConPTY vs Unix behavior (see `docs/cc/cc-phase-4-interactive-pty.md` → "PTY hardening")
- [x] Scrollback replay size and truncation; optional session recording + retention policy (see `docs/cc/cc-phase-4-interactive-pty.md` → "PTY hardening" and "CC / Admin UI")
- [x] Operator RBAC for PTY access; privacy banner and consent handling (see `docs/cc/cc-phase-4-interactive-pty.md` → "PTY hardening", "CC / Admin UI")

## File operations (Phase 5)
- [x] Resume/retry for large transfers; checksum verification; atomic temp-then-rename (see `docs/cc/cc-phase-5-file-ops.md` → "Transfer reliability (resume/retry)" and "Integrity and atomicity")
- [x] Overwrite confirmation policy; symlink handling; sparse files; Windows path rules (see `docs/cc/cc-phase-5-file-ops.md` → "Path rules and safety")
- [x] Rate limits per transfer and per-tenant quotas (see `docs/cc/cc-phase-5-file-ops.md` → "Quotas and rate limits")
- [x] Boundary-backed SCP: verify active tunnel and surface actionable errors (see `docs/cc/cc-phase-5-file-ops.md` → "Boundary tunnel precheck")

## Boundary/OpenVPN specifics
- [x] Tunnel state reconciliation and auto-repair flows (see `docs/cc/cc-architecture.md` → "Boundary/OpenVPN specifics")
- [x] Credential/token refresh and failure-mode UX in Admin UI (see `docs/cc/cc-architecture.md` → "Boundary/OpenVPN specifics")

## Operations, SLOs, and testing (Phase 7)
- [x] Concrete SLOs (WS latency, connect success, ingest throughput, SSE fanout) (see `docs/cc/cc-phase-7-ops.md` → "SLOs (initial targets)")
- [x] Load/soak and chaos testing (network flap, packet loss, server restarts) (see `docs/cc/cc-phase-7-ops.md` → "Load & chaos testing")
- [x] Backup/restore verification (DB + object store); key/cert backup strategy (see `docs/cc/cc-phase-7-ops.md` → "Backups & restore")

## Observability and tracing
- [x] End-to-end trace correlation (command/request id through Admin → CC → Agent → PTY) (see `docs/cc/cc-phase-7-ops.md` → "Observability")
- [x] Redaction filters in logs/traces; sampling strategy under load (see `docs/cc/cc-phase-7-ops.md` → "Observability" and Phase 2 redaction matrix)

## Admin/UI flows
- [x] Approvals UX (who can approve, expiry, escalation) (see `docs/cc/cc-phase-6-security.md` → "Controls" → "Approvals" and `docs/cc/cc-architecture.md` → "Admin/UI flows")
- [x] RBAC modeling in UI (scopes, roles, grants) and fleet-scale search/filter (see `docs/cc/cc-phase-6-security.md` → "Controls" → "AuthZ (RBAC)" and `docs/cc/cc-architecture.md` → "Admin/UI flows")
- [x] Change review displays (e.g., diff for risky file ops) (see `docs/cc/cc-architecture.md` → "Admin/UI flows")

## API design and schema hygiene
- [x] WS payload JSON Schemas documented in `docs/cc/cc-protocol.md` (see "API design and schema hygiene")
- [x] OpenAPI for REST endpoints; SDK generation (see `docs/cc/cc-protocol.md` → "OpenAPI and SDK plan")
- [x] API versioning and deprecation policy (see `docs/cc/cc-protocol.md` → "REST API versioning and deprecation")

## Deployment
- [x] Containerization; database migrations/backup strategy for the chosen SQLite-based server deployment (see `docs/cc/cc-phase-7-ops.md` → "Deployment")
- [x] Blue/green or canary rollout; configuration management (12-factor) (see `docs/cc/cc-phase-7-ops.md` → "Deployment")
- [x] Server secrets management (Vault/KMS); rotation runbooks (see `docs/cc/cc-phase-7-ops.md` → "Deployment" → "Server secrets management")

## Compliance and data lifecycle
- [x] Data retention per type, legal hold, tenant overrides (see `docs/cc/cc-architecture.md` → "Compliance and data lifecycle")
- [x] Right-to-erasure handling and audit redaction rules (see `docs/cc/cc-architecture.md` → "Compliance and data lifecycle")

## Cross-file doc fixes
- [x] Fix truncated Acceptance Criteria — all phase docs now have complete Acceptance Criteria sections (Phases 1-7)
- [x] Ensure command lifecycle docs exist and cover lifecycle, idempotency, reconciliation (see `docs/cc/cc-phase-3-control.md`)
- [x] Add a normative mapping table between protocol names and app contracts (see `docs/cc/cc-protocol.md` → "API design and schema hygiene" for schemas; cross-references added throughout phase docs)

## Per-phase quick adds
- [x] Phase 1: enrollment flow, duplicate-connection policy, disk queue decision (see `docs/cc/cc-phase-1-bootstrap.md`)
- [x] Phase 2: event schema/versioning, compression toggle, ordering, inventory publishing (see `docs/cc/cc-phase-2-telemetry.md`)
- [x] Phase 3: command idempotency, timeouts/cancel/retry, reconciliation, RBAC gates (see `docs/cc/cc-phase-3-control.md`)
- [x] Phase 4: ConPTY/encoding details, scrollback/recording policies, operator RBAC (see `docs/cc/cc-phase-4-interactive-pty.md`)
- [x] Phase 5: resume/retry + checksum/atomic rename, overwrite/symlink rules (see `docs/cc/cc-phase-5-file-ops.md`)
- [x] Phase 6: Admin authN (SSO), audit immutability, approvals UX spec (see `docs/cc/cc-phase-6-security.md`)
- [x] Phase 7: SLO targets, chaos tests, restore drills (see `docs/cc/cc-phase-7-ops.md`)
