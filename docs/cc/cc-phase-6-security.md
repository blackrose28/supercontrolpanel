# Phase 6 — Security Hardening

## Scope
- mTLS or token rotation; org+RBAC; approvals.

## Controls
### AuthN (Admin UI)
- SSO: OIDC (preferred) or SAML; support multiple IdPs per org; fallback to local accounts (bcrypt, min 12 chars, optional MFA via TOTP).
- Session management: JWT with 1h access token, 7d refresh token; rotate on use; revoke on logout or admin action.
- MFA: TOTP (RFC 6238) enforced per-org policy; backup codes (10, single-use) generated on enrollment.
- CSRF/CORS: SameSite=Strict cookies; CORS allowlist per org; CSRF token on state-changing requests.

### AuthZ (RBAC)
- Roles: `admin`, `operator`, `viewer` (default); custom roles with granular permissions.
- Permissions: `device:read`, `device:control`, `session:attach`, `session:record`, `audit:export`, `approval:grant`, etc.
- Resource scoping: by org, device tags, session type; least-privilege defaults (viewer sees only assigned devices).
- Enforcement: every API/WS command checks actor permissions; deny emits `nack { status: "unauthorized" }` and audits attempt.

### Approvals
- Policy: define per-org which actions require approval (e.g., `scp_upload`, `start_server_ssh`, `delete_remote_browser_path`).
- Workflow: requester submits; approval queued; min N approvers (role-based); expiry (default 1h); escalation on SLA breach.
- Context: show diff/target (file path, tunnel dest, command params redacted); approver sees risk signals (device status, recent failures).
- Revocation: if risk signals change (device offline, tunnel down), approval auto-revoked and requester notified.

### Network security
- IP allowlist/deny per org; rate limits (login: 5/min, commands: 100/min per device); anomaly detection (login spikes, command bursts) triggers alerts.

## SLOs (Phase 6 Security)

### Test plan (Phase 6 Security)

## Test plan (Phase 6 Security)

- AuthN/SSO
  - OIDC login flow: verify token exchange, session creation, MFA enforcement when enabled.
  - Local account: bcrypt validation, password strength enforcement, MFA TOTP validation.
  - Session expiry: access token expires at 1h; refresh token rotates on use; logout revokes both.
  - CSRF: verify state-changing requests without CSRF token are rejected with `403`.
- AuthZ/RBAC
  - Role enforcement: `viewer` cannot execute control commands; `operator` can attach PTY; `admin` can grant approvals.
  - Resource scoping: user with device tag filter sees only assigned devices; unauthorized device access rejected.
  - Permission checks: every API/WS command validates actor permissions; unauthorized attempts audited.
- Approvals workflow
  - Submit sensitive action (e.g., `scp_upload`); verify queued, min N approvers required, expiry enforced.
  - Approver sees context (file path, redacted params); approval granted; action proceeds.
  - Expiry: approval expires after 1h; action rejected with `approval_expired`.
  - Revocation: device goes offline; approval auto-revoked; requester notified.
- Token rotation
  - Agent token rotation: staggered refresh with ±10% jitter; verify no disconnect storms; pre-expiry warnings at 7d.
  - Clock skew: tolerate ±2 min skew; verify grace window prevents false expiry.
- Audit integrity
  - Append-only log: verify hash chain; tamper attempt detected; export verifies chain.
  - Audit export: NDJSON/CSV format; includes actor, device, command, outcome, timestamps; redacted params.
- Secrets management
  - Agent tokens: stored in OS keyring; never logged plaintext; rotation enforced at 30d TTL.
  - Admin secrets: KMS-backed; rotation runbooks tested; plaintext never in logs/telemetry.
- Network security
  - IP allowlist: verify requests from non-allowlisted IPs rejected.
  - Rate limits: exceed login rate (5/min); verify `429` with `Retry-After`.
  - Anomaly detection: simulate login spike; verify alert triggered.
- Transport security
  - mTLS: verify agent WS handshake with valid cert; invalid cert rejected.
  - HTTPS/HSTS: verify Admin UI enforces HTTPS; HSTS header present; CSP prevents XSS.


## SLOs (Phase 6 Security)

- Token rotation: 100% success rate without disconnect storms; staggered refresh with ±10% jitter
- Approval latency: P95 ≤ 5 s from submission to queue visibility; expiry enforced within 1 s of threshold
- Audit integrity: 100% tamper-evidence via hash chain; export verifies chain with 0 false positives
- RBAC enforcement: 100% unauthorized attempts blocked and audited; P95 authz check latency ≤ 10 ms
- MFA enrollment: 100% TOTP validation success for valid codes; backup codes single-use enforced


### Secrets management
- Agent tokens: stored in OS keyring; max TTL 30d; rotation staggered (±10% jitter) to avoid storms; pre-expiry warnings at 7d.
- Admin secrets: KMS-backed (AWS KMS, GCP KMS, Vault); rotation runbooks; never log plaintext.
- Redaction: apply at source (agent) and ingest (server); audit redaction rules per event type (see Phase 2 Redaction Matrix).

### Transport security
- Agent WS: mTLS (preferred) or Bearer token with TLS 1.3; pubkey pinning optional (org-configurable).
- Admin UI: HTTPS only; HSTS enforced; CSP headers to prevent XSS.

## Audit and integrity
- Append-only audit log with tamper-evidence (hash chain or periodic signed checkpoints).
- Export audits per org (NDJSON/CSV); WORM-compatible storage option.
- Include actor, target device/session, command, parameters (redacted), outcome, reason codes, timestamps.

## Approvals UX (Admin)
- Show pending approvals with diff/context (e.g., file overwrite target, tunnel dest).
- Expiry and escalation (notify on SLA breach); revoke approval if risk signals change.

## Acceptance Criteria
- Rotating token works without disconnect storms (staggered refresh; tolerate ±2 min skew).
- Approval flow blocks sensitive ops until consent; audited end-to-end.
- All actions audited with actor + device; audit export verifies hash chain.

## Risks
- Clock skew on token expiry → allow grace window; show pre-expiry warnings.
- Enrollment UX friction → bootstrap token + QR fallback.
