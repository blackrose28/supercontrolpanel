# Plan review checklist

Use this checklist to harden and complete the CC design before implementation. We’ll work items one by one and check them off.

## Device identity and enrollment
- [ ] Define enrollment UX (bootstrap token, QR, one-time code) and device naming rules
- [ ] Prevent duplicate devices and concurrent connections for same  (evict vs reject policy)
- [ ] Specify  generation, persistence, and re-enrollment behavior
- [ ] Sequence/offset resume semantics across reconnects (how  restarts or resumes)

## Agent offline durability
- [ ] Decide on disk-backed queue for telemetry/acks (survive restarts, long offline)
- [ ] Backpressure strategy when queue is near full (drop-oldest, block, or degrade)
- [ ] Size limits and encryption-at-rest for local queue

## Protocol and versioning
- [ ] Feature negotiation and protocol capability flags (/ exchange)
- [ ] Backward/forward compatibility policy and deprecation window
- [ ] Idempotency keys for control commands and at-least-once vs exactly-once semantics
- [ ] Document replay behavior on reconnect

## Security and authorization
- [ ] Admin UI authN (SSO vs username/password), session mgmt, CSRF/CORS
- [ ] Tenant isolation at every API boundary (multi-org scoping) and PII handling
- [ ] Audit log integrity (WORM/append-only, hashing/attestation) and export
- [ ] Secret redaction in PTY/log streams; prompt/echo masking rules

## Control command lifecycle (Phase 3)
- [ ] State machine: queued → dispatched → executing → succeeded/failed (deterministic reason codes)
- [ ] Timeouts, cancellation, retry with jitter; max concurrency and resource locks
- [ ] Idempotent replays across reconnects; reconciliation on agent resume
- [ ] Define per-command schema and validation; reject invalid/unsafe params with explicit errors

## Telemetry at scale (Phase 2)
- [ ] Server backpressure ( with ) and agent adaptive throttling
- [ ] Event schema versioning and per-type size limits; compression policy (e.g., gzip frame)
- [ ] Ordering guarantees per ; interleaving expectations across streams
- [ ] Inventory publishing ( hash): cadence, diff/apply strategy, acceptance criteria

## Interactive PTY hardening (Phase 4)
- [ ] Input encoding, bracketed paste, mouse reporting; Windows ConPTY vs Unix behavior
- [ ] Scrollback replay size and truncation; optional session recording + retention policy
- [ ] Operator RBAC for PTY access; privacy banner and consent handling

## File operations (Phase 5)
- [ ] Resume/retry for large transfers; checksum verification; atomic temp-then-rename
- [ ] Overwrite confirmation policy; symlink handling; sparse files; Windows path rules
- [ ] Rate limits per transfer and per-tenant quotas
- [ ] Boundary-backed SCP: verify active tunnel and surface actionable errors

## Boundary/OpenVPN specifics
- [ ] Tunnel state reconciliation and auto-repair flows
- [ ] Credential/token refresh and failure-mode UX in Admin UI

## Operations, SLOs, and testing (Phase 7)
- [ ] Concrete SLOs (WS latency, connect success, ingest throughput, SSE fanout)
- [ ] Load/soak and chaos testing (network flap, packet loss, server restarts)
- [ ] Backup/restore verification (DB + object store); key/cert backup strategy

## Observability and tracing
- [ ] End-to-end trace correlation (command/request id through Admin → CC → Agent → PTY)
- [ ] Redaction filters in logs/traces; sampling strategy under load

## Admin/UI flows
- [ ] Approvals UX (who can approve, expiry, escalation)
- [ ] RBAC modeling in UI (scopes, roles, grants) and fleet-scale search/filter
- [ ] Change review displays (e.g., diff for risky file ops)

## API design and schema hygiene
- [ ] OpenAPI/JSON Schema for REST and WS payloads; SDK generation
- [ ] API versioning and deprecation policy

## Deployment
- [ ] Containerization; database migrations (Timescale/ClickHouse strategy)
- [ ] Blue/green or canary rollout; configuration management (12-factor)
- [ ] Server secrets management (Vault/KMS); rotation runbooks

## Compliance and data lifecycle
- [ ] Data retention per type, legal hold, tenant overrides
- [ ] Right-to-erasure handling and audit redaction rules

## Cross-file doc fixes
- [ ] Fix truncated Acceptance Criteria in 
- [ ] Ensure  exists and covers lifecycle, idempotency, reconciliation
- [ ] Add a normative mapping table once between protocol names and app contracts; cross-link in all relevant docs

## Per-phase quick adds
- [ ] Phase 1: enrollment flow, duplicate-connection policy, disk queue decision
- [ ] Phase 2: event schema/versioning, compression toggle, ordering, inventory publishing
- [ ] Phase 3: command idempotency, timeouts/cancel/retry, reconciliation, RBAC gates
- [ ] Phase 4: ConPTY/encoding details, scrollback/recording policies, operator RBAC
- [ ] Phase 5: resume/retry + checksum/atomic rename, overwrite/symlink rules
- [ ] Phase 6: Admin authN (SSO), audit immutability, approvals UX spec
- [ ] Phase 7: SLO targets, chaos tests, restore drills
