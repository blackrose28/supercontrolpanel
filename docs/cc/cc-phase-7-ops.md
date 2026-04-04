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
- DB daily full + WAL; object store versioning; periodic restore drills (quarterly).
- Runbooks: RPO ≤ 15m, RTO ≤ 1h; tested against staging copies.

## Load & chaos testing
- Load targets: at expected fleet size, P95 SLOs pass under 2× normal traffic.
- Chaos: network flap, packet loss, server restarts, DB failover; verify graceful degradation and recovery.

## Acceptance Criteria
- Dashboards cover all SLOs; alerts wired with actionable runbooks.
- Backup/restore validated with sampled data; drill reports archived.
- Load+chaos tests pass with SLOs met or with documented mitigations.

## Risks
- Cost creep → sampling, compression, tiered storage.
- Alert fatigue → tune thresholds, add SLO burn-rate alerts.
