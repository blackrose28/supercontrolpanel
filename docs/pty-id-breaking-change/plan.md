# CC PTY session id — breaking change plan

**Status:** In development — no production migration. Old random “remote” PTY ids and `RemotePtyStore` local↔remote mapping are replaced by a **single canonical id** shared across agent, control plane, logs, and admin UI.

**Goal:** One opaque `sessionId` on the entire **CC plane** (WebSocket, HTTP ingest, DB rows, `function-status`, `pty-output`, admin PTY APIs, tunnel log correlation). **Local** Tauri `SessionStore` ids stay unchanged for `write_pty` / desktop-only paths; the bridge is only at the CC boundary.

---

## Canonical format

- **String:** `ccpty:{deviceId}:{localSessionId}`
- **Rules:**
  - `deviceId` and `localSessionId` are trimmed non-empty strings.
  - Neither segment may contain `:` (so split/parse stays unambiguous).
  - Server **must** verify that the `deviceId` segment equals the authenticated agent’s device before accepting any PTY operation keyed by this id.

**Implementation helpers (Phase A — started):**

| Location | Module | Status |
|----------|--------|--------|
| Tauri | `src-tauri/src/cc/pty_session_id.rs` | `cc_pty_session_id`, `parse_cc_pty_session_id` + `cargo test pty_session_id` |
| cc-server | `cc-server/src/pty/ccPtySessionId.ts`, `ccPtySessionId.spec.ts` | Vitest; Worker can import the same module when wired |

Both must stay **bit-for-bit identical** in format and validation rules. Add a cross-check in CI later if needed (golden vectors).

---

## Phase 0 — Contract (this document)

- [x] Lock format and validation rules (above).
- [x] Add a one-page **protocol note** in `docs/cc/` referencing this plan for agent/gateway authors.

---

## Phase 1 — Control plane (`cc-server`)

**Objective:** Server **does not mint** a second id for the same PTY; it **accepts** the canonical id from the agent (or derives it with the **same** function as the agent so they never disagree).

**Progress (incremental):**

- [x] New session create paths (admin + worker) use `ccPtySessionId` / `newServerSshStartSessionId` / `resolveServerSshAttachSessionId` (PR B).
- [x] `function-status` + ingest: `coalesceFunctionStatusSessionId` — top-level `sessionId` preferred over `payload.sessionId`; `LogStore.append` / `D1LogStore.append` persist merged id; runtime summary merge uses helper; `getLastSessionIdForFunction` scans recent rows with coalescing (tunnel log resolution).
- [x] Agent handler: `handleAgentPtyOutput` / `handleAgentPtyExit` / `handleAgentFunctionStatus` prefer envelope `sessionId` before payload; worker WS `function-status` uses same coalescing before `appendAgentTelemetry`.
- [x] OpenAPI: `CcPlanePtySessionId` schema; `request-attach-server-ssh` optional `sessionId`; removed unused `sessionId` from `request-start-server-ssh` body; ingest + `pty-sessions` descriptions (`openapi.yaml`).
- [x] Exported / author types: `TelemetryEvent` / `WorkerTelemetryEvent` `sessionId` JSDoc (`log.store.ts`, `d1-log-store.ts`).
- [x] Test sweep checklist: `test-sweep.md` (this folder). Worker suites `admin-pty`, `logs-ingest`, `streams` verified green with current tree.

### Touch areas

- Agent WebSocket / gateway: `agent.gateway`, `agent-event-handler`, `agent-message-router`, `pty.gateway`, `pty-hub`.
- PTY session persistence (Nest + Worker Durable Object / D1): primary key = canonical id.
- Admin PTY attach / write / resize / close (`admin-pty`, worker relay routes).
- HTTP/WS responses returning `sessionId` to admin-next (`request-start-server-ssh`, attach, `pty-sessions` list).
- Ingest: `POST /v1/logs`, `handleAgentPtyOutput` — top-level `sessionId` = canonical.
- `device_runtime_state` merge (`log.store.ts`, `d1-log-store.ts`): for `function-status`, **prefer top-level** `event.sessionId` over `payload.sessionId` when building `summary.functions[*].sessionId`.
- Tunnel logs resolution (`tunnelLogs` / worker `handleTunnelLogs`) — align with canonical id only.
- OpenAPI (`openapi.yaml`) + author JSDoc on telemetry types (`TelemetryEvent`, `WorkerTelemetryEvent`).

### Suggested PR order

1. Shared `ccPtySessionId` usage in **new** session create paths (admin + agent).
2. Ingest + runtime summary merge + tunnel logs.
3. Sweep tests (`admin-pty.spec.ts`, `streams.spec.ts`, `logs-ingest.spec.ts`, `log.store.spec.ts`, worker DB specs).

---

## Phase 2 — Desktop agent (`src-tauri`)

**Objective:** Remove dual-id identity at the CC wire; mapping becomes **derivation** only.

**Progress (started):**

- [x] **RemotePtyStore:** removed redundant `local_to_remote` map; resolve local→CC id by scanning `sessions`; `bind_session` evicts stale rows that reused the same local id; `unbind_session` / `clear_session_binding` verify local↔remote pair.
- [x] **`function-status` telemetry:** `upsert_function_status` / `upsert_boundary_status` pass **canonical** top-level `sessionId` to `forward_log_event` when `RemotePtyStore` has a binding for the status’s local PTY id (`function_service.rs`); UI `FunctionStatus.session_id` stays local.
- [x] **WS telemetry (`connection.rs`):** `pty-output` and `pty-exit` listeners rewrite `id` → canonical `sessionId` via `RemotePtyStore::remote_session_for_local`; `pty-exit` clears the binding after send.
- [x] **SSE telemetry (`sse_connection.rs`):** `pty-output` already maps local→canonical; `pty-exit` now uses the same `pty_exit_payload_for_polling` helper and clears the binding (parity with WS path). Tests added.
- [x] **Dispatcher (`dispatcher.rs`):** `request-start-server-ssh` accepts a canonical `sessionId` and threads the parsed local segment through `start_server_ssh_action` so server- and agent-side ids agree (`forced_local_session_for_admin_ssh_start`); attach / open / write / resize / close paths bind, resolve, and unbind through `RemotePtyStore`.
- [x] **Service plumbing:** `services/pty_service.rs` and `services/server_service.rs` accept `forced_local_session_id` so the dispatcher can pin a local id derived from a canonical CC id.

### Touch areas

- `RemotePtyStore` — replace `local_to_remote` with canonical id as key, or delete mapping and compute `cc_pty_session_id(device_id, local_id)` at bind/send sites.
- `dispatcher.rs` — all CC-visible `sessionId` fields use canonical id.
- `connection.rs`, `sse_connection.rs` — telemetry `pty-output` / `pty-exit` use canonical id (no `remote_session_for_local` substitution for identity).
- `telemetry.rs` — `forward_log_event` passes canonical top-level `sessionId` for CC-bound events (including `function-status` when a PTY is bound).
- `function_service` — desktop `function-status` emit keeps **local** `session_id` on `FunctionStatus`; CC ingest top-level `sessionId` uses `RemotePtyStore` when bound, else same local id (OpenVPN / Boundary).

### Suggested PR order

1. Outbound WS telemetry (`pty-output` / `pty-exit`) + `forward_log_event` for those types.
2. Dispatcher attach / open / close / write paths.
3. Remove or slim `RemotePtyStore` identity mapping.

---

## Phase 3 — Admin UI (`cc-server/admin-next`)

- [x] `SshTerminalPanel.tsx` — `sessionId` already treated as opaque string (no parsing / generation).
- [x] `adminApi.ts` — no client-side id generation or parsing.
- [x] `ServerRuntimePage.tsx` + `.test.tsx` — updated; fixtures align with canonical ids where relevant.
- [ ] Optional: dashboard hooks audit pass — confirm no remaining UI assumptions about random remote-id format.

---

## Phase 4 — Security, docs, cleanup

- [x] **Reject canonical ids whose embedded `deviceId` ≠ authenticated device.**
  - Added `verifyCanonicalSessionIdForDevice` in `cc-server/src/pty/ccPtySessionId.ts`.
  - `handleAgentPtyOutput`, `handleAgentPtyExit`, `handleAgentFunctionStatus` now verify `envelope.sessionId` against `client.deviceId`; mismatched canonical ids are dropped (no log append, no PTY hub broadcast, warning emitted).
  - Tests: `cc-server/src/agent/agent-event-handler.spec.ts` (5 cases) + helper cases in `ccPtySessionId.spec.ts`.
- [x] **Length / rate limits on malformed ids.**
  - Per-segment cap `CC_PTY_SEGMENT_MAX = 128` and total cap `CC_PTY_SESSION_ID_MAX` mirrored in `cc-server/src/pty/ccPtySessionId.ts` and `src-tauri/src/cc/pty_session_id.rs`.
  - `parseCcPtySessionId` / `parse_cc_pty_session_id` reject oversized inputs at construction, parse, and verify time. Unit tests on both sides.
- [x] **Dead-code sweep.** No legacy random-remote-id paths remain in cc-server or src-tauri after PR slices A–E. `localSessionId` API fields and `RemotePtyStore::remote_session_for_local` are still load-bearing in active code paths and are correct to keep.
- [ ] Manual regression: tunnel logs, admin SSH terminal, CC reconnect, agent restart, multi-server. **Checklist:** [manual-regression.md](./manual-regression.md).

---

## PR slice table (reviewable chunks)

| ID | Scope | Notes |
|----|--------|--------|
| A | Spec + `cc_pty_session_id` / `ccPtySessionId` + unit tests | **Done** |
| B | cc-server: session create (admin + agent) uses canonical id | **Done** |
| C | cc-server: ingest + `device_runtime_state` + tunnel logs | **Done** |
| D | Tauri: WS + telemetry + dispatcher (incl. SSE pty-exit parity) | **Done** |
| E | Tauri: `RemotePtyStore` simplification | **Done** — `local_to_remote` removed, `clear_session_binding` requires matching pair |
| F | admin-next + fixtures | In progress (Phase 3) |

Run `cargo test` (Tauri), `npm test` (cc-server), `npm test` (admin-next) after B–D.

---

## Non-goals

- Migrating historical DB rows (dev-only reset).
- Changing local `SessionStore` id generation.
- Renaming unrelated “session” concepts (remote browser, MCP) unless they share PTY tables.

---

## References

- `src-tauri/src/stores/remote_pty_store.rs` — current local↔remote mapping.
- `src-tauri/src/cc/connection.rs` — `pty-output` uses remote id today.
- `cc-server/src/agent/agent-event-handler.ts` — `pty-output` ingest `functionId: null`.
- `cc-server/src/logs/log.store.ts` — `updateRuntimeSummary` merges `function-status` session from payload vs event.
