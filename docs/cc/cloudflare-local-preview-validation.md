# Cloudflare Local Preview Validation

This runbook validates the Cloudflare Workers + Pages path without removing or replacing the existing NestJS + Vite deployment path.

## Prerequisites

From `cc-server`:

```bash
npm install
npm test
npm run build
npm --prefix admin-next run build
```

For local D1-backed auth/API checks, create or use a local D1 database with the Worker schema from `src/worker/db/d1-schema.ts` before exercising stateful routes.

## Preview modes

### Worker API preview

```bash
cd cc-server
npx wrangler dev
```

Default URL is usually `http://localhost:8787`.

### Pages static admin preview

Build admin-next first, then run:

```bash
cd cc-server
npm --prefix admin-next run build
npx wrangler pages dev public/admin-next --compatibility-date=2026-04-29
```

Default URL is usually `http://localhost:8788`.

## Required checks

### 1. Health route security headers

```bash
curl -i http://localhost:8787/v1/health
```

Expected response includes:

- `content-type: application/json; charset=utf-8`
- `x-content-type-options: nosniff`
- `referrer-policy: no-referrer`
- `permissions-policy` denying camera/microphone/geolocation
- `content-security-policy` with restrictive defaults

### 2. Admin login cookies

Open `/admin/login`, log in with `CC_ADMIN_PASSWORD`, and inspect cookies in browser DevTools.

Expected:

- `cc_admin_session`
  - `HttpOnly`
  - `SameSite=Strict`
  - `Secure` when served over HTTPS
- `cc_admin_csrf`
  - readable by frontend JavaScript
  - `SameSite=Strict`
  - `Secure` when served over HTTPS

### 3. Unauthorized admin-next redirect

1. Clear `cc_admin_session` and `cc_admin_csrf` cookies.
2. Open a Pages path such as `/admin-next/?device=dev-1#logs`.
3. Trigger an API load.

Expected redirect:

```text
/admin/login?next=%2Fadmin-next%2F%3Fdevice%3Ddev-1%23logs
```

### 4. CSRF rejection for cookie-authenticated mutations

With a valid `cc_admin_session` but no `csrfToken`, run a mutation:

```bash
curl -i \
  -X POST http://localhost:8787/v1/file-transfers/dismiss \
  -H 'Cookie: cc_admin_session=<session>' \
  -H 'Content-Type: application/json' \
  --data '{"deviceId":"dev-1","transferId":"tr-1"}'
```

Expected: `403` with `CSRF_INVALID`.

### 5. Bearer-token mutation compatibility

When `CC_ADMIN_API_TOKEN` is configured, the static env bearer token remains supported for backend/API compatibility and does not require CSRF:

```bash
curl -i \
  -X POST http://localhost:8787/v1/file-transfers/dismiss \
  -H 'Authorization: Bearer <CC_ADMIN_API_TOKEN>' \
  -H 'Content-Type: application/json' \
  --data '{"deviceId":"dev-1","transferId":"tr-1"}'
```

Expected: auth succeeds; route-specific validation may still return `404`/`409` if fixture data is missing.

## Pass/fail notes

Record the preview date, Wrangler command, URL, and any deviations in `docs/cc/cloudflare-migration-checklist.md` before marking the Phase 4 validation item complete.
