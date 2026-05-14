# PTY session id breaking change

Canonical CC-plane PTY session identifiers (`ccpty:{deviceId}:{localSessionId}`) replace dual local/remote ids.

**Full plan, phases, and PR slices:** [plan.md](./plan.md).

**Protocol note (agent / gateway authors):** [pty-session-id-protocol.md](../cc/pty-session-id-protocol.md). **Test sweep:** [test-sweep.md](./test-sweep.md). **Manual regression checklist:** [manual-regression.md](./manual-regression.md).
