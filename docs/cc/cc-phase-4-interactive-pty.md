# Phase 4 — Interactive PTY

## Scope
- Remote terminals via CC: open/write/resize/close mapped to agent PTYs.

## Current implementation note

This phase is still design-only in the current codebase pair (`SuperControlPanel` + `cc-server`):

- the local app already has PTY commands for its own UI
- the CC agent/server protocol does not yet define or handle remote `open_pty`, `write_pty`, `resize_pty`, or `close_pty`
- there is no remote Admin terminal UI or CC-mediated PTY session lifecycle yet

Treat the rest of this document as the target design, not as shipped behavior.

## Protocol
- CC → Agent: `open_pty`, `write_pty`, `resize_pty`, `close_pty`.
- Agent → CC: `pty-output`, `pty-exit` with `{sessionId}`.

## Agent
- PTY multiplex; backpressure to CC on slow consumers.
- Idle timeout + server-side session GC.

## PTY hardening
- Input and encoding
  - UTF-8 only; invalid sequences replaced with U+FFFD.
  - Enable bracketed paste; do not split pasted chunks mid-UTF8 rune.
  - Pass-through mouse reporting (xterm SGR mode) when enabled by the app.
  - Sanitize NUL and disallowed control sequences; drop ESC [ 8 ; ... ; t (window ops), OSC 52 (clipboard) unless explicitly allowed in org policy.
- Windows vs Unix
  - Windows: ConPTY; Unix: `portable-pty`. Normalize resize semantics (cols×rows) and CRLF handling so apps see consistent behavior.
  - Wide-char rendering sanity checks on first attach; warn if detected terminal font width mismatch.
- Rate limits and abuse protection
  - Keystrokes: average 100 cps, burst up to 200 cps (token-bucket). Excess input dropped and `nack { rate_limited: true }` surfaced to UI.
  - Resize: 10/s steady, burst up to 20/s. Additional resizes coalesced (drop intermediate) with last-known applied.
- Scrollback and replay
  - Per-session scrollback cap: 128 KB (org-configurable: 32–512 KB). On attach, replay up to the cap. Truncation is indicated with a header line.
  - Optional server-side recording does not change scrollback size; recording writes are async and bounded by backpressure rules.
- Recording and privacy
  - Recording: off by default; org-configurable retention (default 7 days). Redaction rules applied (see Telemetry Redaction Matrix).
  - Privacy banner is displayed to the operator on record-enabled sessions; operator must acknowledge before input is sent.
- Security and RBAC
  - Enforce per-session ACL checks before open/write/resize/close. Deny emits `nack { status: "unauthorized" }`.
  - Explicit consent required for record-enabled sessions; consent stored in audit with actor, device, session, timestamp.
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

## SLOs (Phase 4 PTY)
- Keystroke → echo: P50 ≤ 120 ms, P95 ≤ 300 ms (WAN)
- Resize propagation: first paint within 1 frame (≤ 100 ms target)
- Attach to running session: scrollback delivered ≤ 300 ms for 128 KB buffer
- Session cleanup: idle timeout enforced ≤ 5 s after threshold
- Error budget: <0.1% `nack rate_limited` under steady-state typing; bursts may be limited per policy

## Test plan (Phase 4 PTY)

## Test plan (Phase 4 PTY)

- Keystroke echo latency
  - Simulate 100 keystrokes at 5 Hz and 20 Hz over WAN-like latency (100–200 ms RTT) and assert P50 ≤ 120 ms, P95 ≤ 300 ms echo from `pty-output`.
- Resize behavior
  - Emit rapid `resize_pty` events (10 resizes over 2s). Assert the terminal reflects dimensions within one frame and no garbled output.
- Scrollback replay
  - Produce 200 KB output, detach, reattach. Assert only last 64–128 KB (configured) replays, with truncation notice present.
- Recording flag & privacy banner
  - Start session with recording enabled; assert banner rendered and audit entry created. With recording disabled, banner absent.
- Rate limit/abuse protection
  - Send keystrokes at 200 Hz for 10s; assert `nack rate_limited` and graceful degradation (no agent crash), then recovery after cooldown.
- Cross-platform
  - Windows (ConPTY) vs Unix (portable-pty): verify wide-char rendering, paste, mouse reporting, and consistent resize semantics.
- Security
  - Verify per-session ACL enforcement: unauthorized actor cannot attach; attempt is audited with `unauthorized`.
- Cleanup
  - Idle timeout triggers server-side GC; session is closed and no residual resources leak.
