# Phase 7 — Operations

## Scope
- Observability, SLOs, retention, backups, on-call, chaos/load.

## SLOs (initial targets)
- WS connect success: P50 < 3s, P95 < 10s.
- Telemetry end-to-end latency (agent → Admin SSE): P50 < 250ms, P95 < 800ms.
- Command dispatch to start execution: P50 < 500ms, P95 < 2s.
- Terminal keystroke echo: P50 < 120ms, P95 < 300ms over WAN.
- Uptime: 99.9% monthly for control/ingest APIs.

## Observability
- Metrics: OpenTelemetry + RED metrics (rate, errors, duration) for WS, queues, SSE.
- Tracing: distributed trace with correlation ids (commandId/reqId) across Admin → CC → Agent → PTY.
- Logs: structured JSON; PII/secrets redacted; sampling under burst.
- Dashboards: latency, error rate, backlog, reconnect rate, SSE fanout size; per-tenant views.

## Retention & storage
- Hot logs: 7–14d; Warm: 30–90d; Cold (optional object store).
- Configurable per-org overrides; deletion jobs with safeguards.

## Backups & restore
### Backup strategy
- SQLite: scheduled hot backups of the database file plus periodic validated restore snapshots; retention 30d; store encrypted backups in S3-compatible object storage with versioning enabled.
- If a future relational sidecar is introduced for scale, apply the same backup/PITR discipline there as needed.
- Object store (recordings, exports): versioning on; lifecycle policy moves to Glacier after 90d.
- Secrets (KMS keys, agent tokens): backup via KMS native replication; rotation runbooks tested quarterly.

### Restore runbooks
- RPO target: ≤ 15 min (WAL replay from last checkpoint).
- RTO target: ≤ 1 h (restore + validation + cutover).
- Runbook steps:
  1. Identify restore point (timestamp or WAL position).
  2. Restore the SQLite database snapshot to a clean host or volume.
  3. Apply the latest backup increment/export set if the backup workflow uses them.
  4. Validate data integrity (row counts, sample queries, audit chain).
  5. Cutover DNS/load balancer; monitor for anomalies.
- Drill cadence: quarterly; drill reports archived with lessons learned.

### Disaster recovery
- Multi-region replication: async replication to secondary region (optional, Phase 7+).
- Failover: manual cutover to secondary; RTO ≤ 2h including validation.
- Game days: simulate full region failure; validate runbooks and RTO/RPO targets.
## Load & chaos testing
### Load testing targets
- Fleet size: 1,000 devices (Phase 7 baseline); 10,000 devices (future scale).
- Traffic profile: 100 events/s aggregate telemetry; 10 commands/s; 50 concurrent PTY sessions.
- Load test duration: 1h sustained; 15min burst at 2× normal traffic.
- SLO validation: P95 latency within targets under 2× load; error rate < 0.1%.

### Chaos scenarios
- Network flap: disconnect 10% of agents for 30s; verify reconnect with backoff and replay without duplicates.
- Packet loss: inject 5% packet loss on WS path; verify graceful degradation and recovery within SLOs.
- Server restart: rolling restart of CC instances; verify zero data loss and <5s reconnect per agent.
- DB recovery: restore the SQLite database on a replacement host/volume; verify writes resume within 30s and queries within 10s.
- Disk full: fill agent queue disk to 95%; verify drop-oldest policy and recovery after space freed.
- Clock skew: introduce ±5 min skew on agent; verify token grace window prevents false expiry.

## Deployment

### Test plan (Phase 7 Operations)

## Test plan (Phase 7 Operations)

- SLO monitoring
  - Verify dashboards display all SLOs (WS connect, telemetry latency, command dispatch, PTY echo, uptime).
  - Trigger SLO breach (e.g., inject latency); verify alert fires with actionable runbook link.
- Backup/restore drills
  - Execute quarterly restore drill: restore from snapshot, replay WAL, validate data integrity, cutover.
  - Verify RPO ≤ 15 min, RTO ≤ 1 h; document lessons learned.
- Load testing
  - Simulate 1,000 devices at 2× normal traffic for 1h; verify P95 SLOs met; identify bottlenecks.
  - Burst test: 2× traffic for 15 min; verify graceful degradation and recovery.
- Chaos testing
  - Network flap: disconnect 10% agents; verify reconnect and replay without duplicates.
  - Server restart: rolling restart; verify zero data loss and <5s reconnect.
  - DB failover: trigger failover; verify writes resume within 30s.
  - Disk full: fill agent queue to 95%; verify drop-oldest and recovery.
  - Clock skew: ±5 min skew; verify token grace window.
- Deployment validation
  - Blue/green: deploy new version, cutover, validate SLOs, rollback via DNS flip.
  - Canary: route 5% traffic, monitor for 1h, ramp to 100% over 24h.
  - Migration: apply schema migration in staging; verify rollback script; apply in production with lock.
- Observability
  - Verify distributed traces with correlation ids (commandId/reqId) across Admin → CC → Agent → PTY.
  - Verify structured logs with PII/secrets redacted; sampling under burst.
  - Verify per-tenant dashboards show latency, error rate, backlog, reconnect rate, SSE fanout size.
- Secrets rotation
  - Rotate agent token; verify staggered refresh with ±10% jitter; no disconnect storms.
  - Rotate DB password via KMS; verify zero downtime.
  - Rotate admin session keys; verify active sessions remain valid until expiry.

## Deployment

### Containerization
- CC Server: Docker image with NestJS + SQLite runtime support; multi-stage build; base image: node:20-alpine.
- Agent: bundled in Tauri app; no separate container (desktop app).
- Admin UI: static build served via Nginx or CDN; CSP headers enforced.

### Database migrations
- SQLite: schema versioning via migration scripts; applied on startup with locking to prevent concurrent runs.
- Any future auxiliary database should follow the same versioned migration discipline, with rollback tested in staging.
- Migration strategy: additive-only changes preferred; breaking changes require blue/green deployment.

### Rollout strategy
- Blue/green: deploy new version alongside old; cutover DNS/load balancer after validation; rollback via DNS flip.
- Canary: route 5% traffic to new version; monitor SLOs for 1h; gradual ramp to 100% over 24h.
- Rolling restart: for minor updates; restart instances one at a time with health checks; zero downtime.

### Configuration management
- 12-factor: env vars for secrets (KMS-backed); config files for non-sensitive settings.
- Secrets: stored in KMS (AWS Secrets Manager, GCP Secret Manager, Vault); injected at runtime; never in source control.
- Rotation: automated rotation for DB passwords, API keys; agent tokens rotated via CC API.

### Server secrets management
- KMS integration: AWS KMS, GCP KMS, or Vault; keys rotated quarterly; rotation runbooks tested.
- Agent tokens: generated server-side; stored in agent OS keyring; max TTL 30d; rotation staggered.
- Admin secrets: session keys, CSRF tokens, OAuth client secrets; rotated on schedule; never logged.

### Acceptance
- All chaos scenarios pass with graceful degradation; SLOs met or documented mitigations in place.
- Load tests pass at 2× traffic; bottlenecks identified and mitigated before production.
- Runbooks updated with lessons learned from chaos/load tests.
## Acceptance Criteria
- Dashboards cover all SLOs; alerts wired with actionable runbooks.
- Backup/restore validated with sampled data; drill reports archived.
- Load+chaos tests pass with SLOs met or with documented mitigations.

## Risks
- Cost creep → sampling, compression, tiered storage.
- Alert fatigue → tune thresholds, add SLO burn-rate alerts.
