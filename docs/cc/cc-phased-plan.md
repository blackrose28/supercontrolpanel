# CC Server – Phased Plan

## Phase 1 — Bootstrap
- Server skeleton (WS `/agent`, basic auth), in-memory registry.
- Agent connects, sends `heartbeat` + `function-status`.
- Admin UI: device list + live status.

## Phase 2 — Telemetry
- Stream `pty-output`, `openvpn-log`.
- Persist logs (Postgres + Timescale/ClickHouse). SSE for UI stream.

## Phase 3 — Core Control
- Implement `start/stop_*` for OpenVPN/Boundary/tunnel/ssh.
- Add RBAC and audit trail.

## Phase 4 — Interactive PTY
- Remote PTY control; admin live terminal view.

## Phase 5 — File Ops
- SCP upload/download + remote browser controls; progress via PTY or progress events.

## Phase 6 — Security Hardening
- mTLS, token rotation, org scoping, approvals, rate limits, IP allowlist.

## Phase 7 — Operations
- Metrics, dashboards, backups, log retention, alerts.

## Tech Choices
- Server: Node (NestJS) or Python (FastAPI), Postgres, Redis fanout.
- Admin UI: Vite + React (or server-rendered + htmx).
- Observability: OpenTelemetry + JSON logs + Sentry.


---
See detailed Phase 1: [cc-phase-1-bootstrap.md](cc-phase-1-bootstrap.md)

## Detailed Phase Docs
- [Phase 1 — Bootstrap](cc-phase-1-bootstrap.md)
- [Phase 2 — Telemetry](cc-phase-2-telemetry.md)
- [Phase 3 — Control](cc-phase-3-control.md)
- [Phase 4 — Interactive PTY](cc-phase-4-interactive-pty.md)
- [Phase 5 — File Operations](cc-phase-5-file-ops.md)
- [Phase 6 — Security Hardening](cc-phase-6-security.md)
- [Phase 7 — Operations](cc-phase-7-ops.md)
