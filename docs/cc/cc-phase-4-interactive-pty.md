# Phase 4 — Interactive PTY

## Scope
- Remote terminals via CC: open/write/resize/close mapped to agent PTYs.

## Protocol
- CC → Agent: `open_pty`, `write_pty`, `resize_pty`, `close_pty`.
- Agent → CC: `pty-output`, `pty-exit` with `{sessionId}`.

## Agent
- PTY multiplex; backpressure to CC on slow consumers.
- Idle timeout + server-side session GC.

## CC / Admin UI
- Live terminal component; per-session ACLs.
- Replay last N KB scrollback on attach (agent keeps short buffer).

## Acceptance Criteria
- Keystroke → echo <120 ms P50 over WAN.
- Resize reflects within one frame.
- Session cleanup on disconnect.

## Risks
- Abuse/DoS via spam → rate limits + RBAC.
- Unicode/input edge cases → pass-through with tests.
