# PTY session id change — test sweep (cc-server)

Use the Node/npm build that matches **`better-sqlite3`** (see `cc-server/package.json` script `test:programfiles-node` on Windows when using nvm).

## Commands

```bash
cd cc-server
npm test
# or (Windows, Program Files Node + npm)
npm run test:programfiles-node
```

## High-signal suites for PTY / logs / worker

| Area | Vitest path |
|------|-------------|
| Canonical id helpers | `src/pty/ccPtySessionId.spec.ts`, `src/logs/functionStatusSessionId.spec.ts` |
| Agent event handler — device-id verification | `src/agent/agent-event-handler.spec.ts` |
| Nest log store + runtime summary | `src/logs/log.store.spec.ts` |
| Nest PTY controller / agent gateway | `src/pty/pty.controller.spec.ts`, `src/agent/agent.gateway.spec.ts` |
| Worker D1 log store | `src/worker/db/d1-log-store.spec.ts` |
| Worker agent polling + SSH dispatch | `src/worker/agent-poll.spec.ts` |
| Worker admin PTY relay | `src/worker/admin-pty.spec.ts` |
| Worker logs ingest | `src/worker/logs-ingest.spec.ts` |
| Worker SSE streams | `src/worker/streams.spec.ts` |

Run a subset:

```bash
npx vitest run src/pty/ccPtySessionId.spec.ts src/logs/functionStatusSessionId.spec.ts src/agent/agent-event-handler.spec.ts src/logs/log.store.spec.ts src/worker/agent-poll.spec.ts src/worker/admin-pty.spec.ts src/worker/logs-ingest.spec.ts src/worker/streams.spec.ts
```

## admin-next

```bash
cd cc-server/admin-next
npm test
```

See Phase 3 in `plan.md` for UI fixture updates.
