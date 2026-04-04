# Phase 6 — Security Hardening

## Scope
- mTLS or token rotation; org+RBAC; approvals.

## Controls
- Auth: device certificates (mTLS) or short-lived tokens with refresh.
- RBAC: roles → permissions; per-org tenancy.
- Sensitive-action approvals: queue until local user approves.
- Network: IP allowlist, rate limits, anomaly detection.

## Acceptance Criteria
- Rotating token works without disconnect storms.
- Approval flow blocks sensitive ops until consent.
- All actions audited with actor + device.

## Risks
- Clock skew on token expiry → tolerate ±2 min.
- Device enrollment UX.
