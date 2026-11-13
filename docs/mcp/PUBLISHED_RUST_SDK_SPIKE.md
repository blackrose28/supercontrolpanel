# Published Rust MCP SDK / HTTP and stdio spike

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
resource, plus a separate injected overview adapter described below. HTTP tests
bind ephemeral IPv4 loopback sockets, use a public fixture bearer constant and
shut down their hosts. The synthetic `stdio-fixture` binary added below hosts the
same adapter on subprocess pipes. Tests never read app configuration/keyring or
execute a desktop operation; stdio tests launch only that synthetic binary. A test
watchdog is not a production shutdown guarantee.

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
and IPv6 sockets were not exercised. No SDK dependency is added to core/CLI, and
no production executable changes.

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


## Isolated custom-request protocol boundary (2026-11-13)

Source inspected before implementation: published rmcp 3.10.0
`handler/server.rs::ServerHandler::on_custom_request` returns `METHOD_NOT_FOUND`
with the untrusted method as message. Its handler dispatch sends
`ClientRequest::CustomRequest` directly to that hook. `model.rs::ts_union` derives
an untagged enum; `ClientRequest` ends with `CustomRequest`, which preserves raw
method/params. Some invalid typed known-method payloads therefore never reach
`OverviewAdapter::call_tool`. This is native fallback behavior, not generic argument
validation. In `streamable_http_server/tower.rs`, negotiated direct handling maps
`METHOD_NOT_FOUND` to HTTP 404 and sends terminal errors as JSON even when configured
for SSE. Legacy direct/session paths bypass this status mapper; initialized session
requests use request-wise SSE with HTTP 200.

The adapter now overrides only `on_custom_request`: `ErrorData::new` with native
`METHOD_NOT_FOUND` (`-32601`), exact `MCP method not supported`, and `None` data.
Method, params and context are ignored; no host call, error-string formatting,
body polling or logging is added. This intentionally retains `-32601` for malformed
known-method fallback instead of asserting all invalid arguments produce `-32602`.
The host seam and all existing operation validation remain unchanged. This is not
redaction for parser/header/session failures, SDK tracing, native failures, panics
or successful output values. The SDK remains exactly 3.10.0.

Hosting is an explicit test choice. New `tests/adapter/protocol.rs` raw probes mount
the adapter directly, and its official probes use identity hosting (no HTTP-status
replacement). The test-only progress wrapper delegates custom handling to the actual
adapter. Existing three official-client tests explicitly choose
`with_fixed_http_errors`; their successful JSON/SSE, decoded progress, explicit
cancellation and concurrent read paths retain the prior composition. The new
protocol tests do **not** claim that composition preserves modern error envelopes:
mounting the HTTP-status experiment over these requests still replaces modern 404
with plain text and removes its JSON-RPC code/envelope. Legacy 200 errors pass that
layer unchanged. Neither hosting choice establishes a final production error policy.
Original raw `http.rs`, `lifecycle.rs`, `error_boundary.rs`, `fixture_router`, and
raw overview probes remain unchanged, preserving the native reflection evidence.

| New synthetic custom/fallback case | 2026-07-28, JSON or SSE configured | 2025-11-25 stateless JSON | 2025-11-25 initialized stateful SSE |
| --- | --- | --- | --- |
| Unknown secret method and secret params | 404 JSON / -32601 | 200 JSON / -32601 | 200 SSE / -32601 |
| tools/call string arguments | Same | Same | Same |
| tools/call object name | Same | Same | Same |
| initialize string capabilities, after valid lifecycle | Same | Same | Same |

All errors have only fixed code/message, no error data or raw arguments/native text.
The raw matrix additionally deserializes every case as `ClientRequest::CustomRequest`,
asserts response status/content type, JSON-RPC 2.0 and exact request ID, and checks
host non-entry. Modern metadata uses the SDK client-context builder; method headers
match the body and string tool names carry matching `Mcp-Name`. The legacy SSE probe
asserts a session ID from real initialize, posts initialized with that ID, and sends
subsequent requests with it. Its small synthetic SSE decoder skips native empty
priming, consumes only to a terminal event, and checks a 64 KiB pending-frame bound;
it never collects an entire live SSE response. This is test-only parsing, not a
production framing implementation. The official matrix requires decoded
`ServiceError::McpError` with the same fixed error, rather than accepting transport
failure. Both matrices verify discovery remains usable after rejection. Fixtures
use ephemeral IPv4 loopback, five-second waits and existing listener cleanup without
bearer middleware; they do not qualify production authentication or native cleanup.

TDD: after correcting a test-only SDK metadata builder call, both tests fail against
the default hook because the error message reflects the synthetic secret method.
The override makes both pass. Raw fixture corrections supply required modern tool
name headers and skip the native legacy SSE empty priming event; no implementation
or expected native status/code changes are needed. Refactoring separates terminal
SSE event parsing from stream framing and makes hosting explicit at every official
probe call site. No synthetic progress is added to the adapter or host.

Validation: locked isolated suite **22 pass** (8 adapter, 5 boundary, 5 HTTP,
4 lifecycle), strict all-target Clippy, formatting and diff whitespace checks pass.
No dependency/lockfile or production desktop/core/CLI source changes; root/desktop,
native OS/MSRV and UI suites are not rerun for this isolated adapter increment.

Ripwire impact/uses before editing identify the adapter's raw/official test hosts
and three existing official start callers. SDK trait dispatch is approximate;
affected selection for the new hook resolves no transitive test edge but names
three adapter construction sites. Edit-check reports the new hook and the intentional
four-to-five-argument test start contract with zero incompatible callers; all four
call sites explicitly supply hosting. Final quality exits 2 with one verbosity gate
on that test start (60 → 61 lines from the explicit hosting parameter), four minor
churn rows, and four new non-gating reachability findings for the exercised hooks
and test entries. This small test-only growth is retained without suppressions;
initial wrapper-clone and decoder complexity/nesting findings were removed by the
focused refactors above. Staged test gate exits 4: three executed adapter files/
modules have unknown runners and two documentation symbols are unmodeled; eight
unlocated hunks use six file-scope seeds (133 symbols, six impacted). This is not a
clean static coverage or quality-gate claim. Executed Cargo tests provide the
runtime dispatch evidence.

Next: characterize pre-dispatch protocol/header negotiation errors with synthetic
secret markers and preserve native standards status/code when adding a separately
reviewed scoped mapping. Keep original raw evidence and successful SSE unbuffered;
this custom hook cannot sanitize errors emitted before dispatch. SDK logging and
native failure policy remain shipping decisions. Independent coordinated CLI import,
other operation contracts, stdio MCP and packaging remain actionable.

## Isolated pre-dispatch negotiation boundary (2026-11-13)

Inspected published `tower.rs` protocol-version/header validators, standard-header
validation, `serve_negotiated_request_directly` and `jsonrpc_http_status`, plus
`common/server_side_http.rs` and `service/server.rs` inline negotiation. Header/body
consistency is checked before hook dispatch. The inline service rejects unsupported
versions and missing required metadata before calling the adapter; negotiated HTTP
handling returns terminal JSON for these non-200 errors even with SSE configured.
Initialize header/body mismatch uses invalid-request `-32600`, not header-mismatch.
No SDK/dependency/lockfile or production desktop/core/CLI changes are needed.

New `tests/adapter/negotiation.rs` is nested under the existing protocol module.
Paired native/mapped raw matrices run each of eight cases after real lifecycle setup
in both revisions and JSON/SSE configurations. Legacy SSE requires an initialize
session header and initialized notification, reusing the existing probe. Every case
returns HTTP 400/application-json with ID 42 and no host entry:

| Case | Native code | Synthetic marker in native response |
| --- | --- | --- |
| Legacy header/body metadata version mismatch | -32020 | Message |
| Modern header/body metadata version mismatch | -32020 | Message |
| Matching unsupported header + metadata version | -32022 | Error data (`requested`) |
| Modern Mcp-Method mismatch | -32020 | Message |
| Modern Mcp-Name mismatch | -32020 | Message |
| Missing required modern metadata | -32602 | No |
| Incomplete required modern metadata | -32602 | No |
| Initialize header/body version mismatch | -32600 | Message |

Raw fixtures from previous increments remain untouched. Native probes continue
asserting native reflection rather than passing through the new mapper. Catalog
requests after the failures still work and stream when configured for SSE.

The new separately named `negotiation_errors::with_fixed_negotiation_errors` is an
**opt-in experiment immediately outside the SDK route**. It examines only HTTP 400,
exact `application/json`, exact-size response bodies at most 65536 bytes. The pinned
SDK uses finite `Full`/infallible bodies for the qualified paths. It bounded-collects
only those candidates, validates a typed JSON-RPC error, and maps only codes -32020,
-32022, -32600 and -32602 to exact `MCP request negotiation failed`. Native HTTP
status, code and ID remain; native message/data and response headers are discarded,
with fresh JSON content type. Integer/string IDs are correlation values, not
redacted fields. Explicit null versus absent ID is preserved; SDK Option serialization
alone would omit null. Unknown envelopes/codes retain original bytes and headers.
Unknown-size or oversized candidates and all other statuses pass unchanged, including
HTTP 200 protocol errors. No request body or successful SSE body is polled by this
layer. It is not an arbitrary transport, logging or universal redaction guarantee.
If incorrectly mounted over a fallible/non-SDK finite body that fails reading, the
fallback is same HTTP 400 / fixed `MCP negotiation response unavailable` plain text,
without native details or a JSON-RPC code. Native code preservation is qualified only
for recognized SDK envelopes. Parser/plain-text/native/session errors, SDK warnings
and raw message tracing, panics, successful values and production admission limits
still require explicit shipping decisions. No production hosting is enabled.

The official matrix uses a test-only header corrupter for modern tools/list, and a
legacy custom initialize request with mismatched version. Both must decode
`ServiceError::McpError` (-32020/-32600 respectively), with fixed text/no data, across
JSON and SSE. After host non-entry is checked, a real overview call completes. In SSE
mode the official client consumes matching progress 1 while the host is still gated,
then receives the result after release. Progress stays in the existing test-only
wrapper. This checks liveness rather than collecting a successful stream. It does
not qualify official-client production auth, every malformed request shape or native
cleanup. Fixture five-second waits/listener cleanup are unchanged.

`tests/negotiation_boundary.rs` checks selected-code mapping, integer/string/null
IDs, dropped diagnostic headers/data, unchanged HTTP 200/404/500 responses, plain
text, malformed/unknown-code JSON and oversized bodies. Two explicit composition
orders prove `with_fixed_http_errors` still erases the envelope/code, returning its
original fixed plain text. Existing three official success/cancellation tests retain
that HTTP wrapper. The new negotiation tests do not compose with it unless explicitly
testing that loss; neither combination is final standards-ready production hosting.

TDD: an identity stub for the new layer produces behavioral RED on reflected native
-32020 text; native characterization passes unchanged. The mapper makes it GREEN.
The added explicit-null correlation test then fails because SDK serialization omits
`id: null`; preserving the original ID presence makes it pass. A subsequent identity
mutation independently fails raw and official fixed-error tests and synthetic mapping;
restoring the layer passes. Refactoring reuses the SDK metadata builder (removing a
clone of an older fixture helper), separates raw case assertions and official error
checks, and keeps existing raw probe construction as an identity-hosted wrapper.

Validation: locked isolated suite **28 pass** (11 adapter, 5 boundary, 5 HTTP,
4 lifecycle, 3 negotiation-boundary), strict all-target Clippy, formatting and staged
whitespace pass. The desktop SDK dependency probe also passes (18 existing desktop
bin warnings); full desktop/core/CLI, UI, minimum Rust and native OS qualification
are not rerun. The temporary identity mutation is restored before final checks.

Ripwire impact/uses before extraction identify one existing raw-probe start caller
and its parent test module; the old start signature and identity hosting are retained.
Edit-check reports unchanged signature, no incompatible callers. Affected selection
for the new mapper resolves the two new test modules but no runner; runtime tests
supply that evidence. Final quality has zero gates/preexisting regressions, nine new
reachability findings for executed tests/test callbacks and one non-gating verbosity
finding for the explicit eight-case data table (68 lines). Initial fixture-helper
clone and test complexity/verbosity findings were removed by the refactors above,
without suppressions. Staged test gate exits 4: five executed test files/modules have
unknown runners; two documentation symbols and dynamically dispatched
`supported_protocol_versions` are unmodeled. Eight of ten hunks use file scope
(140 seeds, 17 impacted). This is not a clean static coverage claim.

Next: qualify the injected adapter over standard stdio in an independently reviewable
isolated spike, inspecting the pinned SDK's IO, service lifecycle and official client
first. Keep synthetic hosts separate from real CLI runtime/keyring integration; prove
clean stdout and lifecycle/cancellation behavior before enabling a CLI MCP command.
Do not assume these HTTP layers sanitize stdio protocol errors or SDK logs. HTTP
logging/admission/session/native error policies remain shipping decisions. Coordinated
CLI import, other operation contracts and packaging remain actionable independently
of the unchanged Linux containment qualification gate.

## Subprocess stdio overview qualification (2026-11-13)

The same unchanged `OverviewAdapter` now runs in the isolated spike's synthetic
`stdio-fixture` binary. It uses `ServiceExt::serve(stdio_pipes())`, waits for the
service to finish, and writes fixed entry/exit/host-read markers only to stderr.
Five explicit test modes inject successful DTOs, a secret-bearing host failure,
an empty category allowlist, an unknown category, or an unknown policy. There is
no runtime, keyring, configuration, profile owner, HTTP listener/token, child
operation, tracing subscriber, or production `scpctl mcp` command.

The lifecycle choice comes from the **registry-published rmcp 3.10.0 source**:

- `transport/io.rs`: `stdio()` uses blocking Tokio stdin; a pending read cannot
  be cancelled and can delay runtime shutdown. `stdio_pipes()` reopens FIFO
  descriptors through `/proc/self/fd` on Linux with private nonblocking flags;
  other platforms/non-FIFO handles fall back to blocking stdio.
- `service/server.rs::serve_server_with_ct_inner`: legacy `initialize` selects
  the handshake path. Modern `server/discover` with complete metadata is a
  bootstrap probe and leaves lifecycle selection open; the first non-discover
  request with complete inline metadata establishes the modern service loop.
- `service/client.rs`: explicit `ClientLifecycleMode::Initialize` and
  `Discover { preferred_versions }` work over generic transports. Default
  `ServiceExt::serve` on a client still initializes; no Auto fallback is tested.
- `transport/async_rw.rs`: newline framing, a default 16 MiB incoming line cap,
  discard of incomplete trailing lines at EOF, skipped syntax failures and
  invalid-request responses for some structurally invalid JSON. An official
  client can ignore non-JSON stdout, so client success alone cannot prove clean
  stdout. `service.rs` closes the transport after draining responses; source
  allows up to five seconds for EOF drain and two for cancellation, then cancels
  the internal serve token. This is not a native-work joining guarantee.
- The SDK child-process wrapper was inspected too. These tests instead retain
  explicit Tokio child ownership/wait plus the official SDK client over the
  `(ChildStdout, ChildStdin)` async transport. That observes real process exit
  rather than treating the wrapper's termination fallback as graceful EOF.

`tests/stdio.rs` adds three tests (16 subprocesses in total):

1. Raw newline JSON-RPC probes run both `2025-11-25` initialization and
   `2026-07-28` discovery/inline metadata. Each checks tools-only discovery,
   the one overview tool, exact schemas, omitted/empty arguments, exact DTO
   structured/text output and revision-specific resultType presence. Failed
   host results expose only `App overview unavailable`, no structured content.
2. Raw policy probes cover disabled/unknown categories, unknown policy, unknown
   and unimplemented tools, and nonempty arguments. Rejections retain `-32602`,
   omit error data/secrets, and produce no host-entry marker.
3. The official SDK client runs both lifecycle modes against the same process,
   checks negotiated revision/capabilities, exact catalog/schema, DTO and fixed
   host errors. Cancelling the idle client closes its write pipe; the test waits
   for successful subprocess exit and the fixed stop marker.

Raw probes validate **every** stdout line as a newline-terminated JSON-RPC
response with the expected correlation ID, including checking for no trailing
output after closing stdin. Their 64 KiB line cap and five-second waits are
fixture bounds. All reads are completed before EOF; active-work cancellation,
concurrent work, progress, backpressure, malformed input, signal cleanup and
EOF before lifecycle selection remain separate tests. No PTY output isolation,
production hard shutdown bound or Windows/macOS native qualification is claimed.
No HTTP error layer is mounted or relevant to these pipes. Native SDK parser,
service errors/logging, panics and successful DTO values remain outside the
adapter's returned-host-error boundary.

TDD: an unhosted binary stub made all three behavioral tests fail at protocol EOF
or official initialization. Minimal hosting passes. A subsequent mutation wrote
the startup diagnostic to stdout: both raw tests failed on invalid protocol JSON,
while the official client test still passed, confirming the need for direct line
inspection. The mutation was restored before final checks.

The exact SDK pin/checksum is unchanged. The spike enables `transport-io`, adds
normal Tokio runtime features for its fixture binary and dev-only Tokio process
support for the harness. The committed isolated lock adds only
`signal-hook-registry 1.4.8` via Tokio process support; existing versions are
unchanged. Root core/CLI dependencies and production desktop commands are unchanged.

Validation: the locked isolated suite passes **31 tests** (11 adapter, 5 boundary,
5 HTTP, 4 lifecycle, 3 negotiation boundary, 3 stdio). Strict all-target Clippy,
format and staged whitespace checks pass. The locked desktop SDK dependency
probe passes with 18 existing warnings; its normal-edge SDK reverse graph remains
empty. The ignored desktop lock was refreshed from its saved original with only
the new spike→Tokio edge, preserving all versions. A discarded full-lock
regeneration selected unrelated upgrades; that check was intentionally stopped
before the minimal refresh and successful rerun. The three additional core targets
selected by Ripwire pass 30 tests (one ignored foreground subprocess fixture).
Full root/desktop suites, release/MSRV/native-OS and UI checks are not rerun.

Ripwire before reuse reports three direct adapter constructors and 16 downstream
symbols; no existing constructor/adapter/test-host source changed. Edit-check
recognizes the new synthetic host method with no incompatible callers. Final
quality delta exits **2**: adding another `OverviewHost::read_overview` implementor
makes its approximate trait resolver mark the unchanged HTTP `FakeHost` method
as dead. The existing HTTP adapter tests exercise that method, including their
entry/release gates, and all 11 pass; this is a static reachability limitation,
not a removed implementation. Four other new, non-gating reachability findings
cover the synthetic trait method and three test entries. No suppression or
baseline changes are added. The staged test gate exits **4**, selecting 14 test
files/modules with unknown runners and 113 untested symbols across a coarse
file-scope/ambiguous `main` blast radius. The complete spike suite and three core
targets exercise those selected test modules; this is not a clean static coverage
verdict for unchanged desktop/runtime paths.

The next increment below qualifies cooperative cancellation and an unrelated
concurrent read. Active EOF and native cleanup remain separate qualifications.

## Cooperative stdio cancellation and concurrency (2026-11-13)

The isolated `stdio-fixture` now accepts a `gated` mode with a harness-supplied
loopback address. Each injected `SyntheticHost::read_overview` opens its own
control connection, emits a fixed preface and `entered`, and waits for either the
actual request token or a test command. `check` returns `active`; `release` emits
`released`/`finished` and permits the unchanged shared DTO. Cancellation emits
`cancelled`/`finished`, closes the gate connection and returns a host error. The
fixed error is discarded by the unchanged `OverviewAdapter`; the SDK suppresses
the response for the cancelled request. These markers describe synthetic work,
not native cleanup. No production host requirement, progress wrapper, tool,
runtime/profile/keyring/config or HTTP transport is introduced. Control reads
never touch MCP stdin. Gate input is capped at 128 bytes per command. The harness
requires a fixed preface to ignore devbox HTTP/TLS port probes; this is test
framing, not production authentication.

Published registry source inspected before fixture selection:

- `service.rs::RequestHandle::cancel` sends a `CancelledNotification` with the
  handle's ID through `Peer::send_notification`; this consumes the handle and
  does not wait for host completion.
- The generic `(AsyncRead, AsyncWrite)` adapter uses `AsyncRwTransport`, with no
  Streamable HTTP worker interception. For both supported revisions the server
  service loop removes the matching request token from `local_ct_pool` and
  cancels it on `notifications/cancelled`. It drops responses whose token was
  removed. Handlers must still cooperate; cancellation does not abort/join them.
- The IO/lifecycle/drain paths described above are unchanged: EOF drains before
  cancelling the internal service token. Active EOF is intentionally not covered
  by this request-cancellation test. The non-Linux/non-FIFO blocking stdin
  fallback and five-second EOF/two-second service-cancel drains still do not
  establish hard process/native cleanup bounds.

`tests/stdio/cancellation.rs` adds one official-client test over both revisions,
two subprocesses. It waits for first entry before submitting the second call,
then confirms both are active. `RequestHandle::cancel` must produce the first
gate's cancellation/completion/EOF, while the second still acknowledges active.
Only an explicit separate-channel release lets the survivor finish and return
the exact structured/text overview DTO with the correct revision result shape.
There are no sleep-based entry assumptions or HTTP progress/priming dependencies.
Cancelling the now-idle client must close its writer, and child wait must succeed
with the fixed stop marker and exactly two stderr host entries.

To avoid the official SDK's non-JSON-line tolerance, every stdout line is checked
and forwarded by a task sharing `read_response_line` with existing raw probes.
The test joins this task through EOF and requires exactly two correlated
responses: handshake and survivor, none for the cancelled ID. Validation retains
the 64 KiB newline cap and rejects diagnostics, invalid envelopes and trailing
output; the forwarding fixture caps response count at 16. These limits and the
five-second watchdogs are test safeguards, not production throughput promises.

TDD: the test compiled first against the existing ungated binary and failed at
the entry watchdog. Early harness attempts accepted devbox HTTP/TLS probes;
the fixed binary preface removed that interference before recording the intended
RED. Gated host implementation passed. A mutation replacing the request token
with `CancellationToken::new()` then failed at cancellation completion; restored
code passed. The full suite passed again after sharing stdout validation to
remove a duplicate identified by Ripwire. No dependency version or lockfile
changes were needed; normal Tokio net/IO features are explicit for the fixture.

Validation: locked isolated suite **32 pass** (11 adapter, 5 boundary, 5 HTTP,
4 lifecycle, 3 negotiation boundary, 4 stdio), strict all-target Clippy, format
and whitespace checks pass. The locked desktop dependency probe passes with
18 existing warnings; the normal-edge SDK reverse graph is empty. Root/core,
full desktop, release/MSRV, native Windows/macOS and UI suites were not rerun.
Ripwire impact/uses cannot resolve the host trait dispatch (one declined adapter
call); source confirms the synthetic fixture use. The shared stdout-reader
refactor affects raw `Wire::request`/start/tests; approximate name matches also
list existing HTTP tests, all exercised by the full suite. Edit-check reports
unchanged signatures and no incompatible callers. Quality delta exits 0 with
no preexisting-worse/gating findings; two new non-gating findings mark the test
as dead/long (71 statements versus a 60 threshold), although Cargo executes it.
No suppressions or baseline changes were added. The staged test gate exits 4:
eight test files/modules have unknown runners, all run by the full spike suite;
three unrelated untested paths are the desktop `run_boundary_oidc_automation`
and two documentation headings. Its coarse file-scope/name graph is not evidence
that this synthetic-only change affects native OIDC, nor a clean static coverage
claim for that unchanged code.

Next: qualify active EOF using the separate gate, beginning with controlled
release during SDK drain and response/process completion. Drain expiry/internal
token cancellation is not a host join; keep non-cooperative/native limitations
explicit. EOF before lifecycle selection, malformed input, backpressure, signals,
production stdio ownership, HTTP shipping policy, CLI coordinated import, other
operations and packaging remain open; the unchanged cgroup gate is not retried.

## Active stdio EOF with controlled drain completion (2026-11-13)

The next focused slice qualifies successful completion during EOF drain, using
`tests/stdio/eof.rs` and the same injected overview adapter/host gate. It does not
qualify drain expiry. Both legacy Initialize (2025-11-25) and modern Discover /
inline metadata (2026-07-28) run in real subprocesses, with no production CLI,
runtime, keyring, config, profile, HTTP host or new MCP tool.

Fixture selection rechecked the published 3.10.0 source:

- `transport/io.rs::stdio_pipes` retains Linux FIFO reopening and the blocking
  fallback on other platforms/non-FIFO streams. `async_rw.rs` owns newline parsing
  and returns no message on input EOF. The harness never reads subprocess stdin.
- `service/server.rs` and `service/client.rs` retain the existing lifecycle
  selection. The harness reuses `Wire`'s checked handshake; modern calls supply
  complete inline metadata, so EOF follows lifecycle selection and host entry.
- `service.rs::serve_inner` emits `input stream terminated` only after its actual
  `transport.receive()` returns None, immediately before selecting Closed. It
  drops its response sender, joins already spawned response-send tasks and drains
  remaining handler responses for up to five seconds. Only afterward does it
  cancel the internal token and close the transport. Cancellation uses a separate
  two-second drain. EOF does not cancel an externally supplied service token.
  The response drain is not a join of all handler tasks or native work.

A fixture-only `gated-eof` mode installs a minimal tracing subscriber. It enables
only INFO events targeted at `rmcp::service`, recognizes that exact fixed message,
and emits only `fixture sdk input eof` on stderr. It forwards no raw event fields,
errors or SDK diagnostics and writes no stdout. Other fixture modes retain their
previous behavior. This is pinned-source test instrumentation, not a production
logging policy or general sanitizer. It reuses the already locked `tracing` crate
as a direct spike dependency; the spike lock adds only that dependency edge.
rmcp remains exactly 3.10.0 with the same checksum. The ignored local desktop lock
also required an offline metadata refresh for this dev-dependency edge; no package
versions changed and no tracked desktop source/lock changes were made.

The test requires gate entry and active acknowledgment before closing stdin.
It consumes stderr through the actual SDK EOF marker, confirms the process has
not exited, and obtains another active acknowledgment before releasing work.
Release/finished/gate EOF must precede the exact correlated shared DTO response,
including text, structured content and revision-specific resultType. The raw
validator checks every stdout line through EOF; no extra response or diagnostic
line is allowed. Successful child wait, one host read, one SDK EOF marker and the
final stop marker complete the evidence. No arbitrary sleep establishes EOF.
The marker identifies the loop's EOF branch, not a separately instrumented drain
entry; source ordering plus work still held until the post-marker release and
the subsequent response establish completion through the EOF drain path.

Five-second per-step watchdogs and kill-on-drop remain test safeguards. This
prompt-release case does not wait for the SDK's five-second drain expiry and
makes no maximum process/native cleanup claim. An expiry test needs a separately
scoped bound exceeding the SDK window, plus explicit internal-token/handler
completion evidence; it must not race the existing five-second watchdog. Blocking
stdio fallback is still not qualified on native Windows/macOS.

TDD: the new behavioral test compiled against the prior fixture and failed at
host entry (unsupported gated-eof mode). Minimal gated hosting plus the fixed
observer passed both revisions. Mutating away the observer's EOF marker then
failed at the EOF-evidence watchdog; restoring it passed the full suite. The raw
harness connection setup is shared with the existing tests, and gate methods are
visible only to their parent test module for reuse. No adapter or host operation
semantics changed.

Validation: locked full spike suite **33 pass** (11 adapter, 5 error boundary,
5 HTTP, 4 lifecycle, 3 negotiation boundary, 5 stdio), strict all-target Clippy,
format and whitespace checks pass. The locked desktop dependency probe passes
with 18 existing warnings; the normal-edge SDK reverse graph remains empty.
Root/core/full desktop, native OS, MSRV, release and UI suites were not rerun for
this synthetic-only slice. Ripwire sees no callers of fixture main; source and
subprocess tests establish its use. The Wire setup refactor affects the two raw
stdio tests and the new EOF probe, all executed in the full suite. Edit checks
retain signatures with no incompatible callers. Static test reach remains
approximate for Cargo tests and tracing trait dispatch. Quality delta exits 0:
seven new non-gating dead-code findings concern the test/tracing trait methods,
with two minor short-horizon churn findings and no gating regression. No
suppressions or baseline changes were added. The staged test gate exits 4 for
nine unknown-runner test files/modules (all exercised by Cargo) and three
unrelated untested paths: desktop `run_boundary_oidc_automation` and two doc
headings. Its file/name graph is not a clean coverage claim for unchanged OIDC.

Next: qualify EOF drain expiry/internal-token cancellation with explicit evidence
and an appropriately scoped watchdog, while distinguishing it from joining
handlers/native cleanup. EOF before lifecycle selection, malformed input,
backpressure and signals remain separate. Production stdio ownership, HTTP
shipping policy, coordinated CLI import, other operation contracts and packaging
remain actionable; do not repeat unchanged cgroup qualification.

## Active stdio EOF drain expiry and internal cancellation (2026-11-13)

`gated-eof-expiry` reuses the same `SyntheticHost`, `OverviewAdapter`, gate protocol
and SDK stdio transport. The observer additionally enables WARN events from
`rmcp::service` and maps only the exact fixed `timed out draining in-flight
responses` message to `fixture sdk drain expired` on stderr. It forwards no raw
fields/diagnostics and never reads MCP stdin or writes stdout. This remains
fixture instrumentation, not production logging or sanitization policy.

Published 3.10.0 `transport/io.rs`, `async_rw.rs`, service/server/client lifecycle
and `service.rs` drain/RunningService code were inspected before fixture selection.
After actual receive-None EOF the loop selects Closed and a five-second response
drain, emits that WARN on expiry, then cancels `serve_loop_ct`. Request/context
tokens descend from that internal token. Service cancellation instead selects a
two-second drain. Neither drain is a join of handler/native work.

`RunningService::waiting(mut self)` takes/awaits its service handle and then drops
the wrapper. Its `dg: ct.drop_guard()` cancels the externally supplied token on
drop even though the EOF loop does not. The new fixture therefore retains the
wrapper while waiting for an optional `Notify` acknowledgment after `gated_read`
returns. That synthetic gate work has written cancelled/finished and closed its
connection; the host asserts the external token is still active before calling
waiting. Closed and the subsequent external-token cancellation are separately
asserted and marked. The host wait prevents runtime exit from discarding gate
work before it reports. It observes synthetic completion only; it neither joins
the entire handler nor proves SDK joining/native cleanup. Other modes retain
the same immediate waiting behavior; explicit serve_with_ct supplies the token
that serve otherwise creates internally.

The raw test runs legacy Initialize and modern Discover/complete inline metadata
in separate subprocesses. It proves gate entry and active state before stdin
closure, actual SDK EOF and active state afterward, then waits for the exact
expiry marker without sending any release/cancel command. Only that observation
uses a 12-second watchdog, exceeding the SDK's five-second window; normal entry,
post-expiry gate and process checks retain five-second fixture watchdogs. No sleep
establishes EOF or expiry, and these are not production shutdown deadlines.
Cancellation/finished/gate EOF, absence of any remaining stdout response, actual
successful child wait, one host read/EOF/expiry marker and the final stop marker
are required. Raw newline/64 KiB JSON-RPC validation remains active through EOF.

TDD: unsupported mode fails host entry first. The minimal expiry mode passes both
revisions. Removing only the expiry marker fails when stderr reaches EOF without
that evidence. A second mutation replacing the actual request token with a fresh
independent token fails waiting for gate cancellation after observed expiry.
Both mutations are restored before final validation. No dependency/lockfile or
production source changes are needed; rmcp remains exactly 3.10.0.

Validation: the locked full spike suite passes **34 tests** (11 adapter, 5 error
boundary, 5 HTTP, 4 lifecycle, 3 negotiation boundary, 6 stdio). Strict all-target
Clippy, formatting and whitespace checks pass. The locked desktop SDK dependency
probe passes with 18 existing warnings; its normal-edge SDK reverse graph is
empty. No root/core/full desktop, native Windows/macOS, MSRV, release or UI checks
were rerun for this synthetic-only slice. No dependency or lock changes occurred.

Ripwire impact/uses report no direct fixture main/observer callers and decline
the host trait call from the adapter; source and subprocess tests verify these
uses. Existing symbol edit checks retain signatures with no incompatible callers.
The EOF tests share their handshake/held-entry/actual-EOF setup after quality
analysis found duplication; the restored full suite passes after refactoring.
Final quality delta exits **2** solely for a major short-horizon-churn finding on
fixture main (`churn=self`, three recent touches); it is retained as a disclosed
history risk, with no suppression, baseline alteration or artificial extraction.
Two minor churn rows and one new non-gating test dead-code row also remain;
Cargo executes that test. The duplicate/length findings are gone. This is not a
clean quality-delta verdict or proof of static coverage. The staged test gate
exits 4 for one unknown-runner EOF test module (executed by Cargo) and two
unrelated untested documentation headings; it is not a native coverage claim.

Next: qualify EOF before lifecycle selection, with explicit observations for
legacy initialization and modern discovery/inline selection. Malformed input,
backpressure, signals, native Windows/macOS stdio, production host ownership,
HTTP shipping policy, coordinated CLI import, other operations and packaging
remain open. The unchanged cgroup gate is not retried.

## Startup stdio EOF before service selection (2026-11-13)

The isolated `startup-eof` mode keeps the same synthetic overview adapter and
`stdio_pipes()` transport, with rmcp pinned exactly to 3.10.0. It adds no tool,
production host/config/keyring/profile wiring, or scpctl MCP command.

Source inspection before fixture selection:

- `service/server.rs::expect_next_message` maps receive-None to typed
  `ServerInitializeError::ConnectionClosed(context)`. The bootstrap loop always
  supplies `initialize request` as context, including after modern discovery.
  Complete inline-metadata discovery requests are dispatched/replied inline and
  continue that loop; only the first valid supported non-discover request enters
  `serve_inner`. Thus a discovery response does not prove RunningService exists.
- Legacy initialization sends InitializeResult then immediately calls
  `serve_inner`; `notifications/initialized` is handled as a regular notification,
  without a startup gate. EOF after that response is service-loop closure.
- `service/client.rs` similarly maps receive-None to typed client ConnectionClosed
  with the response context. Initialize mode sends the initialized notification
  after the response; Discover mode sets peer info/per-request metadata after the
  discovery response, then creates its own service loop. These client semantics
  are why the new server-stage probes use raw pipes, not official-client startup.
  Auto fallback remains unqualified.
- `transport/async_rw.rs` returns None on complete input EOF (also on read/error
  reply failures). Its startup caller does not preserve that transport distinction.
  Here the harness deliberately closes stdin after valid messages or no bytes,
  with stdout still open; no transport fault is injected. Incomplete EOF tails,
  syntax/shape failures, oversized lines and raw SDK logs remain separate cases.
  Linux FIFO reopening versus blocking non-Linux/non-FIFO stdio is unchanged.

Three tests in `tests/stdio/startup_eof.rs` launch separate real subprocesses:

| Stage established before EOF | Exact evidence after stdin closure |
| --- | --- |
| Fixed started marker, no bytes ever written | Typed server ConnectionClosed with context `initialize request`, external token still active; no stdout response, raw Rust termination diagnostic, unsuccessful actual child wait |
| Correlated modern discovery result, exact tools capabilities and both supported revisions; request had complete inline metadata | Same startup error/token/diagnostic/wait evidence, no further stdout response; no RunningService or service-loop EOF marker |
| Correlated legacy InitializeResult with exact revision/tools capabilities, then fixed RunningService-created marker; no initialized notification | Exact SDK service-loop EOF marker, waiting returns Closed, consumed wrapper's external token cancelled, fixed stopped marker and successful child wait |

The legacy creation marker is awaited **before** closing stdin, avoiding a race
between its diagnostic and the SDK loop's EOF event. Startup error instrumentation
matches the actual typed variant/context and asserts the external token remains
active; `inspect_err` forwards the original error unchanged. The fixture returns
it through Rust's Result termination path, producing exactly
`Error: ConnectionClosed("initialize request")` on stderr. This is a raw synthetic
characterization, not a decision to print arbitrary SDK errors in production.
There is no wrapper drop guard/service-loop drain in those two startup paths.
The legacy waiting/drop guard observation retains its existing meaning; it does
not prove that the EOF loop itself cancels the external token.

Every stdout line, including final EOF, uses the shared strict newline/64 KiB
JSON-RPC checker; all responses are correlated before closure. Exact stderr
comparison also excludes host reads, unexpected markers and raw sentinel output.
Five-second fixture watchdogs bound each test step; no sleep establishes stage
or EOF and no cancellation command substitutes for stdin closure. No native work
is launched, and these are not production shutdown deadlines or hard cleanup
claims. The fixed observer remains unchanged test-only instrumentation, not a
shipping diagnostic layer or general redactor.

TDD: the three behavioral probes first fail against the previous fixture at the
missing typed-startup/RunningService observations, after checking protocol output,
EOF and actual wait. Adding the mode passes all three. Removing only the fixed
typed-startup observation makes both startup-error probes fail while the legacy
probe passes; the original source is restored. Refactoring moves the startup
experiment's separate error/exit handling into its own fixture module and splits
protocol-stage setup from EOF checks. Main's existing active-work handling stays
unchanged; the legacy marker is explicitly consumed before closure.

Validation: locked full spike suite **37 pass** (11 adapter, 5 error boundary,
5 HTTP, 4 lifecycle, 3 negotiation boundary, 9 stdio); strict all-target Clippy,
formatting and whitespace pass. Locked desktop SDK dependency probe: 1 pass,
18 existing warnings; normal-edge SDK reverse graph empty. No dependency/lock
changes. Root/core/full desktop, native Windows/macOS, MSRV, release and UI suites
were not rerun for this synthetic-only scope.

Ripwire reports no direct fixture-main callers/uses; subprocess source/tests
establish use. Main edit check preserves signature with no incompatible callers.
Initial quality flagged main growth; separating startup-mode lifecycle handling
and test protocol setup removes verbosity findings. Final quality delta exits 0
with one minor main churn finding and four new non-gating dead-code rows (three
Cargo test functions and module-dispatched fixture run); no suppressions/baseline
changes. The staged test gate exits 4 for ten unknown-runner files/modules
(executed by Cargo), unchanged desktop `run_boundary_oidc_automation` and two
unrelated documentation headings. This is not a static coverage claim for OIDC
or native cleanup; the Cargo suite is the executed evidence for this slice.

Next: characterize malformed stdio input/errors at startup versus service dispatch
with synthetic sentinels and raw protocol/diagnostic evidence before choosing
shipping error policy. Neither HTTP error experiments nor the fixed EOF observer
qualifies that boundary. Backpressure, signals, native Windows/macOS stdio,
production ownership, HTTP shipping policy, coordinated CLI import, other
operation contracts and packaging remain actionable; unchanged cgroup delegation
is not reprobed.

## Unsolicited stdio peer errors and malformed dispatch (2026-11-13)

The isolated `input-errors` fixture mode preserves rmcp =3.10.0 and the same
synthetic overview adapter. Three raw subprocess tests qualify one specific
boundary: a well-formed JSON-RPC error envelope with no ID, supplied where the
server has no pending outbound request. The error's message and password-shaped
data carry only the synthetic `SYNTHETIC_STDIO_SECRET` sentinel. This is a
protocol-stage/association probe, not syntax-invalid or structurally invalid JSON.

Pinned published source was inspected before selecting fixtures:

- `transport/async_rw.rs::receive` skips complete syntax-invalid lines. Valid JSON
  that fails the typed message shape instead queues fixed `-32600` / `Invalid
  request` with no ID. Compatibility parsing can log the supplied raw line at
  DEBUG. Neither branch is exercised by this well-formed error-envelope probe.
- `service/server.rs` handles bootstrap ping/discovery specially; complete modern
  discovery alone does not select the service lifecycle. Non-request envelopes
  reach `ExpectedInitializeRequest(Some(message))`. This typed variant retains
  the supplied message; its Display and derived Debug include that message.
  `error.rs` also formats ErrorData message/data, without a redaction guarantee.
- `service.rs` drops idless peer errors in an active service after a DEBUG event
  containing the error. It does not generate a protocol response for that event.
  This differs from the transport-generated idless error response above.
- The untagged `ClientRequest` union falls back to `CustomRequest` for the chosen
  malformed `tools/call` (string `arguments` instead of an object). ServerHandler
  dispatch reaches OverviewAdapter's existing fixed custom-request error. SDK
  service DEBUG request and TRACE event logging may still contain supplied data.
- `handler/server.rs` rejects modern service ping with method-not-found, while
  bootstrap ping responds inline without selecting a lifecycle. Tests therefore
  establish active dispatch using correlated `tools/list`, not ping or timing.

| State and probe | Required evidence |
| --- | --- |
| No lifecycle selected; unsolicited idless peer error | Actual typed ExpectedInitializeRequest containing Error, absent ID, exact code/message/data; external service token still active; no stdout response; unsuccessful actual process wait |
| Legacy initialize/initialized and correlated tools/list; same peer error then malformed tools/call | No peer-error reply; correlated fixed -32601 / MCP method not supported with no data; subsequent exact catalog result; successful EOF/process wait |
| Modern discovery then correlated tools/list; same active probes | Same fixed error/recovery/exit evidence, with required modern request metadata |

The fixture's `inspect_err` checks the actual SDK variant and retained payload,
emits a fixed typed-observation marker, and forwards the original error unchanged
through `?`. Rust's raw Result termination diagnostic starts with
`Error: ExpectedInitializeRequest(Some(Error(JsonRpcError ...` and includes both
sentinel values. The test pins the **entire** diagnostic, including code, absent
ID, message and data. This is evidence that an unfiltered startup error can expose
input, not a decision to ship that diagnostic. No RunningService exists on this
path and the external token remains active. The active paths check `waiting()`
returns Closed and wrapper consumption cancels the external token; this is not a
claim that EOF itself cancels it or that native handlers are joined.

`tests/stdio/input_errors.rs` uses only real subprocess pipes, actual stdin EOF,
strict newline/64 KiB JSON-RPC stdout validation, final stdout EOF and bounded
actual child waits. Every outgoing response in this slice has an ID and is
correlated; the shared validator is unchanged. The harness deliberately sends an
idless **incoming** error, never expects an idless stdout response. Exact stderr
comparisons exclude host reads and additional diagnostics; active stderr contains
only fixed lifecycle markers. No subscriber is installed in this mode: absence of
sentinels there does **not** qualify raw SDK logs. The existing EOF observer is
unchanged and is not used here. Five-second step watchdogs are test limits, not
production deadlines. No production sanitizer, tool, stdin interceptor, real
runtime/profile/config/keyring wiring or CLI MCP command is added.

TDD: new tests run against the prior mode dispatcher fail at missing typed/stage
observations. An initial ping-based oracle was corrected after inspecting its
bootstrap/modern dispatch distinction; rerunning the corrected tests with the new
mode branch removed produces three expected RED failures. Restoring the branch
passes all three. Removing only the fixed typed-startup marker produces one RED
startup test and two passing active tests; the original module is restored.
Refactoring moves existing active/gated service lifecycle handling unchanged into
`stdio_fixture/active.rs`, leaving fixture main responsible for construction and
mode dispatch. All 12 stdio tests, including previous EOF/cancellation probes,
pass after that move.

Validation: locked full spike suite **40 pass** (11 adapter, 5 error boundary,
5 HTTP, 4 lifecycle, 3 negotiation boundary, 12 stdio), strict all-target Clippy,
formatting and whitespace checks pass. Locked desktop SDK dependency probe:
1 pass with 18 existing warnings; normal-edge SDK reverse graph remains empty.
No dependency/lock changes or root/core/full desktop/native Windows/macOS/MSRV/UI/
release reruns for this synthetic-only scope.

Ripwire main impact/uses report no direct callers; source/tests verify subprocess
usage, so affected-test selection is supplemented with the full spike suite.
Quality delta exits 2 only for major recent main churn after the lifecycle move;
verbosity growth is removed. Five non-gating new-symbol dead-code rows are three
Cargo tests and two module-dispatched run functions. No suppressions or baseline
edits. This structural finding is retained explicitly, not called a clean gate.
The staged test gate exits 4 for eleven unknown-runner test files/modules (all
executed by Cargo), unchanged desktop `run_boundary_oidc_automation` and two
unrelated documentation headings. Main edit-check reports unchanged signature,
zero direct callers and no incompatible uses. No static/native coverage claim.

Next focused increment: qualify complete syntax-invalid lines versus structurally
invalid valid JSON at startup and active dispatch, selecting a bounded fixture
set from the pinned parser source. Any expected no-ID invalid-request stdout
response needs a strict error-envelope checker preserving newline/64 KiB limits;
do not weaken stdout validation to merely valid JSON. Use correlated subsequent
responses or fixed test-only stage evidence, actual stdin EOF and actual child
waits. Keep raw diagnostic characterization distinct from a later shipping
policy. Backpressure, signals, native Windows/macOS stdio and hard native cleanup
remain unqualified. HTTP shipping policy, production ownership, coordinated CLI
import, other operations and packaging remain independently actionable. Do not
repeat unchanged cgroup qualification.

## Complete stdio syntax errors versus typed envelope failures (2026-11-13)

`tests/stdio/parser_errors.rs` adds four subprocess probes and one stdout-checker
regression test, retaining rmcp =3.10.0 and the existing synthetic binary/adapter
unchanged. The bounded input set is deliberately two complete newline-terminated
lines, both carrying only `SYNTHETIC_STDIO_SECRET`:

- Syntax-invalid JSON: an object containing ID 77 and a password-shaped sentinel
  field, with a trailing comma. It is not an incomplete EOF tail.
- Valid JSON, invalid typed request: `jsonrpc: "2.0"`, ID 77, numeric method 17,
  and a password-shaped sentinel in params. A numeric method cannot deserialize
  into CustomRequest or match the compatibility notification bypass. This differs
  from string-arguments `tools/call`, which the previous probes show becomes
  CustomRequest and gets a correlated fixed -32601 from the adapter.

Before selecting these lines, the pinned published `transport/async_rw.rs`,
`model.rs` / `model/serde_impl.rs`, `service/server.rs`, and `service.rs` were
inspected. `receive` treats serde Syntax/Eof classifications as skipped input;
Data/Io classifications queue fixed `ErrorData::invalid_request("Invalid
request", None)` with no ID. `pending_error_response` feeds then flushes that
response before reading the next line, preserving the Queue/Flush state across
receive cancellation. These small probes demonstrate ordinary sequential flushing
and recovery, not cancellation while a write is blocked. Compatibility parsing
tries the typed message first, then recognizes selected notifications to ignore;
its DEBUG diagnostic can include the entire supplied raw line. Numeric method
avoids that notification path. Service dispatch is downstream of this parser;
invalid typed envelopes are not delivered as unsolicited incoming Error messages.

The [published MCP error response shape](https://modelcontextprotocol.io/specification/2026-07-28/basic#error-responses)
allows the ID to be absent when a malformed request prevents reading it. The SDK
`JsonRpcError.id` is optional and omitted on serialization. The observed reply is
exactly `{"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid request"}}`,
including omission of ID even though the invalid input contains numeric ID 77.
The same pinned transport emits it after legacy initialization. This records
SDK behavior, not a claim of full legacy protocol compliance: the
[base JSON-RPC specification](https://www.jsonrpc.org/specification#response_object)
requires a null ID when the request ID cannot be detected. No transport policy or
SDK version changes are made in response to this observation.

| Stage | Syntax-invalid complete line | Valid JSON with numeric method |
| --- | --- | --- |
| Before first discovery, then again after complete discovery | No reply before each correlated discovery result | One exact fixed idless error before each correlated discovery result |
| Active legacy and modern service, established by tools/list | No reply before subsequent correlated catalog result | One exact fixed idless error, then subsequent correlated catalog result |
| Actual stdin EOF after bootstrap probes | No RunningService; original typed ConnectionClosed("initialize request"), active external token, exact raw diagnostic and unsuccessful child wait | Same |
| Actual stdin EOF after active probes | No remaining stdout; exact service-created/waiting-Closed/external-cancelled/stopped markers and successful child wait | Same |

Bootstrap uses the unchanged `startup-eof` mode, whose inspect_err asserts the
actual typed error and active token and forwards the original error with `?`.
Repeated discovery responses establish consumption/recovery without selecting a
service; active probes use correlated tools/list before injecting either line.
No arbitrary sleeps or stolen stdin reads are used. Exact final stderr comparisons
exclude host reads and extra diagnostics. The existing EOF observer in startup
mode admits only its fixed INFO/WARN lifecycle events; active `input-errors` mode
has no subscriber. Clean stderr therefore does **not** qualify raw parser/dispatch
log redaction. Source inspection still identifies raw DEBUG input and service
DEBUG/TRACE payload logging as a separate shipping concern. No generic error,
panic, diagnostic or production sanitizer is introduced.

The shared `read_response_line` remains byte-for-byte unchanged and still requires
an ID. A separate checker permits only the exact fixed error envelope above:
full JSON value equality rejects any ID (including null), extra top-level fields,
result, changed version/code/message or any data field. It retains the 64 KiB
newline bound, rejects empty output and parses every observed line. Its regression
test accepts the expected line and rejects twelve malformed/framing/envelope
variants. Ordinary responses and final EOF still use the existing shared checker.
The [stdio binding](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio)
requires protocol-only stdout and client shutdown by closing input then waiting;
these probes use real pipes, actual EOF and bounded actual child waits. The
five-second watchdogs remain test limits, not production shutdown guarantees.

TDD: with the new checker initially delegating to the unchanged correlated-response
checker, both structural subprocess probes and its acceptance test fail at the
required-ID assertion (3 RED); syntax characterization already passes (2 tests).
Implementing the separate exact checker makes all five pass. A mutation retaining
framing/JSON/version checks but removing full envelope equality fails the checker
regression at an incorrectly accepted null-ID error; the original file is restored
before the final full suite. No fixture or production implementation changes.

Validation: locked full spike suite **45 pass** (11 adapter, 5 error boundary,
5 HTTP, 4 lifecycle, 3 negotiation boundary, 17 stdio); strict all-target Clippy,
formatting and whitespace checks pass. No dependency/lock/binary-source changes;
root/core/desktop/native Windows/macOS/MSRV/UI/release suites are not rerun for this
test-only scope. Ripwire checker impact/uses identify only the new probe helpers
and regression test; affected selection identifies the new test module with an
unknown runner, so Cargo executes it plus the full spike. Quality delta exits 0,
with five non-gating new-symbol dead-code findings for the five Cargo-executed
tests; no existing-symbol regressions or suppressions/baseline changes. Edit-check
reports the checker as a new symbol with no incompatible uses. The staged static
test gate exits 4: twelve unknown-runner test files/modules (all executed by Cargo),
unchanged desktop `run_boundary_oidc_automation` and two unrelated documentation
headings. This is not a static/native coverage claim.

Next focused increment: characterize raw SDK DEBUG parser/dispatch diagnostics
using an explicit test-only subscriber with synthetic sentinels, fixed stage
observations and strict stdout/actual process completion checks. Keep that raw
observation separate from the fixed EOF observer and a later production logging/
error policy. Backpressure, signals, native Windows/macOS stdio and hard native
cleanup remain unqualified. HTTP shipping policy, production ownership, coordinated
CLI import, other operation contracts and packaging remain independently actionable.
Do not repeat unchanged cgroup qualification.
