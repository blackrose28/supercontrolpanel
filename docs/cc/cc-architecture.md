# Control Plane Architecture

## Goal
Centralize control and telemetry for multiple SuperControlPanel clients via a secure outbound control plane.

## Components
- **Agent (SuperControlPanel app)**: Maintains outbound encrypted WebSocket to CC, streams logs/events, and executes approved commands via existing PTY + function runtime.
- **CC Server**: Multi-tenant control plane exposing REST + WebSocket, persistence, authZ, audit, and live log streaming.
- **Admin UI**: Fleet view, per-device control, and live terminals/logs.

## Transport
- Outbound-only from agent: `wss://<cc-host>/agent` (NAT-friendly).
- Auth: mTLS or Bearer token with rotation; one-time enrollment.
- Reliability: pings, sequence ids, resumable control, batched telemetry.

## Security
- Default-deny, RBAC, approvals for sensitive actions.
- Redact secrets (align with MCP redaction).
- Full audit of control messages and outcomes.

## Agent Integration (Rust)
- New `src-tauri/src/cc/`: `connection.rs`, `protocol.rs`, `dispatcher.rs`, `telemetry.rs`.
- Wire in `src-tauri/src/main.rs`; forward Tauri events to CC.
- Config in `sync-local.json`: `ccEnabled`, `ccServerUrl`, `ccAuthMode`, `ccAgentToken?`, `ccDeviceId?`.

## Data Model (Server)
- `devices`, `commands`, `events` (logs), `sessions` (+ optional orgs/users/rbac).

## Observability
- OpenTelemetry, structured JSON logs, Sentry for server.

## Cross-Cutting Concerns
- Retention and Storage
    - Events: 30 days hot (Postgres), 180 days warm (object store/ClickHouse), configurable per org.
    - Commands and audits: 365 days.
    - PTY/log chunks capped at 16KB per frame; per-session max 50MB before truncation with notice.

- Rate Limits and Quotas
    - Per-device: 100 control msgs/min, 256KB/s telemetry soft cap with leaky-bucket.
    - Per-org burst/window: 10k msgs/5m and 1GB/day telemetry (configurable).
    - Backpressure signaled via `nack` with `rate_limited=true` and `retryAfterMs`.

- Disaster Recovery
    - Postgres PITR snapshots hourly; object-store versioning on.
    - Restore runbooks: RPO ≤ 15m, RTO ≤ 1h; periodic game days.

- Secrets Lifecycle
    - Agent token rotation: max TTL 30d; revocation propagates within 60s.
    - mTLS device certs: 1y validity, 60d early-rotation window; pubkey pinning.
    - Redaction at source: passwords, tokens, etc...
