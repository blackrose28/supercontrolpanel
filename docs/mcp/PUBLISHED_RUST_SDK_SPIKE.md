# Published Rust MCP SDK / desktop HTTP spike

Checked 2026-11-13. This is a reproducible feasibility probe, not a replacement
for the working desktop HTTP listener or a claim of complete MCP compliance.

## Release and protocol decision

The official [versioning page](https://modelcontextprotocol.io/specification/versioning)
still identifies **2026-07-28** as current. The
[modern HTTP binding](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http)
uses per-request metadata and POST; the
[2025-11-25 binding](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
retains initialization and its separate session/stream lifecycle. These remain
separate test cases, not one invented hybrid protocol.

The official [rmcp 3.10.0 release](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.10.0)
was published November 12 and was the latest release when checked. `cargo info
rmcp@3.10.0` downloaded the registry package and reports Rust 1.88. The spike pins
`rmcp = "=3.10.0"`; its committed Cargo.lock records the registry checksum
`b5949e504d5ab9944a168259afc0c39ac0230627b6ec5367ace194c59bcbe163`.
No Git/main dependency is used. Axum resolves to 0.8.9; the official client's
reqwest backend resolves to 0.13.5 in this lock.

Inspection and execution use the **published package**, including its server
handler, client lifecycle, HTTP configuration, protocol-validation and discovery
fixtures, rather than assuming current upstream source was shipped. Tagged sources:
[client lifecycle](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.10.0/crates/rmcp/src/service/client.rs),
[HTTP hosting](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.10.0/crates/rmcp/src/transport/streamable_http_server/tower.rs),
[body parser](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.10.0/crates/rmcp/src/transport/common/server_side_http.rs).
The package has explicit support for modern discovery and legacy initialization.
This release is a viable **candidate** for the next adapter increment; the tests
below qualify only the exercised subset.

## Reproduce and build boundary

From the repository root:

```sh
cargo test --locked --manifest-path spikes/mcp-sdk/Cargo.toml
cargo clippy --locked --manifest-path spikes/mcp-sdk/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path spikes/mcp-sdk/Cargo.toml --check
cargo test --locked --manifest-path src-tauri/Cargo.toml --test mcp_sdk_dependency
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo tree --locked --manifest-path src-tauri/Cargo.toml -i rmcp -e features
cargo tree --locked --manifest-path src-tauri/Cargo.toml -i rmcp -e normal
```

`spikes/mcp-sdk` has its own workspace/lock. It is not a member of the headless
core/CLI workspace. Its library supplies one synthetic echo tool and one text
resource; it has no binary or persistent listener. Tests bind ephemeral IPv4
loopback sockets, use a public fixture bearer constant and shut down their hosts.
They never read app configuration/keyring, execute a desktop operation or launch
external programs. A test watchdog is not a production shutdown guarantee.

The desktop adds only a **dev-dependency** on the fixture library and a router
construction test. This compiles the pinned SDK server and Axum against the real
Tauri dependency graph, without changing production startup. Normal-edge reverse
inspection prints no rmcp dependents. Desktop reqwest remains 0.12.28 in the local
resolved graph; the 0.13 client feature is enabled only by the isolated spike's
dev-dependencies. No application reqwest upgrade or shared protocol types are
required for this hosting probe. The desktop lock remains ignored under the
repository's existing convention; the isolated spike lock is committed.

Validation here uses Linux and rustc 1.99.1. The declared SDK MSRV is not a test of
our entire graph on Rust 1.88/1.89. Windows/macOS builds, native desktop interaction
and IPv6 sockets were not exercised. No core/CLI dependency or executable changes.

## Observed compatibility matrix

Five compatibility HTTP integration tests run the actual published Rust SDK client over TCP,
plus raw HTTP probes. The desktop adds one dependency smoke test.

| Mode | Lifecycle and response selection | Observed |
| --- | --- | --- |
| 2026-07-28, JSON | Explicit `ClientLifecycleMode::Discover`, `server/discover`, per-request metadata; no session header | Discovery, tools/list+call, resources/list+read pass |
| 2026-07-28, SSE | Same lifecycle; JSON preference disabled | Same calls pass; raw response is `text/event-stream`, no session header |
| 2025-11-25, JSON | Explicit `Initialize`; legacy session mode disabled, JSON preference enabled | Same calls pass; raw initialize is JSON without a session header |
| 2025-11-25, SSE | Explicit `Initialize`; legacy session mode enabled | Same calls pass; raw initialize is SSE with a session header |

Tests assert the negotiated version and exactly tools/resources capabilities,
fixture input/output schemas, structured result plus text content, `isError` for
an operational failure, and protocol errors for absent/wrong-typed arguments and
unknown tools/resources. Modern tool results carry `resultType: complete`; the
SDK strips that discriminator for legacy clients. Modern list caching defaults
are zero TTL/private; the resource read fixture explicitly supplies those values.
This does not prove automatic validation of arbitrary output schemas.

Raw probes additionally check:

- Unsupported version: HTTP 400, code `-32022`.
- Header/body version mismatch: HTTP 400, code `-32020`.
- Unknown modern method: HTTP 404, code `-32601`.
- Modern GET and DELETE: HTTP 405, independent of the legacy session option.
- Missing/wrong bearer: HTTP 401 on POST, GET and DELETE, via fixture middleware.
- Invalid Origin and non-loopback Host: HTTP 403. Official-client requests with
  no Origin succeed. The SDK's default empty Origin allowlist **disables checks**;
  tests explicitly call `enforce_origin_validation()` to reject present Origins.
- Explicit 4096-byte body bound: a 4097-byte body is rejected with HTTP 413.
- Malformed JSON `{` with JSON content type: **HTTP 415/plain text**, beginning
  `fail to deserialize request body`, rather than a JSON-RPC parse-error envelope.
  Source inspection confirms this is published SDK behavior, not a missing request
  content type. The probe records it instead of rewriting the SDK parser.

The malformed-body behavior needs an explicit compatibility/redaction decision
before shipping. Native parser error text is not an app-level redaction boundary.
The synthetic bearer middleware is not production token loading, rotation, OAuth
or an authorization-policy implementation. Default SDK `serve` still uses legacy
initialization; modern clients must select the discovery lifecycle explicitly.
No Auto fallback or representative third-party application was qualified.

## Scoped adapter and hosting follow-ups

Keep `src-tauri/src/mcp/server.rs` and the helper's six custom REST routes working
while implementing a separate standard `/mcp` service. The current shared catalog
has 40 metadata records but no complete executable schema/handler registry. Its
five resource mappings delegate to the policy-first desktop tool dispatcher.
Existing Tauri/CC command and event contracts and custom envelopes must remain
unchanged; they are not standard tool results.

The next adapter should be a Tauri-independent crate with injected host handlers
and dedicated wire schemas, starting with a bounded read-only family. Reuse
`scp-core` policy/catalog/DTO contracts, but advertise only implemented host
capabilities and allowed operations. Translate successful payloads into standard
results with text/structured output; deliberately map operational failures versus
protocol errors, resource errors and caching fields. Fixture handlers validate
arguments manually; the SDK's default `get_tool` returns None and is not proof of
schema enforcement. Preserve permissive legacy parsing in the compatibility path.
Do not publish secret-bearing runtime settings as schemas/results.

Hosting can use `StreamableHttpService` under Axum on a Tauri-managed Tokio task.
Move blocking desktop operations off async workers; keep request cancellation
separate from accepted app-owned operations. Inject trusted local auth/policy,
enforce Origin and Host before dispatch, and preserve local-only binding and
redacted audits. Choose explicit body/concurrency/queue/time limits and token
rotation/shutdown semantics. The 4 KiB fixture limit is not a production policy.
Do not replace the current listener with this synthetic router.

## Controlled in-flight lifecycle evidence (2026-11-13)

Four additional tests in `tests/lifecycle.rs` use raw reqwest HTTP requests and a
synthetic gated handler. Compatibility tests above still use the official client.
The lifecycle handler reuses the fixture's exact supported versions/capabilities;
no SDK, lockfile or advertised protocol upgrade. Current official
[modern cancellation rules](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#cancellation)
and [legacy cancellation rules](https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/cancellation)
were rechecked. Tests execute registry-published 3.10.0 rather than treating those
rules or upstream comments as proof of implementation.

Each handler reports entry, exposes its `RequestContext.ct`, owns a drop guard,
and waits on an independently owned oneshot release. It deliberately ignores the
cancellation token. A separate release test proves the gate holds the request and
that releasing it produces the expected structured result. For the SSE disconnect
probe, the handler emits a progress notification and the client consumes it before
closing the stream (including any legacy priming event). A second request remains
gated during cancellation, then completes successfully with its token unaffected.

| Probe | 2026-07-28 | 2025-11-25 |
| --- | --- | --- |
| Close SSE response after progress | Request token cancels | Stateful session request token remains unset during 200 ms observation; explicit `notifications/cancelled` for its ID then cancels it |
| Disconnect before any response, JSON/stateless configuration | Request token cancels | Request token cancels; this is **not** the legacy stateful session case |
| Cancel configured server token with two handlers active before any response, JSON and SSE configurations | Both request tokens cancel; listener task joins while handlers remain gated | Same observed behavior, including stateful SSE session |
| Handler ignores cancellation | Remains alive during a 100 ms observation after cancellation; explicit release is required for fixture cleanup | Same |

Thus modern stream closure, legacy explicit cancellation, and legacy stateless
transport loss are distinct exercised paths. The negative observations are finite
windows, not a claim that a token/future can never change later. Cancellation does
not stop a non-cooperative future in these probes. In particular, joining the Axum
listener does **not** prove SDK handler tasks have joined. Tests explicitly release
every gate and observe each drop guard afterward, including after listener shutdown.
No native work or child operation is involved.

All listener tasks share test-only ownership with a five-second graceful-join
watchdog and cancel+abort fallback on drop. Network/event waits are also bounded
by five-second fixture watchdogs. Releasing/dropping the gate senders unblocks the
synthetic handler; these bounds are not production cancellation deadlines. The
lifecycle listener binds an ephemeral IPv4 loopback socket without the compatibility
fixture's bearer middleware; it serves only synthetic data. Existing bearer tests
remain separate. No production authentication or transport path changes.

TDD: after correcting the raw probe's missing modern `Mcp-Name` header, removing
the gate produced the expected failing entry/watchdog test; restoring the gate
passed. A separate RED rejects reuse of the private fixture type; GREEN exposes
that synthetic handler for test composition. Initial expectations that cancellation
would drop a non-cooperative handler were disproved and replaced with observations,
not SDK modifications. Shared listener ownership removes duplicate fixture cleanup.

Validation: all nine isolated HTTP tests pass; strict all-target spike Clippy and
spike formatting pass. The desktop dependency smoke test verifies the reused public
fixture against its dependency graph. No production desktop/core/CLI source changes,
native OS/minimum-Rust, UI/E2E, event-store resume, post-progress shutdown, request
queue saturation or hard shutdown bounds are qualified here.

## Next injected boundary

**Next focused increment:** build a testable injected host-handler adapter in this
isolated workspace for one read-only operation (`app_get_overview`), with a fake
host, shared fail-closed policy/catalog and public DTO/result mapping. Test denied
and unknown requests never enter the host, errors remain redacted, and request
cancellation reaches cooperative read work without affecting unrelated requests.
Keep the existing production listener functional; defer desktop wiring and
compatibility-route hosting until that adapter contract is reviewable.

The hosting owner must track/drain its own accepted work independently of Axum's
listener task, supply cancellation to cooperative request-scoped work, and enforce
admission/concurrency/queue limits before dispatch. Do not infer forced cancellation
of `spawn_blocking`, keyring/native calls or app-owned operations from this SDK.
For any later operation start, define when ownership transfers to app runtime,
status/operation IDs, loss-of-response and duplicate-start semantics explicitly;
request cancellation must not silently discard that ownership. The current tests
supply no production task registry, timeout policy or operation supervisor.

Backpressure/subscriptions, bounded blocking work, real operation/result schema
validation, token rotation, actual compatibility-route hosting, client setup UX,
third-party clients and native OS builds remain unqualified. No prompts, sampling,
elicitation, tasks or subscriptions are advertised by the fixture.

Coordinated CLI import, other operation contracts, stdio MCP and packaging remain
independently actionable. Linux operation-supervision/keyring/elevation gates are
unchanged; this spike does not rerun or resolve them.
