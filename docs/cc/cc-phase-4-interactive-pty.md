# Phase 4 — Interactive PTY

## Scope
- Remote terminals via CC: open/write/resize/close mapped to agent PTYs.

## Protocol
- CC → Agent: `open_pty`, `write_pty`, `resize_pty`, `close_pty`.
- Agent → CC: `pty-output`, `pty-exit` with `{sessionId}`.

## Agent
- PTY multiplex; backpressure to CC on slow consumers.
- Idle timeout + server-side session GC.

## PTY hardening
- Input/encoding: support UTF-8 strictly; enable bracketed paste; pass-through mouse reporting; sanitize null/control sequences where required.
- Windows vs Unix: use ConPTY on Windows, `portable-pty` on Unix; normalize resize semantics; test wide-char rendering.
- Scrollback: agent keeps a small in-memory scrollback per session (e.g., 64–128 KB) for replay on attach; truncate with clear notice when exceeded.
- Recording: optional server-driven session recording flag; retain for a bounded period per org policy.
- Security/RBAC: enforce per-session ACLs; show operator privacy banner; require explicit consent if recording is enabled.
- Abuse protection: keystroke and resize rate limits; drop bursts over threshold with `nack rate_limited`.

## CC / Admin UI
- Live terminal component; per-session ACLs.
- Replay last N KB scrollback on attach (agent keeps short buffer).

## Acceptance Criteria
- Keystroke → echo <120 ms P50 over WAN.
- Resize reflects within one frame.
- Session cleanup on disconnect; scrollback replay works and truncates with notice.
- RBAC enforced; recording flag is visible and audited.

## Risks
- Abuse/DoS via spam → rate limits + RBAC.
- Unicode/input edge cases → pass-through with tests.
