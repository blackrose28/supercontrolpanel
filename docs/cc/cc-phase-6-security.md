# Phase 6 — Security Hardening

?? Scope
- mTLS or token rotation; org+RBAC; approvals.

## Controls
- AuthN (Admin UI): support SSO (OIDC/SAML) and local accounts (optional); enforce MFA when enabled.
- AuthZ (RBAC): roles ’ permissions; resource scoping by org, device, session; least-privilege defaults.
- Approvals: sensitive actions require approval according to policy (min approvers, role, expiry). Queue until approved.
- Network: IP allowlist/deny, rate limits, anomaly detection (login spikes, command bursts).
- Secrets: redact in logs/telemetry; store with OS keyring/KMS; rotate on schedule (agent tokens ©30d TTL).
- Transport: mTLS for agent WS or Bearer with rotation; pubkey pinning option.

## Audit and integrity
- Append-only audit log with tamper-evidence (hash chain or periodic signed checkpoints).
- Export audits per org (NDJSON/CSV); WORM-compatible storage option.
- Include actor, target device/session, command, parameters (redacted), outcome, reason codes, timestamps.

## Approvals UX (Admin)
- Show pending approvals with diff/context (e.g., file overwrite target, tunnel dest).
- Expiry and escalation (notify on SLA breach); revoke approval if risk signals change.

## Acceptance Criteria
- Rotating token works without disconnect storms (staggered refresh; tolerate â2 min skew).
- Approval flow blocks sensitive ops until consent; audited end-to-end.
- All actions audited with actor + device; audit export verifies hash chain.

## Risks
- Clock skew on token expiry —  allow grace window; show pre-expiry warnings.
- Enrollment UX friction — bootstrap token + QR fallback.
�