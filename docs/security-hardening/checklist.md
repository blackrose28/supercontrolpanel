# Security Hardening Checklist — cc-server / admin-next

Status: 🔴 = not started, 🟡 = in progress, 🟢 = completed

---

## Phase 1 — Foundation

- [ ] 🔴 **1.1** — Password hashing with bcrypt
  - [ ] 🔴 1.1.1 Write spec: `src/config/auth.spec.ts` — tests for hash/verify, plaintext migration
  - [ ] 🔴 1.1.2 Add bcrypt dep to package.json
  - [ ] 🔴 1.1.3 Implement hashing in `src/config/auth.ts` — `hashPassword()`, `verifyPassword()`
  - [ ] 🔴 1.1.4 Update `validateAuthConfig()` to hash at startup
  - [ ] 🔴 1.1.5 Update `admin.controller.ts` login to use `verifyPassword()`
  - [ ] 🔴 1.1.6 Run all tests — ensure no regressions

- [ ] 🔴 **1.2** — Lock CORS to configurable origin
  - [ ] 🔴 1.2.1 Write spec: test CORS configuration with allowed/disallowed origins
  - [ ] 🔴 1.2.2 Add `CC_CORS_ORIGIN` env var (default: `http://localhost:3000`)
  - [ ] 🔴 1.2.3 Replace `app.enableCors()` with explicit cors config in `src/main.ts`
  - [ ] 🔴 1.2.4 Run all tests

- [ ] 🔴 **1.3** — Persistent rate limiting (SQLite-backed)
  - [ ] 🔴 1.3.1 Write spec: `src/rate-limit/rate-limit.service.spec.ts` — test persist/restore
  - [ ] 🔴 1.3.2 Add `rate_limit_buckets` table to `database.service.ts`
  - [ ] 🔴 1.3.3 Add async persist layer to `rate-limit.service.ts`
  - [ ] 🔴 1.3.4 Add periodic cleanup for expired buckets
  - [ ] 🔴 1.3.5 Run all tests

- [ ] 🔴 **1.4** — Account lockout + escalating delays
  - [ ] 🔴 1.4.1 Write spec: test lockout after N failed attempts
  - [ ] 🔴 1.4.2 Add lockout config env vars (`CC_LOGIN_MAX_ATTEMPTS`, `CC_LOGIN_LOCKOUT_MS`)
  - [ ] 🔴 1.4.3 Track failed login attempts per session/IP in `admin-auth.ts`
  - [ ] 🔴 1.4.4 Implement escalating delays: 1s -> 2s -> 4s -> 8s -> 16s -> locked
  - [ ] 🔴 1.4.5 Run all tests

- [ ] 🔴 **1.5** — Auth failure timing delay
  - [ ] 🔴 1.5.1 Write spec: verify auth failure path includes random delay
  - [ ] 🔴 1.5.2 Add random delay (50-200ms) on auth failure in `admin-auth.ts`
  - [ ] 🔴 1.5.3 Run all tests

- [ ] 🔴 **1.6** — Request body size limits
  - [ ] 🔴 1.6.1 Add `express.json({ limit: '1mb' })` in `src/main.ts`
  - [ ] 🔴 1.6.2 Confirm no endpoints need larger limits
  - [ ] 🔴 1.6.3 Run all tests

---

## Phase 2 — Multi-Factor Auth (TOTP)

- [ ] 🔴 **2.1** — TOTP service + secret storage
  - [ ] 🔴 2.1.1 Add `otplib` and `qrcode` deps to package.json
  - [ ] 🔴 2.1.2 Write spec: `src/admin/totp.service.spec.ts`
  - [ ] 🔴 2.1.3 Implement `src/admin/totp.service.ts` — generate secret, verify code, encrypt secret
  - [ ] 🔴 2.1.4 Add `admin_totp` table to `database.service.ts`
  - [ ] 🔴 2.1.5 Store TOTP secret encrypted with server-derived key (e.g., SHA-256 of bootstrap token)
  - [ ] 🔴 2.1.6 Run all tests

- [ ] 🔴 **2.2** — TOTP API endpoints
  - [ ] 🔴 2.2.1 Write spec: `src/admin/admin.controller.spec.ts` — add totp tests
  - [ ] 🔴 2.2.2 `POST /admin/totp/setup` — generate TOTP secret, return QR code data URL
  - [ ] 🔴 2.2.3 `POST /admin/totp/verify-setup` — verify TOTP code, enable 2FA
  - [ ] 🔴 2.2.4 `POST /admin/totp/disable` — disable 2FA (requires current TOTP code)
  - [ ] 🔴 2.2.5 `GET /admin/totp/status` — return whether 2FA is enabled
  - [ ] 🔴 2.2.6 Run all tests

- [ ] 🔴 **2.3** — TOTP during login flow
  - [ ] 🔴 2.3.1 Write spec: test 2FA login flow (password -> TOTP -> session)
  - [ ] 🔴 2.3.2 Modify login endpoint: if 2FA enabled, validate password first, then require TOTP
  - [ ] 🔴 2.3.3 Store intermediate "password-ok" state in a short-lived cookie (2 min TTL)
  - [ ] 🔴 2.3.4 `POST /admin/login/totp` — verify TOTP code + intermediate cookie -> create session
  - [ ] 🔴 2.3.5 Run all tests

- [ ] 🔴 **2.4** — TOTP UI in admin-next
  - [ ] 🔴 2.4.1 Write component tests for `TotpSetup.tsx`
  - [ ] 🔴 2.4.2 Create TOTP setup component with QR code display + code input
  - [ ] 🔴 2.4.3 Update login page to show TOTP input when intermediate cookie present
  - [ ] 🔴 2.4.4 Add TOTP status badge to admin shell
  - [ ] 🔴 2.4.5 Run all admin-next tests

- [ ] 🔴 **2.5** — TOTP recovery codes
  - [ ] 🔴 2.5.1 Write spec: test recovery code generation, use, and one-time enforcement
  - [ ] 🔴 2.5.2 Generate 8 recovery codes on 2FA setup (hashed in DB)
  - [ ] 🔴 2.5.3 `POST /admin/login/recovery` — authenticate with recovery code
  - [ ] 🔴 2.5.4 Show recovery codes once on setup, allow regeneration
  - [ ] 🔴 2.5.5 Run all tests

---

## Phase 3 — Session Hardening

- [ ] 🔴 **3.1** — Session management UI
  - [ ] 🔴 3.1.1 Write spec: test session list API + revoke
  - [ ] 🔴 3.1.2 `GET /admin/sessions` — list active sessions (token, createdAt, expiresAt)
  - [ ] 🔴 3.1.3 `POST /admin/sessions/:token/revoke` — invalidate specific session
  - [ ] 🔴 3.1.4 UI panel showing active sessions in admin settings
  - [ ] 🔴 3.1.5 Run all tests

- [ ] 🔴 **3.2** — "Kick all sessions"
  - [ ] 🔴 3.2.1 Write spec: test bulk revocation
  - [ ] 🔴 3.2.2 `POST /admin/sessions/revoke-all` — clear all sessions except current
  - [ ] 🔴 3.2.3 UI button with confirmation dialog
  - [ ] 🔴 3.2.4 Run all tests

- [ ] 🔴 **3.3** — Session fingerprinting (IP + User-Agent)
  - [ ] 🔴 3.3.1 Write spec: test fingerprint recording and mismatch detection
  - [ ] 🔴 3.3.2 Add `ip`, `userAgent` columns to `admin_sessions` table
  - [ ] 🔴 3.3.3 Record fingerprint on session creation
  - [ ] 🔴 3.3.4 Optional: flag sessions where IP changes mid-session
  - [ ] 🔴 3.3.5 Run all tests

---

## Phase 4 — Defense in Depth

- [ ] 🔴 **4.1** — IP allowlisting for admin routes
  - [ ] 🔴 4.1.1 Write spec: test allowlist middleware
  - [ ] 🔴 4.1.2 Add `CC_ADMIN_IP_ALLOWLIST` env var (comma-separated CIDRs)
  - [ ] 🔴 4.1.3 Implement IP allowlist middleware in `src/ip-allowlist.middleware.ts`
  - [ ] 🔴 4.1.4 Apply middleware to `/admin/*` and `/admin-next/*` routes
  - [ ] 🔴 4.1.5 Run all tests

- [ ] 🔴 **4.2** — Admin audit log
  - [ ] 🔴 4.2.1 Write spec: `src/admin/admin-audit.service.spec.ts`
  - [ ] 🔴 4.2.2 Add `admin_audit_log` table to `database.service.ts`
  - [ ] 🔴 4.2.3 Implement `src/admin/admin-audit.service.ts`
  - [ ] 🔴 4.2.4 Log: login success/failure, token create/revoke, config changes, 2FA changes
  - [ ] 🔴 4.2.5 Run all tests

- [ ] 🔴 **4.3** — HSTS header
  - [ ] 🔴 4.3.1 Add `Strict-Transport-Security` header to `src/security-headers.ts`
  - [ ] 🔴 4.3.2 Configurable via `CC_HSTS_MAX_AGE` env var
  - [ ] 🔴 4.3.3 Run all tests

- [ ] 🔴 **4.4** — Password complexity enforcement
  - [ ] 🔴 4.4.1 Write spec: test complexity rules
  - [ ] 🔴 4.4.2 Enforce min 12 chars + mixed case + digits on `CC_ADMIN_PASSWORD`
  - [ ] 🔴 4.4.3 Add `CC_ADMIN_PASSWORD_MIN_LENGTH` env var override
  - [ ] 🔴 4.4.4 Log warning on weak passwords instead of failing startup (for migration)
  - [ ] 🔴 4.4.5 Run all tests

- [ ] 🔴 **4.5** — WebSocket auth origin check
  - [ ] 🔴 4.5.1 Write spec: test WS origin validation
  - [ ] 🔴 4.5.2 Add origin check to `agent.gateway.ts` WebSocket upgrade
  - [ ] 🔴 4.5.3 Reject non-matching origins with 403
  - [ ] 🔴 4.5.4 Run all tests
