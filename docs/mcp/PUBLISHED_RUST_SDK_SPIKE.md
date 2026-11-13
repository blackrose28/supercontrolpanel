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
