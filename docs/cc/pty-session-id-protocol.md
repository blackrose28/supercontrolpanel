# CC PTY session id (canonical `ccpty:` format)

This note is the short protocol reference for **agent**, **gateway**, and **admin API** authors. The full migration plan and rationale live in [`../pty-id-breaking-change/plan.md`](../pty-id-breaking-change/plan.md).

## Format

- **Value:** `ccpty:{deviceId}:{localSessionId}`
- **Rules:**
  - `deviceId` and `localSessionId` are non-empty after trim.
  - Neither segment may contain `:` (parsing is a single split after the prefix).
  - Any server or worker that accepts a canonical id **must** verify that `deviceId` equals the authenticated agent’s device id before treating the id as authoritative for that device.

## Usage

- **One id on the CC plane:** WebSocket control messages, HTTP ingest (`sessionId` on PTY log lines), D1 / session rows, admin “open / attach / write / resize / close” APIs, and `function-status` / runtime summaries should use this same string where a PTY session is identified for CC-visible traffic.
- **Desktop `SessionStore`:** The agent may keep an opaque local PTY key; the **local segment** of the canonical id is chosen so the agent can open the PTY under that key and stay aligned with telemetry. See the Tauri module `src-tauri/src/cc/pty_session_id.rs` and cc-server `cc-server/src/pty/ccPtySessionId.ts` (helpers must stay consistent).

## Related endpoints (cc-server)

- Admin **start server SSH** mints a new canonical id (`newServerSshStartSessionId`) and sends it to the agent as `request_start_server_ssh.payload.sessionId`.
- Admin **attach server SSH** resolves the canonical id from `deviceId` + `localSessionId`, with optional client `sessionId` that must match (`resolveServerSshAttachSessionId`).

## Legacy

Until all callers are migrated, some paths may still accept non-`ccpty` ids; new code should prefer canonical ids only. Historical DB rows are out of scope for automatic migration (see plan non-goals).
