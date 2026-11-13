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
resource, plus a separate injected overview adapter described below; it has no
binary or persistent listener. Tests bind ephemeral IPv4
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
and IPv6 sockets were not exercised. No SDK dependency is added to core/CLI, and no executable changes.

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

## Injected overview boundary (2026-11-13)

`src/overview.rs` adds `OverviewAdapter<H>` and the `OverviewHost` async read seam
inside the isolated spike only. The adapter depends on `scp-core` without optional
features to reuse the exact `McpAppOverviewResult`, catalog and independent
fail-closed policy. This justifies the spike-to-core edge and its lockfile additions
(including the core's existing PTY dependencies); core and CLI gain no SDK edge.
`tokio-util` moves from dev-only to normal spike dependency to name the SDK request
cancellation token at the host seam. The registry pin remains exactly 3.10.0.
The existing echo/resource router and production custom listener are unchanged.

The host supplies immutable `OverviewPolicy` settings independently of arguments
or request metadata. Discovery filters the shared catalog to the one implemented,
permitted operation; no resources capability or other catalog tool is advertised.
Dispatch checks shared policy first, then the implemented-operation allowlist,
then manually validates this operation's empty arguments. Omitted arguments are
accepted; nonempty objects fail. The dedicated input schema is an empty object
with `additionalProperties: false`. This standard adapter is deliberately stricter
than the existing custom dispatcher's ignored arguments; that compatibility path
is unchanged. This is not a generic schema validator; SDK deserialization/parser
failures, including its previously recorded raw error text, remain outside this
host-error boundary.

The output schema requires `appVersion` (string), `activeFunctions` and
`knownFunctionCount` (nonnegative integers), with no additional properties.
A host constructs the shared DTO from its own version/status snapshot; the adapter
serializes it directly into structured content and the SDK's JSON text content.
It adds no custom five-field envelope. Tests check modern `resultType: complete`
and its absence in legacy responses. No automatic arbitrary output validation is
claimed. Returned host failure strings are discarded without logging: the public
result has `isError: true`, text `App overview unavailable`, and no structured data.
Policy/unknown/unimplemented/argument errors are protocol `-32602` errors with fixed
messages and no caller data; there is no host panic/native error redaction claim.

Three raw-HTTP integration tests exercise discovery/initialization, tools/list and
tools/call against the injected adapter for both exact protocol revisions. Fake
host reads report entry and await individual release gates. Denied categories,
unknown policy/category, unknown names, a known but unimplemented tool, and extra
arguments all return before host entry. Success maps the public DTO exactly;
secret-bearing fake host errors return only fixed public text. All exposed adapter
capabilities are tools-only. Existing five official-client compatibility tests and
four lifecycle tests remain separate and continue exercising the original fixture.

For cancellation, two JSON/stateless requests enter independently for each
revision. Aborting and joining the first client request cancels the token received
by that host read; the cooperative fake observes it, exits its select and reports
completion. The other token stays unset at the check and its read completes
successfully after explicit release. This proves token propagation and cooperative
fake read completion, not hard shutdown/native work cleanup or accepted background
operation cancellation. It does **not** qualify legacy stateful SSE cancellation
for this new adapter. As in the earlier lifecycle tests, each listener binds only
an ephemeral IPv4 loopback socket, has no bearer middleware and uses synthetic data;
five-second watchdogs bound fixture waits, not production latency or cleanup.

TDD: initial compile RED reports only the missing core dependency/overview module.
After GREEN, separate deliberate mutations remove dispatch authorization, expose
the raw host failure, and replace the request token with an unrelated token. Each
fails its focused test (entry watchdog, exact redacted message, cancellation
watchdog respectively); all mutations are restored. Refactoring reuses the exact
fixture protocol-version declaration and separates result assertions. Short SDK
capability declarations remain independent because the fixture advertises resources
and the adapter must not.

Validation: the locked isolated suite passes all 12 tests (3 adapter, 5 existing
HTTP, 4 lifecycle); strict all-target spike Clippy, formatting and staged whitespace
checks pass. The locked desktop dependency smoke passes with its 18 existing bin
warnings. The ignored desktop lock first needed an offline metadata refresh for
the new spike dependency; the final locked run succeeds. The desktop normal-edge
reverse graph has no SDK dependents; the root workspace has no rmcp package at all.
All preexisting spike packages retain their version/source/checksum, including
rmcp 3.10.0; 30 packages are added by the core dependency. Full desktop/root CLI,
minimum Rust, native OS and UI/E2E suites are not rerun for this isolated adapter.

Ripwire impact/uses identify only the new adapter and its HTTP probe; external
`ServerHandler` is not indexed, and affected selection has zero proven tests but
lists adapter test construction as a candidate. Runtime integration evidence above
covers that missing dispatch edge. New `call_tool` edit-check reports no incompatible
callers. Final quality exits 2: one false unreachable finding on the unchanged,
executed `Fixture::list_tools` after same-named trait implementations were added,
and one 27-token similarity finding on the short capability constructors. Those
constructors deliberately differ (fixture tools/resources versus adapter tools-only);
sharing the fixture's capability value would advertise unsupported resources.
Seven new non-gating reachability findings cover SDK-dispatched methods/test entry
points. No suppressions/acknowledgments are added. The staged test gate exits 4:
three executed test files have unknown runners and two documentation symbols are
unmodeled; ten unlocated hunks fall back to seven file-granular seeds. These are
static-analysis limits/reviewed short similarity, not a claim of a clean static gate.

The following increment qualifies the official-client adapter matrix and cooperative
SSE cancellation. **Next focused increment:** establish an explicit malformed-input /
parser-error redaction boundary in the isolated spike before shipping; returned host
error redaction does not cover SDK parsing. Desktop wiring/auth/token rotation,
host-owned work tracking/drain, admission/concurrency/queue limits, compatibility
routes, real runtime/keyring calls and third-party/native OS clients remain later.
A host must define accepted-operation ownership and response-loss/duplicate-start
semantics before adding operation starts; request cancellation alone is insufficient.

## Official-client overview qualification (2026-11-13)

`tests/adapter/official_client.rs` is a child module of the adapter integration test,
reusing its cooperative fake host, DTO fixture and result assertions. Production
sources, dependency versions, lockfiles and the `OverviewHost` seam are unchanged.
The published package's `service.rs` (`RequestHandle::cancel`) and
`transport/streamable_http_client.rs` (`Event::ControlMessage`) were inspected
before choosing cancellation APIs. There is no cancel-on-drop assumption.

| Exact revision | Response configuration | Adapter observations |
| --- | --- | --- |
| 2026-07-28 | JSON | Discover, tools-only capabilities, filtered schemas, DTO/text/resultType, rejections and returned-host-error redaction pass |
| 2026-07-28 | SSE | Same contract checks; cancellation after matching client progress reaches cooperative work, unrelated read completes |
| 2025-11-25 | JSON, stateless | Initialize; same contract checks with legacy result mapping |
| 2025-11-25 | SSE, stateful | Same contract checks; explicit cancellation with recorded session and request ID reaches cooperative work, unrelated read completes |

The allowed catalog has exactly `app_get_overview`, its complete input/output
schemas, and tools-only capabilities. Omitted and empty arguments succeed. Empty
categories, unknown policy/category, unknown and known-but-unimplemented names,
and extra arguments return protocol `-32602` without raw input/error data and
before fake host entry. A secret-bearing returned host error becomes exactly
`App overview unavailable`, `isError: true`, with no structured content. This
qualifies returned strings only, not native errors, panics, parser errors or
success-value redaction.

A test-only `ServerHandler` wrapper sends progress through the real context peer
before delegating to `OverviewAdapter::call_tool` with that context. It does not
add progress to the adapter or host seam. The official client's `on_progress`
callback reports the decoded token/value to the test; matching progress is consumed
before cancellation, not merely a socket or priming event. Both reads have entered
and both progress tokens are observed before the first request is cancelled.
For legacy SSE, the request recorder verifies session headers on both tool calls
and one `notifications/cancelled` POST with the first request's exact ID. For modern
HTTP, the inspected worker intercepts that API's control message and closes the
request SSE stream; the recorder snapshot contains no cancellation POST. The fake
observes its token and reports completion. The other token is unset at the check,
then that read reports completion and returns the expected DTO after release.
There is no indefinite negative-window, native cleanup, hard deadline, or accepted
background operation cancellation claim.

The recorder buffers only these small synthetic test request bodies (64 KiB test
limit) and forwards them unchanged; it is not a production parser or error boundary.
Listeners use ephemeral IPv4 loopback without bearer middleware, as in the existing
adapter probes. Five-second waits and listener cleanup remain fixture watchdogs.

TDD: the three tests first fail compilation solely for the missing `OfficialProbe`.
Implementing the test wrapper/client/recorder makes them pass. Behavioral mutation
REDs then independently suppress streamed progress, replace the adapter's request
token, and expose the raw host error: the first two hit the relevant observation
watchdog; the third fails the exact public-content assertion. All mutations are
restored. Refactoring extracts catalog/rejection assertions and the wire recorder.
The final locked spike suite passes **15 tests** (6 adapter including these three,
5 HTTP, 4 lifecycle); strict all-target spike Clippy, formatting and whitespace
checks pass. No production source or dependency changes require a desktop/root
suite rerun here; full desktop/root/native OS/MSRV tests are not rerun.

Ripwire impact/uses/affected still cannot prove the external SDK dispatch edge;
affected lists two adapter constructions as candidates and zero proven tests.
Source review and actual HTTP/mutation tests cover that edge. The new wrapper
edit-check reports no incompatible callers, but its same-named client caller is
approximate. Final quality exits 0 with no gating/preexisting regressions and six
new non-gating reachability findings on executed test/trait entry points. Initial
nesting, complexity and length findings are removed by the test-only refactor;
no suppressions are added. Test gate exits 4: four executed test files/modules have
unknown runners, the unchanged adapter dispatch and two documentation symbols are
unmodeled; seven unlocated hunks use five file-scope seeds (122 symbols, 11 impacted).
The official-client child module executes within `--test adapter`, not as a separate
Cargo integration target. This is not a clean static test-gate claim.

Next: characterize secret-bearing SDK parse/deserialization failures and implement
an explicit fixed-message error boundary in the isolated spike, preserving valid
modern/legacy JSON/SSE behavior. Authentication, production desktop wiring and
compatibility routes, runtime/keyring work, host admission/drain and accepted-operation
ownership stay separate. Other CLI contracts/import, stdio and packaging can proceed
independently; the unchanged cgroup qualification is not repeated.

## Isolated HTTP-status error boundary (2026-11-13)

Published registry source was inspected first: `common/server_side_http.rs`
`expect_json` bounds and collects the request, returns 413 on overflow, 500 with
body-read error text, and 415 with serde text on deserialization failure. These
parser branches do not log that error there. `internal_error_response` separately
formats native error text into HTTP 500 **and logs it**. `StreamableHttpService`
validates Accept/Content-Type before parsing; protocol/header/session checks and
handler dispatch follow. `model.rs` uses an untagged message wrapper; request enum
fallback can deserialize an invalid known method as `CustomRequest`. The default
`ServerHandler::on_custom_request` uses the method itself as the `-32601` message.
Returned `OverviewHost` strings never cover these SDK paths.

`tests/error_boundary.rs` uses synthetic markers only, ephemeral loopback and
five-second fixture watchdogs, with the original bearer fixture and explicit Origin
validation. The raw comparison uses JSON/stateless configuration for both revisions,
not an initialized legacy session. `Mcp-Method` matches the body and tool calls carry
`Mcp-Name: spike_echo`; modern request metadata uses the SDK's client-context builder.
No raw body/arguments/native diagnostic is printed or logged by this test/boundary.
The original raw `http.rs`, lifecycle fixture and raw overview probes are unchanged.

| Synthetic input | Native 2026-07-28 | Native 2025-11-25, stateless | Marker observed in native response |
| --- | --- | --- | --- |
| Truncated JSON; string/array root; invalid jsonrpc; object ID/method | 415 text beginning `fail to deserialize request body`, no JSON-RPC code | Same | No, for these cases only |
| tools/call arguments is a string; initialize capabilities is a string | 404 / `-32601` | 200 / `-32601` | No; typed request falls through to custom handling |
| tools/call arguments null, extra synthetic secret field | 200 / `-32602` | Same | No; null becomes omitted arguments, fixture rejects missing message |
| Unknown method containing marker | 404 / `-32601` | 200 / `-32601` | **Yes**, method is the native error message |
| Client result with unknown action string | 400 / `-32600` | Same | No; not evidence of enum validation: an empty-result fallback can accept the shape before the server rejects the response message |

`src/http_errors.rs::with_fixed_http_errors` is a separately named, opt-in hosting
layer in the isolated spike. It examines **only the HTTP status**. All HTTP 4xx/5xx
bodies become exact `MCP HTTP request failed` with `text/plain; charset=utf-8`,
no JSON-RPC envelope/code/data or reflected ID. Status is retained, including 415
for malformed envelopes; this deliberately does not turn it into `-32700`.
All error headers are discarded except `Allow` matching exactly `POST`, `GET, POST`
or `GET, POST, DELETE`, preserving known SDK method guidance without forwarding
arbitrary header text. This also replaces native JSON-RPC bodies on HTTP 400/404;
it is a scoped experimental transport choice, not final standard protocol-error
compatibility. The table's native 200 responses pass through unchanged, including
legacy unknown-method reflection. The layer must not be described as a complete
protocol-error, native-error, success-value, panic or logging redactor.

Mounting outside fixture auth/admission retains wrong-token 401, Origin/Host
403, bad Accept 406 and Content-Type 415 before a deliberately oversized malformed
body; authorized otherwise-valid admission reaches the existing 4096-byte limit
and returns 413. Modern GET remains 405 with `Allow: POST`. These admission probes
use the modern revision. They prove status/precedence preservation, not concurrency
limits, production token policy or work admission/drain. The boundary never polls
request or response bodies; it adds no request buffer or size policy. Successful
200/202/204 headers/bodies pass unchanged. A synthetic HTTP 500 with marker-bearing
body/header verifies replacement, not a real native/session failure or SDK logging
redaction. No SDK tracing subscriber is installed by the fixture. The SDK still has
raw message trace sites and header/error warning paths: production logging requires
its own reviewed policy, independently of this response layer.

The existing official-client overview harness explicitly mounts the layer now.
The exact modern/legacy JSON/SSE catalog/schema/DTO/policy/returned-host-error tests
still pass. Progress is decoded before cancellation; cooperative fake completion
and the unrelated successful read still pass for both SSE revisions, including
initialized stateful legacy. This exercises successful streaming through the layer,
not malformed-input coverage in every stateful/session mode. Progress remains in
the test wrapper only; `OverviewHost` and `OverviewAdapter` are unchanged.

TDD: first compile RED is solely the absent `http_errors` module. Initial GREEN
passes native/fixed comparisons; the admission fixture is corrected to replace,
not append, its existing Authorization header. A separate 405 `Allow` preservation
test fails before the safe-header rule is implemented. A behavioral mutation that
bypasses replacement fails on native parser text versus exact fixed text. Another
mutation buffers responses before classifying status and fails the official SSE
progress watchdog. Both mutations are restored. Refactoring uses the SDK metadata
builder, explicit fixture headers/config and one synthetic-response helper; it
leaves the original native SDK fixtures untouched.

Validation: final locked spike suite passes **20 tests** (6 adapter, 5 error boundary,
5 original HTTP, 4 lifecycle); strict all-target spike Clippy, formatting and
whitespace pass. Dependencies/lockfiles and production sources are unchanged;
root/desktop/native OS/MSRV and UI/E2E suites are not rerun for this isolated layer.

Ripwire impact/uses on the changed official-client start find its three executed
matrix/cancellation tests; affected selection for the layer names the official child
module and new boundary target (unknown runners). New layer edit-check and unchanged
start signature have zero incompatible callers. Final quality exits 0: zero gates
or preexisting regressions, six new non-gating reachability findings on executed
test entries and the message builder. Initial cloned fixture setup/metadata and
response-test duplication are removed using the SDK builder and focused helpers;
no suppressions are added. Staged test gate exits 4: three executed test files/modules
have unknown runners; two documentation symbols and the SDK-dispatched unchanged
supported-version method are unmodeled. Seven unlocated hunks use seven file-scope
seeds (136 symbols, 12 impacted). This is not a clean static test-gate claim.

Next: add a fixed-message protocol-level custom/unknown-request boundary in the
injected adapter, retaining error codes/statuses and covering malformed-known-method
fallback plus legacy initialized SSE. Do not solve HTTP 200 errors by buffering SSE.
Logging and other native failure policies remain shipping gates; CLI import, other
operations, stdio MCP and packaging remain independently actionable.
