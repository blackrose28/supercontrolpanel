# cc-server Cloudflare Workers + Pages Migration Plan

## Goal

Add Cloudflare Workers and Cloudflare Pages deployment support for `cc-server` and `cc-server/admin-next` while preserving the existing control-plane behavior, security model, and live agent/admin workflows.

This is a **dual-runtime migration**, not a replacement migration:

- The existing NestJS backend must remain runnable on a normal cloud server.
- The existing Vite/admin-next development and build workflow must remain runnable.
- Cloudflare Worker and Pages code should be added alongside the current implementation.
- Do not delete or decommission existing NestJS + Vite paths unless a later explicit cleanup plan is approved.
- The end state should support both deployment modes: normal cloud/server deployment and Cloudflare infrastructure deployment.

## Non-negotiables

- Preserve dual-runtime support: existing NestJS + Vite must continue to build, test, and run while Worker/Pages support is added.
- Do not remove existing server-runtime code, routes, tests, or frontend behavior as part of the Cloudflare migration unless explicitly approved in a separate cleanup phase.
- Follow strict TDD for every implementation change:
  1. RED: write or update a failing test first.
  2. GREEN: implement the smallest change that passes.
  3. REFACTOR: clean up while keeping tests green.
- Keep changes incremental and reviewable.
- Preserve current public contracts unless a phase explicitly updates both caller and callee.
- Keep the desktop agent protocol compatible during migration.
- Run the relevant test suite before and after each implementation slice.
- Update docs when contracts, deployment steps, or operational behavior changes.

## Target architecture

Dual supported deployments:

1. Existing normal server deployment:
   - NestJS/Express backend remains available.
   - Existing Vite/admin-next development and backend-served static asset flow remains available.
2. Cloudflare deployment:
   - `cc-server/admin-next` deploys as a Cloudflare Pages static React/Vite app.
   - A Cloudflare Worker provides a Worker-native implementation of the HTTP API surface.
- Cloudflare D1 provides the Cloudflare deployment persistence layer; the existing normal-server path continues to use local `better-sqlite3`.
- Durable Objects own stateful WebSocket coordination for the Cloudflare deployment:
  - agent connection/session routing
  - admin PTY WebSocket fanout
  - per-device or per-session live coordination where needed
- Workers secrets and environment bindings provide Cloudflare runtime configuration; `.env` remains available for the existing normal-server deployment.
- SSE endpoints are either preserved using Worker streaming responses or replaced with WebSocket/event polling only after frontend/backend contract updates.

## Phase 0 decisions

- Worker router/test stack: start with a small Worker-native `fetch(request, env, ctx)` router and Vitest request/response tests. Introduce Hono only if route complexity justifies it.
- Durable Object sharding model: start with one per-device Durable Object for agent connection, command dispatch, and per-device live events; add per-PTY-session objects only if fanout/isolation requires it during Phase 7.

## Migration principles

1. Add Cloudflare support alongside the existing implementation; do not replace the NestJS/Vite path.
2. Build a compatibility layer before moving behavior.
3. Implement pure/stateless Worker endpoints before live WebSocket paths.
4. Keep the existing Nest server runnable throughout and after Worker parity is proven.
5. Use shared protocol/types where practical so agent/admin clients remain consistent across both runtimes.
6. Prefer narrow vertical slices: one endpoint or workflow at a time.
7. Every phase must validate both affected Cloudflare code and the existing normal-server path.

## Phases

### Phase 0 — Baseline and inventory

Deliverables:

- Document existing API, WebSocket, SSE, auth, DB tables, and deployment assumptions.
- Capture current test baseline for backend and admin frontend.
- Decide Worker router/testing stack.
- Decide D1 schema migration strategy.

TDD notes:

- No production code changes unless preceded by tests.
- If adding migration utilities, write tests for SQL/schema output first.

Validation:

- `npm test` in `cc-server`
- `npm --prefix cc-server/admin-next run test`

### Phase 1 — Cloudflare project scaffolding

Deliverables:

- Add Worker source entrypoint and `wrangler.toml`/environment binding plan.
- Add local Worker test harness.
- Add Pages build/deploy config for `admin-next`.
- Keep existing Nest entrypoint untouched.

TDD notes:

- First test: Worker health route returns expected JSON/status.
- First frontend test: production API/asset URL behavior remains correct for Pages.

Validation:

- Worker unit tests pass.
- `npm --prefix cc-server/admin-next run build` passes.

### Phase 2 — D1 persistence foundation

Deliverables:

- D1 schema equivalent for current SQLite tables.
- Async repository/store interfaces for devices, sessions, admin sessions, tokens, logs, commands, PTY sessions, transfers, remote browser sessions, rate limits, and TOTP.
- Migration/export notes for existing SQLite data.

TDD notes:

- For each store, port existing store specs first to the new async D1-backed interface.
- Confirm failing tests before implementation.

Validation:

- Store tests pass against local D1/test database.
- Schema migration tests prove required tables/indexes exist.

### Phase 3 — Stateless HTTP API parity

Deliverables:

- Worker routes for:
  - `GET /v1/health`
  - enrollment
  - device list/detail
  - logs query/export/settings
  - commands query/detail/audit
  - file transfer query/status endpoints that do not require live agent dispatch
- Shared auth middleware for bearer tokens, admin sessions, CSRF, scopes, and rate limits.

TDD notes:

- Port each controller spec to Worker route tests before implementing that route.
- Add regression tests for unauthorized/forbidden/error responses.

Validation:

- Worker API route tests pass.
- Existing Nest tests remain green until decommissioning.

### Phase 4 — Admin auth and Pages integration

Deliverables:

- Cloudflare-compatible admin login/logout/session/TOTP/token flows.
- `admin-next` served from Pages and configured to call Worker API.
- Cookie, CSRF, CORS, and security headers work across Pages + Worker domains.

TDD notes:

- Test cookie attributes, CSRF enforcement, redirects/401s, and TOTP edge cases first.
- Frontend tests cover auth redirect behavior under Pages paths.

Validation:

- Backend auth tests pass.
- Admin frontend tests pass.
- Manual login/logout smoke test in local preview.

### Phase 5 — Agent WebSocket migration

Deliverables:

- Worker/Durable Object `/agent` WebSocket endpoint.
- Agent token authentication.
- Device session registration, duplicate connection policy, heartbeat/status/log ingestion.
- Command dispatch request/ack/nack lifecycle.

TDD notes:

- Tests should simulate WebSocket messages and assert persisted state changes.
- Start with connection rejection and welcome-message tests.

Validation:

- Agent WS tests pass.
- Real desktop agent can enroll/connect to Worker preview.

### Phase 6 — Live admin streams

Deliverables:

- Preserve or replace:
  - `/v1/devices/stream`
  - `/v1/logs/stream`
  - `/v1/commands/stream`
  - `/v1/file-transfers/stream`
- Ensure admin-next live updates continue working.

TDD notes:

- First tests assert stream event format compatibility.
- Frontend tests verify subscriptions still update state.

Validation:

- Existing admin-next EventSource tests pass or are updated with contract changes.
- Live device/log/command/transfer updates work in preview.

### Phase 7 — Admin PTY WebSocket and interactive workflows

Deliverables:

- Worker/Durable Object `/admin/ws/pty` endpoint.
- PTY open/attach/detach/write/resize/close/delete flows.
- PTY audit and session expiration behavior.
- Browser terminal in `admin-next` works through Cloudflare.

TDD notes:

- Port `pty.gateway` and handler specs into Worker/Durable Object tests one behavior at a time.
- Add fanout tests for multiple admin clients attached to the same session.

Validation:

- PTY tests pass.
- Real browser terminal can attach to a desktop agent session.

### Phase 8 — File operations and remote browser parity

Deliverables:

- Remote browser open/list/stat/mkdir/delete/close command dispatch through Worker stack.
- SCP upload/download command submission and transfer progress lifecycle.
- Stop/dismiss/force-remove/dismiss-finished behavior.

TDD notes:

- Test command creation, dispatch payloads, transfer progress updates, and terminal states first.

Validation:

- File operation tests pass.
- Manual upload/download command lifecycle works with a test agent.

### Phase 9 — Dual-runtime operations and production rollout

Deliverables:

- Production wrangler/Page deployment docs.
- Secret/binding setup docs.
- SQLite-to-D1 migration/runbook.
- Rollback plan.
- Document how to operate both the existing Nest deployment path and the Cloudflare deployment path.
- Do not remove either runtime during this migration.

TDD notes:

- Any cleanup/refactor remains test-first.

Validation:

- Full backend Worker tests pass.
- Full admin-next tests/build pass.
- Real end-to-end smoke test passes for enrollment, device connection, commands, logs, PTY, and transfers.

## Definition of done

- Existing NestJS normal-server deployment still builds and tests successfully.
- Existing admin-next Vite workflow still builds and tests successfully.
- Cloudflare Pages serves `admin-next` in preview and production.
- Worker API passes route/auth/store tests.
- D1 contains migrated or freshly enrolled data as expected.
- Durable Object WebSocket flows are stable across reconnects.
- Existing desktop app/agent protocol still works or documented contract changes are implemented on both sides.
- README and operational docs reflect Cloudflare deployment.
