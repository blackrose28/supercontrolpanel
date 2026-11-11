# Desktop Streamable HTTP: Slice 0 evidence and decision

Date: 2026-11-11. Source baseline: `e9e843092`, with the documentation split already present in the working tree.
Status: operation inventory and isolated HTTP SDK/client feasibility verified on Linux. Native Windows/macOS desktop qualification, production integration, and release conformance remain open.

## Decision

Proceed with **published `rmcp = 3.9.0` and `axum = 0.8.8` as the tested candidates** for the next implementation slices. Both target revisions work with the tested official clients and Inspector CLI. Keep both revisions in scope, subject to the remaining application/platform gates. No production endpoint, handler, policy, dependency manifest, or settings were changed.

Enable **legacy sessions** in the HTTP adapter. Modern requests remain sessionless through the SDK's revision-specific routing. With legacy sessions disabled, cancelling a legacy request rejected the client promise but left server-side work active. With sessions enabled, the same check passes. Do not copy a stateless-only example into the desktop host or build custom cancellation routing around the SDK.

The operation matrix is [DESKTOP_STREAMABLE_HTTP_INVENTORY.md](DESKTOP_STREAMABLE_HTTP_INVENTORY.md): 40 existing tools, five existing resource mappings, all other registered desktop commands, known operation gaps, proposed sensitivity, current argument/result shapes, ownership, helper mappings, future CLI mappings, and acceptance IDs. An automated test checks that every current tool/resource and top-level desktop registration is accounted for. The individual application acceptance tests remain work for subsequent slices.

## Pinned components and build evidence

| Component | Exact evidence / configuration |
| --- | --- |
| Rust server SDK | Published `rmcp 3.9.0`; registry publication `2026-11-05T16:07:10.311069Z`; crate checksum `d24a7e92f4bcaa36b97fa93c4fa7ca8f3e669eced949345b3462dba333c7f909` |
| SDK features tested | SDK defaults (`base64`, `macros`, `server`) plus `transport-streamable-http-server`; no SDK OAuth/client/reqwest features enabled by this fixture |
| HTTP framework | `axum 0.8.8`, Tower service integration with `StreamableHttpService`; SDK `LocalSessionManager` for legacy sessions |
| Relevant resolved versions | Tokio `1.54.1`, Hyper `1.12.0`, HTTP `1.5.0`, Tower `0.5.3`, Schemars `1.2.3`; exact graph in fixture Cargo.lock |
| Official legacy client | `@modelcontextprotocol/sdk 1.32.2`, initialization-based lifecycle, negotiated `2025-11-25` |
| Official modern client | `@modelcontextprotocol/client 2.4.0`, version negotiation pinned to `2026-07-28` |
| Representative real client | `@modelcontextprotocol/inspector 2.14.0` CLI, explicitly selected `--protocol-era modern` and `legacy`; its locked client/core/server packages are `2.4.0` |
| Tested host | Linux x86_64; Rust `1.99.1`, Cargo `1.99.1`, Node `22.23.4`, npm `10.9.9`; system GTK `3.24.41` and WebKitGTK `2.52.6` |
| Declared MSRVs | rmcp/macros `1.88`, axum `1.78`; highest declared MSRV in isolated resolved graph is UUID `1.89.0`. Builds ran on 1.99.1, **not** an MSRV toolchain |
| Dependency locks | Independent [Cargo manifest](../../experiments/mcp-http-slice0/Cargo.toml), [Cargo.lock](../../experiments/mcp-http-slice0/Cargo.lock), [package.json](../../experiments/mcp-http-slice0/package.json), and [package-lock.json](../../experiments/mcp-http-slice0/package-lock.json). Root npm and desktop Cargo manifests/locks unchanged |

Published package evidence: [rmcp 3.9.0](https://crates.io/crates/rmcp/3.9.0), [tagged Rust SDK](https://github.com/modelcontextprotocol/rust-sdk/tree/rmcp-v3.9.0), [legacy TypeScript SDK](https://www.npmjs.com/package/@modelcontextprotocol/sdk/v/1.32.2), [modern client](https://www.npmjs.com/package/@modelcontextprotocol/client/v/2.4.0), and [Inspector](https://www.npmjs.com/package/@modelcontextprotocol/inspector/v/2.14.0). Published artifacts were downloaded/installed and executed; these are not assumptions based on upstream main.

A disposable copy of the desktop dependency manifest also resolved with both candidate dependencies. Starting from the **existing desktop lockfile**, resolution added 20 package/version entries and removed or replaced none. Existing Tauri `2.10.1`, tauri-build `2.5.4`, and reqwest `0.12.28` remain. Existing duplicate reqwest/Schemars major versions remain baseline concerns; the server feature does not require enabling SDK reqwest. The disposable dependency compile results are recorded below; this is not proof of integrated Tauri listener lifecycle behavior.

## Compatibility results

All network tests ran against a separate fixture on an ephemeral loopback port using the public fixture bearer. No VPN, SSH, transfer, settings mutation, keyring access, or CC enrollment ran.

| Client / revision | JSON preference | SSE preference | Discovery / tools / resource | Authentication and cancellation |
| --- | --- | --- | --- | --- |
| Official TS SDK 1.32.2 / 2025-11-25 | Pass; session calls return SSE | Pass; SSE | Initialize + initialized; list/call; structured result and `isError`; list/read resource | Bearer passed; session ID present; explicit cancellation releases server work |
| Official client 2.4.0 / 2026-07-28 | Pass; application/json | Pass; text/event-stream | server/discover, no initialize; version asserted; same tools/results/resources | Bearer passed; no session ID; stream cancellation releases server work |
| Inspector CLI 2.14.0 / legacy | Pass; session calls return SSE | Pass; SSE | tools/list, tools/call with exact structured value, resources/read with exact text | Explicit bearer and `--protocol-era legacy`; each CLI command reconnects successfully |
| Inspector CLI 2.14.0 / modern | Pass; application/json | Pass; SSE | Same three methods through modern discovery | Explicit bearer and `--protocol-era modern`; each command reconnects successfully |

Inspector is an actual shipped client executed as a separate CLI subprocess, not a hand-written HTTP mock. Both official clients are from the same ecosystem, so this is not a claim of independent client diversity. Claude Desktop, ChatGPT, Cursor, VS Code, and other user-facing integrations were not tested. Choose the intended product clients and add their exact-version smoke evidence before advertising their setup examples.

### Observed wire behavior and negative checks

The [captured TAP output](evidence/SLICE0_COMPAT.tap) includes per-request method/version/content-type/status traces and exact synthetic Inspector results. Tokens are omitted from traces. Highlights:

- Modern requests use `server/discover`, then `tools/list`, `tools/call`, `resources/list`, `resources/read`; the tested path sends no `initialize` and receives no session ID.
- Legacy sends `initialize`, then `notifications/initialized` (202), followed by session-bound operations. Legacy session responses use SSE even with the JSON preference.
- Missing bearer returns 401. A present untrusted Origin and an untrusted Host return 403; native requests without Origin succeed. Origin enforcement must be explicitly enabled: the SDK's empty-origin-list default otherwise disables validation.
- Unsupported version `2099-01-01` returns HTTP 400 / `-32022` and the exact two supported revisions. A mismatched tool routing header returns HTTP 400 / `-32020`. Wrong-type tool input returns JSON-RPC `-32602`; a synthetic operational denial uses tool `isError: true`.
- Cancellation tests first observe a server active-work count of 1, abort the client request, then assert the count returns to 0 within a bounded polling window. This detects orphaned work instead of checking only a rejected promise.

The modern and legacy transport rules differ; see [modern HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http) and [legacy HTTP](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports). These fixtures establish the tested subset, not full protocol conformance, durable event delivery, or application policy correctness.

## Reproduction and TDD record

See the [fixture README](../../experiments/mcp-http-slice0/README.md) and [integration tests](../../experiments/mcp-http-slice0/tests/compat.test.mjs). The crate is separate from the desktop and is not a production implementation. `npm ci --ignore-scripts` successfully installs its pinned Inspector CLI without running dependency install scripts.

RED: the initial default SDK handler failed the expected identity/tool/resource assertions (`rmcp` rather than `slice0-fixture`, empty tools). Tests then drove the fixture's typed tool/resource responses. The cancellation extension first failed on the missing active-work resource; after adding instrumentation, the legacy stateless case failed with active work still 1 after cancellation. Enabling SDK-managed legacy sessions made the same cancellation assertion pass. Modern cancellation also passes. GREEN: all 13 integration checks pass; the recorded output is linked above.

The synthetic fixture denial only checks protocol error representation. It cannot stand in for Slice 1's real application policy, protected-config, secret-redaction, or write-gate tests. Application acceptance IDs in the inventory are explicitly pending.

## Native feasibility and baseline checks

| Check | Result / limitation |
| --- | --- |
| Isolated Linux fixture `cargo build --locked` | Pass; real Linux binary used by clients |
| Isolated Windows `cargo check --target x86_64-pc-windows-gnu` | Pass; cross-target type checking only, no Windows executable linking or runtime test |
| Combined desktop dependencies, clean resolve, Linux `cargo check` | Pass, including Tauri/AppHandle and SDK/Axum references in a disposable package; no production source integration |
| Combined dependencies starting from existing desktop lockfile | Linux `cargo check --locked` passes with 20 package additions and no replacements; disposable manifest plus Tauri/AppHandle and SDK/Axum references |
| Fixture `cargo clippy --locked -- -D warnings` | Pass |
| Existing desktop `cargo test --manifest-path src-tauri/Cargo.toml` | Pass: 101 tests; helper binary contains no tests |
| Existing frontend `npm run build` | Pass |
| Existing `npx playwright test` | Baseline: 32 passed, 15 skipped, 4 failed because Firefox/WebKit executables are not installed. Failures are the two example tests under each missing browser. No application source changed |
| macOS native build | Not run; no macOS host/SDK available here |
| Native Windows desktop build and smoke | Not run on this Linux host; cross-target fixture check is insufficient |
| Keyring/elevation/browser smoke | Not exercised by this fixture; retain release gates |

Ripwire quality-delta reports no gated regressions; its new-symbol dead-code warnings concern SDK trait callbacks, the Drop hook, and the test fetch callback invoked dynamically. The static test gate reports documentation symbols as untested and does not model the subprocess integration boundary; the 13 executable Node tests are the runtime evidence for that boundary. No production symbol was edited.

Before calling this release-ready, run native Windows/macOS builds, resolve missing-browser baseline coverage, add intended end-user clients, and validate the integrated Tauri shutdown/reconfigure and operation-ownership paths. These remaining gates are explicit; neither target revision is being declared unsupported merely because native GUI qualification is unavailable here.

## Slice 1 handoff

1. Use the inventory as the review boundary: safe public DTOs, fail-closed policy, sensitivity review, protected config/CC fields, and error/redaction contracts first.
2. Retain `legacy_session_mode(true)` for explicit legacy cancellation. Keep modern JSON/SSE behavior under SDK routing. Set allowed listener authority and call `enforce_origin_validation()` for the default no-browser-origin policy.
3. Treat protocol sessions as transport state only. Application sessions/PTYs/transfers remain desktop-owned after acceptance. The fixture's per-request cancellation counter is not a design for stopping accepted VPNs or tunnels.
4. Characterize shared HTTP/resource/Tauri dispatch contracts before changing production handlers. Review the helper's ignored `hidden` flag, dimension truncation, raw-store status filtering, incomplete sensitivity list, and CC-token redaction findings.
5. Add the production SDK/framework dependencies only in the implementation change, with the approved toolchain/lockfile policy. Do not copy the fixture's public bearer, simplistic validation, synthetic policy, or process shutdown into production.
