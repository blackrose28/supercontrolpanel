# Security Hardening Plan — cc-server / admin-next

## Goal

Make the cc-server safe to expose to the public internet. The server currently runs
on plain HTTP with single-factor password auth, no multi-factor auth, global CORS,
and in-memory rate limiting.

## Threat Model

- **Threat actor:** Anyone on the internet who discovers the server endpoint.
- **Assets:** Admin dashboard (device control, PTY sessions, file transfers, logs),
  agent enrollment, telemetry data.
- **Attack surface:** HTTP endpoints (`/admin`, `/admin-next`, `/v1/*`),
  WebSocket endpoints.

## Architecture Context

The cc-server is a NestJS + Express backend with:
- Cookie-based **browser admin sessions** (SHA-256 hashed in SQLite)
- Bearer token-based **API auth** (env token + scoped DB tokens)
- CSRF protection for browser mutations
- In-memory token-bucket rate limiter
- CSP and security headers already in place
- No TLS (expects reverse proxy)
- No 2FA / MFA
- Admin password stored as plaintext in `.env`

## Implementation Phases

### Phase 1 — Foundation (must-do before internet exposure)

| #  | Task                                    | Risk | Effort |
|----|-----------------------------------------|------|--------|
| 1  | bcrypt/argon2 password hashing          | LOW  | Small  |
| 2  | Lock CORS to configurable origin        | LOW  | Small  |
| 3  | Persistent rate limiting (SQLite)       | LOW  | Medium |
| 4  | Account lockout + escalating delays     | LOW  | Small  |
| 5  | Auth failure timing delay (anti-timing) | LOW  | Small  |
| 6  | Request body size limits                | LOW  | Small  |

### Phase 2 — Multi-Factor Auth

| #  | Task                                    | Risk | Effort |
|----|-----------------------------------------|------|--------|
| 7  | TOTP 2FA setup (store secret in DB)     | MED  | Medium |
| 8  | TOTP enrollment UI in admin-next        | MED  | Medium |
| 9  | TOTP verification during login          | MED  | Small  |
| 10 | TOTP recovery codes                     | MED  | Medium |

### Phase 3 — Session Hardening

| #  | Task                                    | Risk | Effort |
|----|-----------------------------------------|------|--------|
| 11 | Session list / invalidation UI          | LOW  | Medium |
| 12 | "Kick all sessions" capability          | LOW  | Small  |
| 13 | Session fingerprinting (IP + UA record) | MED  | Medium |

### Phase 4 — Defense in Depth

| #  | Task                                    | Risk | Effort |
|----|-----------------------------------------|------|--------|
| 14 | IP allowlisting for admin routes        | LOW  | Small  |
| 15 | Audit log for admin actions             | LOW  | Medium |
| 16 | HSTS header (for reverse proxy)         | LOW  | Small  |
| 17 | Password complexity enforcement         | LOW  | Small  |
| 18 | WebSocket auth origin check             | LOW  | Small  |

## TDD Principle

Every task follows strict Red-Green-Refactor:
1. **RED**: Write a failing test that describes the desired behavior.
2. **GREEN**: Write minimal code to make the test pass.
3. **REFACTOR**: Clean up without changing behavior.

Tests live alongside source in `cc-server/src/**/*.spec.ts` (Vitest).

## File Impact Map

| Task | Files Modified | New Files |
|------|---------------|-----------|
| 1. Password hashing | `src/config/auth.ts` | `src/config/auth.spec.ts` |
| 2. CORS lock | `src/main.ts` | — |
| 3. Persistent rate limit | `src/rate-limit/*` | `src/rate-limit/rate-limit-persistent.spec.ts` |
| 4. Account lockout | `src/rate-limit/*`, `src/config/auth.ts` | — |
| 5. Auth timing delay | `src/config/auth.ts` | — |
| 6. Request body size | `src/main.ts` | — |
| 7-10. TOTP 2FA | `src/admin/*`, `admin-next/*` | `src/admin/totp.service.ts`, `src/admin/totp.service.spec.ts`, `admin-next/src/components/TotpSetup.tsx` |
| 11-12. Session mgmt | `src/admin/admin-session.service.ts`, `src/admin/admin.controller.ts` | — |
| 13. Session fingerprint | `src/admin/admin-session.service.ts`, `src/database/database.service.ts` | — |
| 14. IP allowlist | `src/main.ts`, `src/config/auth.ts` | `src/ip-allowlist.middleware.ts` |
| 15. Audit log | `src/admin/*` | `src/admin/admin-audit.service.ts` |
| 16. HSTS | `src/security-headers.ts` | — |
| 17. Password complexity | `src/config/auth.ts` | — |
| 18. WS auth origin | `src/agent/agent.gateway.ts` | — |

## Design Decisions

### Why TOTP over Passkeys/WebAuthn

- Passkeys require WebAuthn RP infrastructure, origin binding, and platform
  authenticator support. This is a single-user admin panel, not a consumer app.
- TOTP is simpler to implement, test, and deploy. It provides equivalent
  protection against credential theft (attacker needs password + TOTP code).
- TOTP works on any device with any authenticator app (Google Authenticator,
  Authy, 1Password, etc.).
- Implementation: `otplib` + QR code setup. Secret stored encrypted in SQLite.

### Why bcrypt over argon2

- bcrypt is sufficient for this threat model and has fewer native dependency
  issues across platforms. The password is already a strong random secret
  (not user-chosen), so the hashing function's memory-hardness matters less.

### Persistent Rate Limiting Strategy

- Store rate limit buckets in a new `rate_limit_buckets` SQLite table.
- Buckets keyed by `(clientId, actionType)`.
- Cleanup expired buckets on server start and periodically.
- Keeps the existing in-memory token-bucket algorithm but backs it with SQLite.
