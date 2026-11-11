# Desktop HTTP MCP Slice 2: first vertical protocol slice

Date: 2026-11-11. Standard MCP now runs at exact `/mcp` on the existing disabled-by-default loopback listener. The REST helper routes, Tauri commands, device-local settings keys, function/PTY ownership, and status DTO fields remain intact. Active `implementationStage` changes to `http-streamable-slice2`; `transport` stays `http`.

## Delivered boundary

Pinned production dependencies are `rmcp = 3.9.0` (defaults plus `transport-streamable-http-server`) and `axum = 0.8.8`, matching Slice 0. Local desktop lock resolution adds 20 packages and replaces none. The repository intentionally ignores `src-tauri/Cargo.lock`; that policy is retained, so the two direct version pins are committed but the full desktop dependency graph is not frozen by this change. The tested Linux toolchain is Rust/Cargo 1.99.1; this is not an MSRV test.

`protocol.rs` owns SDK adaptation; `server.rs` supplies desktop host closures. Only `function_status_list` and its registry resource `scp://status/functions` are implemented at the standard endpoint. Other tools/resources remain unadvertised and fail explicitly. The existing policy/category registry, input/output validators, backup gate, desktop dispatch and safe status DTO remain the authority. Neither protocol sessions nor client names confer permissions. Discovery and execution recheck current settings.

The standard input is an empty object. Tool discovery includes its input schema and output schema `{ "statuses": [SafeFunctionStatus] }`; successful calls supply this object as structured content and equivalent text. The matching resource returns the same JSON text with `application/json`. Legacy envelopes retain the array in `data`. Unknown tools/arguments use SDK invalid-params errors; resource failures use SDK resource errors; valid tool failures return `isError: true` and a stable safe application code/message. No raw runtime diagnostic is returned.

The SDK handles revision-specific discovery/lifecycle, JSON/SSE, and legacy sessions. All routes share bearer authentication, deny all browser Origins, and validate loopback Host and any absolute request authority plus configured port, including bracketed IPv6. Native clients may omit Origin. Bodies are capped at 1 MiB. Legacy authentication failures retain envelope fields. This remains a local static-bearer deployment, with no general remote OAuth claim.

Blocking desktop reads and legacy dispatch run on Tokio blocking workers. Listener stop drops the transport runtime and bounds runtime shutdown to five seconds; it does not stop desktop-owned PTYs. Full stream/session revocation, request/worker capacity limits, receive deadlines, event subscriptions, cancellation ownership and backup/reconfigure stress qualification remain Slice 4. No mutation is enabled through the standard endpoint.

## Reproduction and TDD

Install the pinned official clients using `npm ci --ignore-scripts --prefix experiments/mcp-http-slice0`. Build the helper before the integration suite:

```sh
cargo build --manifest-path src-tauri/Cargo.toml --bin supercontrolpanelhelper
cargo test --locked --manifest-path src-tauri/Cargo.toml
npm run build
npx playwright test
```

The two inline Rust integration tests start the actual production router on ephemeral loopback ports with an injected host that executes the real contract validator/policy against synthetic status data. They run `experiments/mcp-http-slice0/tests/desktop.test.mjs` as a bounded Node subprocess and execute the compiled helper. No desktop settings, credentials, VPN, SSH or keyring are used. These tests cover the adapter/router boundary; they do not claim native desktop host smoke coverage.

RED: the new behavioral test failed because `ProtocolHost`, the router and SDK dependency were absent. The implemented adapter passed both official clients' discovery/query/resource checks but the REST compatibility assertion failed: nesting the SDK intercepted `/mcp/tools/call`. Mounting it at exact `/mcp` preserves REST dispatch. A later authority test reproduced an untrusted absolute URI being accepted with a valid Host (200); checking both authority and Host rejects it with 403. The suite checks JSON and SSE response modes independently, schemas, structured/text equivalence, unknown tools/resources, malformed arguments, unsupported revisions, auth on every route, Origin/Host rejection, oversized bodies, actual helper execution, and live policy denial on established clients.

## Verification and remaining gates

Validation results are recorded below. Native Windows/macOS builds, GUI/native smoke tests, intended end-user client setup, complete lifecycle/cancellation and full revision conformance remain release gates. Slice 0 Inspector evidence is unchanged; Slice 2 uses the pinned official legacy SDK 1.32.2 and modern client 2.4.0, not a new Inspector or product-client qualification claim.

Final Linux validation:

- `cargo test --locked --manifest-path src-tauri/Cargo.toml`: **114 passed**, including both official-client integration modes; helper unit target compiled (0 tests).
- Compiled helper `function list`: passed against the compatibility router in both integration modes.
- `npm run build`: passed.
- `npx playwright test`: **36 passed, 15 existing skips**.
- `git diff --check`: passed.
- Ripwire impact/uses/affected and edit-check: listener callers include startup, enrollment, settings saves and MCP commands; no incompatible signatures. Quality delta has zero gated regressions after two narrowly recorded acknowledgements for SDK trait boilerplate shared with the isolated spike. The test graph gate still reports 19 untested transitive symbols and one indexed test; it does not fully resolve the inline Rust/subprocess boundary. The executed tests above are the runtime evidence, not a claim that this static gate passes.
