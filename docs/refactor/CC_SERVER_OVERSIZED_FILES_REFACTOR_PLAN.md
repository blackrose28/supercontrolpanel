## Context
`cc-server` has several oversized files that slow edits and increase regression risk: `public/admin.js`, `src/agent/agent.gateway.ts`, `src/commands/command-dispatch.service.ts`, `src/file-ops/file-transfer.store.ts`, and `src/pty/pty.gateway.ts`. The goal is to split them into focused modules while preserving current API/WS/DB behavior and enabling safer incremental changes.

## Recommended approach
Use incremental extraction with a **facade-first** strategy: keep current entry files stable, extract pure helpers and focused handlers first, then delegate orchestration.

### Phase 0 — Baseline verification (no behavior changes)
- Capture current behavior for critical flows (agent WS connect/message handling, PTY lifecycle, transfer lifecycle, admin UI key screens).
- Ensure `npm run build` is green as a gate before/after every slice.

### Phase 1 — Refactor `src/file-ops/file-transfer.store.ts` (lowest risk first)
- Extract:
  - `src/file-ops/file-transfer.types.ts`
  - `src/file-ops/file-transfer.validators.ts`
  - `src/file-ops/file-transfer.mappers.ts`
  - optional query/constants module `src/file-ops/file-transfer.queries.ts`
- Keep `file-transfer.store.ts` as facade with existing public methods/signatures.

### Phase 2 — Refactor `src/pty/pty.gateway.ts`
- Extract:
  - `src/pty/pty-auth.service.ts`
  - `src/pty/pty-policy.service.ts`
  - `src/pty/pty-message.router.ts`
  - `src/pty/pty-message.handlers.ts`
  - `src/pty/pty-gateway.errors.ts`
- Keep gateway lifecycle methods (`handleConnection`/`handleDisconnect`) in `pty.gateway.ts`, delegating message handling.

### Phase 3 — Refactor `src/commands/command-dispatch.service.ts`
- Extract:
  - `src/commands/command-dispatch.types.ts`
  - `src/commands/command-dispatch.parsers.ts`
  - `src/commands/command-dispatch.policies.ts`
  - `src/commands/command-dispatch.transfer.ts`
  - `src/commands/command-dispatch.remote-browser.ts`
  - `src/commands/command-dispatch.generic.ts`
- Keep `CommandDispatchService` as composition root and route by command family.

### Phase 4 — Refactor `src/agent/agent.gateway.ts`
- Extract:
  - `src/agent/agent-auth.service.ts`
  - `src/agent/agent-envelope.service.ts`
  - `src/agent/agent-message.router.ts`
  - `src/agent/agent-telemetry.handler.ts`
  - `src/agent/agent-pty.handler.ts`
  - `src/agent/agent-file-op.handler.ts`
- Keep gateway as websocket facade and DI anchor; preserve protocol envelopes and response-correlation behavior.

### Phase 5 — Refactor `public/admin.js` (largest, last)
- Split into ES modules under `public/admin/`:
  - `app-state.js`, `api-client.js`, `formatters.js`
  - `devices-view.js`, `logs-view.js`, `pty-view.js`, `transfers-view.js`, `remote-browser-view.js`, `telemetry-view.js`, `alerts-view.js`
  - `index.js` bootstrap
- Preserve DOM IDs, event names, and backend contracts during split.

## Incremental commit slices
1. Extract file-transfer types/validators
2. Extract file-transfer mappers/queries
3. Extract PTY utility/error/parser helpers
4. Extract PTY action handlers/router
5. Extract command dispatch parsers/policies
6. Extract command transfer and remote-browser sub-handlers
7. Extract agent auth/envelope utilities
8. Extract agent message handlers/router
9. Split admin utilities (`api-client`, `formatters`, shared state)
10. Split admin feature modules one by one
11. Final wiring/import cleanup

## Execution checklist (live)

### Completed
- [x] Phase 1: `file-transfer.store.ts` split into:
  - `src/file-ops/file-transfer.types.ts`
  - `src/file-ops/file-transfer.validators.ts`
  - `src/file-ops/file-transfer.mappers.ts`
  - `src/file-ops/file-transfer.queries.ts`
- [x] Phase 2: `pty.gateway.ts` split into:
  - `src/pty/pty-auth.service.ts`
  - `src/pty/pty-policy.service.ts`
  - `src/pty/pty-message.router.ts`
  - `src/pty/pty-message.handlers.ts`
  - `src/pty/pty-gateway.errors.ts`
  - plus utility extractions: `src/pty/pty-rate-limit.ts`, `src/pty/pty-input-sanitizer.ts`
- [x] Phase 3: `command-dispatch.service.ts` split into:
  - `src/commands/command-dispatch.types.ts`
  - `src/commands/command-dispatch.parsers.ts`
  - `src/commands/command-dispatch.policies.ts`
  - `src/commands/command-dispatch.transfer.ts`
  - `src/commands/command-dispatch.remote-browser.ts`
  - `src/commands/command-dispatch.generic.ts`
  - supporting slices: `src/commands/command-dispatch.transfer-ack.ts`, `src/commands/command-dispatch.catalog.ts`, `src/commands/command-dispatch.errors.ts`, `src/commands/command-dispatch.validation.ts`
- [x] Phase 4 (started): first extraction from `src/agent/agent.gateway.ts`
  - `src/agent/agent-transport.ts` created
  - `sendMessage` + `payloadHasServerId` delegated to transport helper
- [x] Phase 4: outbound request-method extraction completed
  - `src/agent/agent-request-handler.ts` created
  - delegated methods: status/inventory/recent logs, restart requests, file-op dispatch, PTY open/write/resize/close
  - characterization tests added with Vitest in `src/agent/agent.gateway.spec.ts`
  - test tooling added: `vitest.config.ts`, `npm test`, `npm run test:coverage`

### In progress
- [ ] Continue Phase 4: extract inbound handlers/router layers from `src/agent/agent.gateway.ts`

  **Inbound handler checklist (AgentGateway):**
  - [x] `handleAck` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handleNack` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handleHeartbeat` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handleStatusSnapshot` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handleInventory` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handleRecentLogs` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handleFunctionStatus` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handlePtyOutput` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handlePtyExit` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handleTransferProgress` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handleTransferStatus` → delegated to `src/agent/agent-event-handler.ts`
  - [x] `handleCommandStatus` → delegated to `src/agent/agent-event-handler.ts`

  **Progress:** 12/12 inbound handlers delegated, 0 remaining.
  - [x] Inbound handler extraction complete for `AgentGateway`.

  **Router-layer checklist (AgentGateway):**
  - [x] `handleRawMessage` dispatch extraction to `src/agent/agent-message-router.ts`
  - [x] Router parse/error guard for invalid JSON (`try/catch` behavior parity)
  - [x] Router unknown-message fallback logging (`Unknown message type` parity)
  - [x] Router dispatch map parity for all inbound envelope types:
    - [x] `ack`
    - [x] `nack`
    - [x] `heartbeat`
    - [x] `status_snapshot`
    - [x] `inventory`
    - [x] `recent_logs`
    - [x] `command_status`
    - [x] `function-status`
    - [x] `pty-output`
    - [x] `pty-exit`
    - [x] `transfer_progress`
    - [x] `transfer_status`
  - [x] Facade wiring: `AgentGateway.handleRawMessage` delegates to router helper

  **Router progress:** 16/16 checklist items completed.
  - [x] Router-layer extraction complete for `AgentGateway`.
  - [x] `src/agent/agent-message-router.ts` created and wired.
  - [x] TDD characterization remains green for router dispatch parity.
  - [x] Build gate re-validated (`npm run build`).

  **Phase 4 status:** complete (handler layer + router layer).

  - [x] TDD characterization (`src/agent/agent.gateway.spec.ts`) remains green for delegated handlers
  - [x] Build gate re-validated (`npm run build`)

  - [x] TDD characterization (`src/agent/agent.gateway.spec.ts`) remains green for delegated handlers
  - [x] Build gate re-validated (`npm run build`)

### Remaining
- [x] Finish Phase 4: extract router layer from `src/agent/agent.gateway.ts` using `src/agent/agent-message-router.ts`
- [~] Start Phase 5: split `public/admin.js` into ES modules under `public/admin/`
- [ ] Final wiring/import cleanup and end-to-end smoke pass

### Next up (immediate)
- [x] Phase 5 slice 1: split `public/admin.js` shared utilities/bootstrap-safe helpers
  - [x] `public/admin/formatters.js` created (bootstrap-safe global export)
  - [x] extracted: `formatBytes`, `formatRate`, `formatEta`
  - [x] wired `views/devices.ejs` to load `/admin/formatters.js` before `/admin.js`
  - [x] `public/admin.js` delegated to extracted formatters with inline fallback parity
  - [x] validation: `npm test` + `npm run build` green
- [~] Phase 5 slice 2: split one feature module at a time with contract parity checks
  - [x] `public/admin/token-manager.js` created
  - [x] extracted token feature helpers: scope render/select, token reveal/mask, token rows, token CRUD wrappers
  - [x] wired `views/devices.ejs` to load `/admin/token-manager.js` before `/admin.js`
  - [x] `public/admin.js` delegates token flows to `AdminTokenManager` with fallback parity
  - [x] validation: `npm test` + `npm run build` green
  - [x] extracted next feature module: `public/admin/alerts-manager.js`
  - [x] wired `views/devices.ejs` to load `/admin/alerts-manager.js` before `/admin.js`
  - [x] `public/admin.js` delegates alerts refresh/UI handlers to `AdminAlertsManager` with fallback parity
  - [x] validation after alerts extraction: `npm test` + `npm run build` green
  - [x] extracted next feature module: `public/admin/commands-panel.js`
  - [x] wired `views/devices.ejs` to load `/admin/commands-panel.js` before `/admin.js`
  - [x] `public/admin.js` delegates command panel flows to `AdminCommandsPanel` with fallback parity
  - [x] validation after commands extraction: `npm test` + `npm run build` green
  - [x] extracted next feature module: `public/admin/transfers-panel.js`
  - [x] wired `views/devices.ejs` to load `/admin/transfers-panel.js` before `/admin.js`
  - [x] `public/admin.js` delegates transfer panel helpers to `AdminTransfersPanel` with fallback parity
  - [x] validation after transfers extraction: `npm test` + `npm run build` green
  - [x] remaining in slice 2: extract next feature module (transfers)
  - [x] extracted next feature module: `public/admin/telemetry-widgets.js`
  - [x] wired `views/devices.ejs` to load `/admin/telemetry-widgets.js` before `/admin.js`
  - [x] `public/admin.js` delegates telemetry helpers to `AdminTelemetryWidgets` with fallback parity
  - [x] validation after telemetry extraction: `npm test` + `npm run build` green
  - [x] extracted next feature module: `public/admin/logs-stream.js`
  - [x] wired `views/devices.ejs` to load `/admin/logs-stream.js` before `/admin.js`
  - [x] `public/admin.js` delegates log stream helpers to `AdminLogsStream` with fallback parity
  - [x] validation after logs-stream extraction: `npm test` + `npm run build` green
  - [x] extracted next feature module: `public/admin/server-browser.js`
  - [x] wired `views/devices.ejs` to load `/admin/server-browser.js` before `/admin.js`
  - [x] `public/admin.js` delegates server-picker/remote-browser helpers to `AdminServerBrowser` with fallback parity
  - [x] validation after server-browser extraction: `npm test` + `npm run build` green
  - [x] extracted next feature module: `public/admin/pty-panel.js`
  - [x] wired `views/devices.ejs` to load `/admin/pty-panel.js` before `/admin.js`
  - [x] `public/admin.js` delegates PTY panel helpers to `AdminPtyPanel` with fallback parity
  - [x] validation after PTY extraction: `npm test` + `npm run build` green
  - [x] next up in Phase 5: continue remaining feature modules and final wiring cleanup
- [x] Phase 5 slice 3: finalize imports/wiring and remove dead code from monolith file (logs + server-browser + pty + transfers + telemetry + alerts fallback cleanup done)
- [~] Phase 5 slice 4: bootstrap hardening for initialization-order safety in `public/admin.js`
  - [x] fixed formatter TDZ (`formatBytes`/`formatRate`/`formatEta` init order)
  - [x] fixed command refresh TDZ (`refreshCommands`/`refreshCommandsSoon` lazy delegate wiring)
  - [x] fixed missing `debugPty` symbol and wired debug helper
  - [x] fixed alerts interval TDZ (`refreshAlerts` deferred callback)
  - [x] hardened early module dependency wiring with lazy function delegates for PTY/transfers cross-references
  - [x] PTY wrapper trim: removed remaining local pass-throughs (`getSelectedPtySession` / `isAdminSessionAttached` / `setPtyStatus`) and rewired callers to `ptyPanel.*`
  - [x] batch wrapper trim: converted transfer/server-browser/logs delegates from local pass-through functions to direct const bindings
  - [x] batch call-site rewrite: switched transfer gating callers to `transfersPanel.can*` direct calls
  - [x] validation after latest batch trim: `node --check public/admin.js` + `npm test` + `npm run build` green
  - [x] validation after latest PTY trim: `node --check public/admin.js` + `npm test` + `npm run build` green
  - [x] validation after bootstrap hardening: `npm test` + `npm run build` green

### Remaining work snapshot (Phase 5+)
- Phase 4 inbound handlers: complete
- Phase 4 router layer: complete
- Phase 5 (`public/admin.js` modular split): in progress (feature extraction + wiring cleanup complete; bootstrap hardening complete)
- Final wiring/smoke pass: in progress (runtime browser pass still being verified)

### Remaining items total: 1 (final wiring/smoke 1)

> Counting rule: totals above count checklist bullets in this plan file (not LOC or time). Total updates as checklist granularity changes.
### Build checkpoint
- [x] `npm run build` passing after latest completed slice
- [x] Re-run build after each remaining extraction slice

### Current large-file snapshot (for tracking)
- `public/admin.js`: 2635 LOC
- `src/agent/agent.gateway.ts`: 1240 LOC
- `src/commands/command-dispatch.service.ts`: 277 LOC
- `src/file-ops/file-transfer.store.ts`: 352 LOC
- `src/pty/pty.gateway.ts`: 245 LOC

### Last updated
- 2026-04-18 (ongoing refactor session)

## Reuse existing project patterns
- Controller/service/store pattern from:
  - `src/commands/commands.controller.ts`
  - `src/commands/command.store.ts`
- Policy-service separation from:
  - `src/file-ops/file-op-admission.service.ts`
  - `src/file-ops/file-op-quota.service.ts`
  - `src/file-ops/file-op-slo.service.ts`
- PTY transport split template from:
  - `src/pty/pty.controller.ts`
  - `src/pty/pty-hub.service.ts`
  - `src/pty/pty-session.store.ts`
- Logs utility extraction template from:
  - `src/logs/log.store.ts`
  - `src/logs/log-redaction.ts`

## Verification checklist (per slice)
- Build:
  - `npm run build`
- Backend contract checks:
  - REST route paths/status/body unchanged for impacted endpoints
  - WS message `type` fields/payload keys unchanged (`/agent`, `/admin/ws/pty`)
  - DB write behavior unchanged for transfer/pty/log tables
- Functional smoke:
  - Agent connect + message roundtrip
  - PTY open/write/resize/close + unauthorized path
  - Transfer start/progress/finalize/resume
  - Admin UI: devices/logs/pty/transfers/telemetry views

## GitNexus requirements for implementation phase
Before editing each symbol in implementation:
1. Run `gitnexus_impact({target: "<symbol>", direction: "upstream"})` and report blast radius.
2. If risk is HIGH/CRITICAL, pause and confirm before proceeding.
3. Before commit, run `gitnexus_detect_changes()` to verify only expected scope changed.
4. Ensure all d=1 dependents are updated where applicable.
