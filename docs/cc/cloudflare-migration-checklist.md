# cc-server Cloudflare Migration Checklist

Use this checklist one item at a time. For implementation items, do not check the implementation box until the RED/GREEN/REFACTOR boxes are complete.

## Dual-runtime gate

- [ ] Before each implementation slice, confirm the change adds Worker/Pages capability without removing existing NestJS + Vite capability.
- [ ] Before each implementation slice is complete, run or justify the relevant tests for both the Cloudflare path and the existing normal-server path.
- [ ] Do not delete existing NestJS routes, stores, gateways, Vite config, or normal-server docs unless a separate explicit cleanup plan is approved.


## Phase 0 — Baseline and inventory

- [x] Record current backend test baseline: `npm test` from `cc-server`. Baseline on 2026-04-29: 23 files / 148 tests passed; after Phase 1 scaffold: 24 files / 149 tests passed.
- [x] Record current admin test baseline: `npm --prefix cc-server/admin-next run test`. Baseline on 2026-04-29: 16 files / 152 tests passed; after Pages config test: 16 files / 153 tests passed.
- [x] Inventory all HTTP endpoints. See `docs/cc/cloudflare-migration-inventory.md`.
- [x] Inventory all WebSocket message types for `/agent`. See `docs/cc/cloudflare-migration-inventory.md`.
- [x] Inventory all WebSocket message types for `/admin/ws/pty`. See `docs/cc/cloudflare-migration-inventory.md`.
- [x] Inventory all SSE endpoints and event names. See `docs/cc/cloudflare-migration-inventory.md`.
- [x] Inventory SQLite tables, indexes, and data migration needs. See `docs/cc/cloudflare-migration-inventory.md`.
- [x] Choose Worker router/test stack. Decision: Worker-native fetch router with Vitest request/response tests first; consider Hono later only if needed.
- [x] Choose Durable Object sharding model. Decision: per-device Durable Object first; split per-PTY-session later only if Phase 7 needs it.

## Phase 1 — Cloudflare project scaffolding

- [x] RED: add failing Worker health-route test.
- [x] GREEN: add minimal Worker entrypoint returning health response.
- [x] REFACTOR: clean Worker scaffold with tests passing.
- [x] Add `wrangler.toml` with planned bindings.
- [x] RED: add Pages build/config test for `admin-next` assumptions.
- [x] GREEN: add Pages-compatible build/deploy config.
- [x] REFACTOR: keep frontend config simple and documented.
- [x] Run Worker tests.
- [x] Run admin-next tests/build.

## Phase 2 — D1 persistence foundation

- [x] RED: schema test expects required D1 tables.
- [x] GREEN: add initial D1 schema/migrations.
- [x] REFACTOR: organize migrations and binding helpers. Initial schema is isolated in `src/worker/db/d1-schema.ts`; executable migrations come next when D1 tooling is introduced.
- [x] RED: device store tests against async D1 store.
- [x] GREEN: implement D1 device store.
- [x] REFACTOR: remove duplication from device store implementation. Initial Worker store isolates D1 types and Web Crypto token normalization in `src/worker/db/d1-device-store.ts`.
- [x] RED: admin session/token store tests.
- [x] GREEN: implement D1 admin session/token stores.
- [x] REFACTOR: consolidate secret hashing/normalization helpers. Worker stores now share Web Crypto helpers in `src/worker/db/worker-secrets.ts`.
- [x] RED: logs/runtime summary store tests.
- [x] GREEN: implement D1 logs/runtime summary stores.
- [x] RED: command/audit store tests.
- [x] GREEN: implement D1 command/audit store.
- [x] RED: PTY session/audit store tests.
- [x] GREEN: implement D1 PTY session/audit store.
- [x] RED: file transfer/remote browser store tests.
- [x] GREEN: implement D1 file transfer/remote browser stores.
- [x] RED: rate limit/TOTP store tests.
- [x] GREEN: implement D1 rate limit/TOTP stores.
- [x] Document SQLite-to-D1 export/import path. See `docs/cc/cloudflare-d1-migration-runbook.md`.

## Phase 3 — Stateless HTTP API parity

- [x] RED: `GET /v1/health` route test. Covered by `cc-server/src/worker/worker.spec.ts`.
- [x] GREEN: implement Worker health route. Implemented in `cc-server/src/worker/index.ts`.
- [x] RED: `POST /v1/enroll` success/failure/rate-limit tests.
- [x] GREEN: implement enrollment route.
- [x] RED: device list/detail/delete/revoke tests.
- [x] GREEN: implement device routes.
- [x] RED: logs query/export/settings tests.
- [x] GREEN: implement logs routes.
- [x] RED: command list/detail/audit tests.
- [x] GREEN: implement command query routes.
- [x] RED: basic file transfer query/mutation tests.
- [x] GREEN: implement basic file transfer routes.
- [x] Run Worker API test suite.

## Phase 4 — Admin auth and Pages integration

- [x] RED: admin login cookie/session tests.
- [x] GREEN: implement Worker admin login/logout/session behavior.
- [x] RED: CSRF enforcement tests.
- [x] GREEN: implement CSRF middleware.
- [x] RED: TOTP setup/verify/recovery tests.
- [x] GREEN: implement Worker TOTP routes.
- [x] RED: admin API token scope tests.
- [x] GREEN: implement token routes and scope enforcement.
- [x] RED: admin-next unauthorized redirect test under Pages path.
- [x] GREEN: update frontend API/auth behavior if needed.
- [x] Validate security headers/CORS/cookies in local preview. Local runbook added at `docs/cc/cloudflare-local-preview-validation.md`; security headers covered by `src/worker/security-headers.spec.ts`.

## Phase 5 — Agent WebSocket migration

- [x] RED: rejects missing/invalid agent bearer token.
- [x] GREEN: implement `/agent` WS auth rejection.
- [x] RED: accepts valid agent and sends welcome envelope.
- [x] GREEN: implement valid Worker connection path with welcome envelope. Durable Object routing remains for subsequent lifecycle/dispatch items.
- [x] RED: duplicate device connection closes/replaces old session.
- [x] GREEN: implement duplicate connection policy.
- [x] RED: heartbeat/status/log messages update D1 state.
- [x] GREEN: implement inbound agent message routing.
- [x] RED: command dispatch emits correct request envelope to connected agent.
- [x] GREEN: implement Worker command dispatch to connected agent. Durable Object routing remains for production hardening.
- [x] RED: ack/nack updates command and pending response state.
- [x] GREEN: implement ack/nack lifecycle.
- [x] Manual smoke: real desktop agent connects to Worker preview. Confirmed manually on 2026-04-29.

## Phase 6 — Live admin streams

- [x] Decide SSE preservation vs WebSocket replacement. Decision: preserve EventSource/SSE contracts for admin-next and back Worker streams with D1 polling snapshots.
- [x] RED: devices stream emits `device-connected`/`device-disconnected` compatible events. Covered by `cc-server/src/worker/streams.spec.ts`.
- [x] GREEN: implement devices live stream using Worker SSE response and D1 status polling.
- [x] RED: logs stream emits `telemetry` events. Covered by `cc-server/src/worker/streams.spec.ts`.
- [x] GREEN: implement logs live stream using Worker SSE response and D1 telemetry polling.
- [x] RED: commands stream emits `command` events. Covered by `cc-server/src/worker/streams.spec.ts`.
- [x] GREEN: implement commands live stream using Worker SSE response and D1 command-state polling.
- [x] RED: transfers stream emits `file_transfer` events. Covered by `cc-server/src/worker/streams.spec.ts`.
- [x] GREEN: implement transfers live stream using Worker SSE response and D1 transfer-state polling.
- [x] Run admin-next stream tests. `npm --prefix cc-server/admin-next run test` passed on 2026-04-29 (154 tests).

## Phase 7 — Admin PTY WebSocket and interactive workflows

- [x] RED: `/admin/ws/pty` rejects unauthenticated admin client. Covered by `cc-server/src/worker/admin-pty.spec.ts`.
- [x] GREEN: implement admin PTY WS auth and accepted upgrade stub. Full PTY workflow remains in subsequent Phase 7 items.
- [x] RED: open PTY request creates pending session and dispatches agent request. Covered by `cc-server/src/worker/admin-pty.spec.ts`.
- [x] GREEN: implement initial Worker PTY open flow: admin `pty_open` creates opening session/audit, dispatches `open_pty`, and notifies device Durable Object.
- [x] RED: attach/detach updates session ownership and audit records. Covered by `cc-server/src/worker/admin-pty.spec.ts`.
- [x] GREEN: implement Worker `pty_attach`/`pty_detach` flow with D1 session ownership updates, audit records, and admin WebSocket acknowledgements.
- [x] RED: write/resize enforce policy and rate limits. Covered by `cc-server/src/worker/admin-pty.spec.ts`.
- [x] GREEN: implement Worker `pty_write`/`pty_resize` forwarding with attached-session policy, write size/rate limits, D1 audit, and Durable Object dispatch.
- [x] RED: close/delete update state and audit records. Covered by `cc-server/src/worker/admin-pty.spec.ts`.
- [x] GREEN: implement close/delete flow.
- [x] RED: agent `pty-output` fanout reaches attached admin clients. Covered by `cc-server/src/worker/admin-pty.spec.ts`.
- [x] GREEN: implement output fanout using Worker in-isolate admin subscriber fanout plus D1 output timestamp updates; Durable Object routing remains the long-term production hardening path.
- [ ] Manual smoke: browser xterm can interact with real agent PTY.

## Phase 8 — File operations and remote browser parity

- [x] RED: open remote browser creates command and browser session state. Covered by `cc-server/src/worker/file-transfers.spec.ts`.
- [x] GREEN: implement open remote browser dispatch.
- [x] RED: list/stat/mkdir/delete/close validate payloads and dispatch correctly. Covered by `cc-server/src/worker/file-transfers.spec.ts`.
- [x] GREEN: implement remote browser command routes.
- [x] RED: SCP upload/download command tests. Covered by `cc-server/src/worker/file-transfers.spec.ts`.
- [x] GREEN: implement SCP command dispatch.
- [x] RED: transfer progress/status messages update transfer state. Covered by `cc-server/src/worker/file-transfers.spec.ts`.
- [x] GREEN: implement transfer lifecycle handlers.
- [x] RED: stop/dismiss/force-remove/dismiss-finished tests. Covered by `cc-server/src/worker/file-transfers.spec.ts`.
- [x] GREEN: implement transfer mutation routes.
- [ ] Manual smoke: transfer lifecycle appears in admin-next.

## Phase 9 — Dual-runtime operations and production rollout

- [x] Document Cloudflare account/project setup. See `cc-server/README.md` and `docs/cc/cloudflare-local-preview-validation.md`.
- [x] Document required Worker secrets and bindings. See `cc-server/README.md`.
- [x] Document Pages deployment steps. See `cc-server/README.md` and `docs/cc/cloudflare-local-preview-validation.md`.
- [x] Document existing NestJS normal-server deployment remains supported. See `cc-server/README.md`.
- [x] Document D1 migration and rollback. See `docs/cc/cloudflare-d1-migration-runbook.md` and `cc-server/README.md`.
- [x] RED: any legacy cleanup test coverage needed. No legacy cleanup/deprecation is included in this dual-runtime migration, so no additional cleanup-specific regression test was required.
- [x] GREEN: document both runtime paths; do not remove/deprecate legacy Nest paths in this migration.
- [x] Run full Worker test suite. `npm test -- --run` passed on 2026-05-05 (49 files / 278 tests). The previous Windows temp directory cleanup `EPERM` failures were resolved by closing test SQLite handles before removing temp directories.
- [x] Run full admin-next test suite. `npm --prefix cc-server/admin-next run test -- --run` passed on 2026-05-05 (16 files / 157 tests).
- [x] Run admin-next production build. `npm --prefix cc-server/admin-next run build` passed on 2026-05-05.
- [ ] Run real E2E smoke: enroll, connect, logs, commands, PTY, file transfer.
- [x] Update `cc-server/README.md` with Cloudflare deployment instructions.

## Per-item TDD gate

Before marking any implementation item done, confirm:

- [ ] A failing test was written first.
- [ ] The test failed for the expected reason.
- [ ] Minimal implementation made it pass.
- [ ] Refactor, if any, kept tests passing.
- [ ] Relevant existing tests still pass.
- [ ] Contract/docs were updated if behavior changed.
- [ ] Existing NestJS + Vite runtime support was preserved.
