# Phase 7 — Operations

## Scope
- Observability, retention, backups, on-call.

## Workstreams
- Metrics: OpenTelemetry, RED metrics for WS + queues.
- Logs: structured JSON, Sentry for exceptions.
- Retention: hot (7–14d), warm (30–90d), cold (optional).
- Backups: DB daily + WAL; restore drills.
- Dashboards + alerts: latency, error rate, backlog, reconnect rate.

## Acceptance Criteria
- Runbook covers common failures with steps.
- Load test at P95 expected fleet size passes SLOs.

## Risks
- Cost creep → sampling + compression.
