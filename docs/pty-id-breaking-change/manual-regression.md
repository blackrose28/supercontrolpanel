# Phase 4 — manual regression checklist

These are the runtime cases the automated tests can't cover, listed in `plan.md` Phase 4. Run them after rolling out the canonical CC PTY id work end-to-end.

## Setup

- Build the Tauri agent: `cd src-tauri && cargo build`.
- Run cc-server: `cd cc-server && npm run start:dev` (Nest) and the Worker dev script if exercising D1 paths.
- Open admin-next in a browser, sign in.
- Have at least **two registered devices** (so cross-device spoofing can be tested) and **two SSH-enabled servers** (so multi-server attach can be exercised).

## Cases

### 1. Admin SSH terminal — open + attach

- [ ] From admin UI, open SSH on Server A.
- [ ] Verify the terminal opens, prompts work, output streams.
- [ ] Refresh the page. Re-attach to the same session (admin-next should use the canonical `ccpty:{deviceId}:{local}` id).
- [ ] Type a command, confirm output continues.

### 2. Tunnel logs

- [ ] With an SSH PTY active, open the server runtime page.
- [ ] Tunnel logs should resolve and show recent PTY output for that PTY (canonical id correlation in `getLastSessionIdForFunction`).
- [ ] Close the SSH session. Confirm `pty-exit` is recorded and the runtime row updates.

### 3. CC reconnect

- [ ] With an SSH PTY active, kill the agent's network (toggle Wi-Fi or block the WS endpoint).
- [ ] Wait until the agent reports disconnected.
- [ ] Restore connectivity; agent should re-attach.
- [ ] Confirm the previously open PTY still resolves on its canonical id and output resumes.

### 4. Agent restart

- [ ] With an SSH PTY active, fully quit and relaunch the desktop agent.
- [ ] Confirm the agent re-establishes the WebSocket and the device shows online.
- [ ] Open a new SSH session — the new canonical id should differ from the previous one.

### 5. Multi-server

- [ ] Open SSH to Server A and Server B from the same device, simultaneously.
- [ ] Confirm output streams to the right terminal in each tab.
- [ ] Function-status rows for `server-ssh-srv-A` and `server-ssh-srv-B` should each carry their own canonical session id.

### 6. Spoof rejection (Phase 4 security)

- [ ] On Device 1's agent, manually craft and send an `envelope.sessionId = "ccpty:{Device 2's id}:loc-x"` for `pty-output` (e.g. via test harness).
- [ ] cc-server should drop the event and log `Rejected pty-output from {Device 1}: canonical sessionId device mismatch …`.
- [ ] No log row should be appended; admin-next should see no spurious output for Device 2.

### 7. Length cap

- [ ] Send an `envelope.sessionId` longer than `CC_PTY_SESSION_ID_MAX` (≥ 263 chars).
- [ ] cc-server should drop the event with `sessionId exceeds max length …`.

## Sign-off

- [ ] All cases above pass.
- [ ] Update `plan.md` Phase 4 to check off the manual regression line.
