# Standalone stdio diagnostics: reviewed boundary

Status: source-backed contract review, 2026-11-13, baseline `ef1e88b`.
**Not implemented or production-qualified.** The next increment is an isolated
policy fixture with behavioral RED/GREEN tests. `scpctl` has no MCP command;
desktop/custom HTTP hosting and the raw SDK fixtures remain unchanged.

## Evidence and boundary selection

Reviewed the installed published sources matching
[`Cargo.lock`](../../spikes/mcp-sdk/Cargo.lock): rmcp **3.10.0**, tracing **0.1.44**,
tracing-core **0.1.36**. Paths below are inside those exact crate sources, not
claims about later releases. Repository evidence remains in the
[raw diagnostic review](PUBLISHED_RUST_SDK_SPIKE.md#raw-stdio-debug-diagnostic-characterization-2026-11-13).

| Source | Consequence for a future executable |
| --- | --- |
| rmcp `transport/async_rw.rs::try_parse_with_compatibility` | DEBUG formats the complete failed input line and serde error. Discarding structured password fields cannot remove values embedded in message text |
| rmcp `service.rs::serve_inner` | DEBUG records requests, peer errors and results. INFO records peer info and notifications; cancellation events include peer reasons. WARN records rejected request messages/response errors; ERROR records transport/task failures. Raising the minimum level to INFO/WARN/ERROR is insufficient |
| rmcp `service/server.rs::ServerInitializeError` | Non-exhaustive typed enum retains messages/results/transport errors. Both derived Debug and Display can include supplied/native data. `ExpectedInitializeRequest(Some(Error(...)))` retains the entire unsolicited peer error |
| rmcp `service.rs::RunningService::waiting`, `QuitReason` | Outer Result can fail with JoinError; an Ok result can itself contain `QuitReason::JoinError`. Neither may be treated as clean completion or formatted into diagnostics |
| tracing `subscriber.rs`; tracing-core `subscriber.rs::NoSubscriber` | Global installation is fallible and applies as the fallback across threads; thread-local defaults override it. NoSubscriber rejects every callsite, span and event and records no fields. Severity/target filtering and a Debug visitor are unnecessary for a deny-all boundary |

The existing [fixture main](../../spikes/mcp-sdk/src/bin/stdio-fixture.rs) returns
`Result<(), Box<dyn Error>>`; input-errors/raw-debug deliberately forward the typed
startup error through `?` to Rust's raw termination diagnostic. The
[startup test](../../spikes/mcp-sdk/tests/stdio/input_errors.rs) pins its full
synthetic contents. The [raw observer](../../spikes/mcp-sdk/src/bin/stdio_fixture/raw_observer.rs)
deliberately records values. Neither changes meaning under this review.
The [EOF observer](../../spikes/mcp-sdk/src/bin/stdio_fixture/eof_observer.rs)
formats a message field to select fixed markers: it is a narrowly characterized
test instrument, not the production subscriber proposed here.

The production [CLI main](../../crates/scp-cli/src/main.rs) already returns
ExitCode, but `run` returns String and main prints it. Core profile/runtime readers
also have String errors; credential and MCP policy errors are typed with separately
defined fixed messages. Those human CLI contracts stay intact. A future MCP host
must intercept failures before this ordinary CLI formatter; converting an SDK error
to String first loses the typed boundary. The isolated
[overview adapter](../../spikes/mcp-sdk/src/overview.rs) discards returned host
strings and emits fixed errors, but does not control SDK diagnostics or other core
operations. Do not move SDK dependencies/subscribers into scp-core or desktop.

## Selected executable policy

Before constructing an SDK transport, adapter, runtime workers or accepting stdin,
the future standalone MCP executable must install exactly one process-global
`tracing::subscriber::NoSubscriber`. This first policy emits **no tracing events or
spans at any target or level**, including the executable's own events. It does not
parse `RUST_LOG`, honor verbose/debug switches in MCP mode, install a formatting
layer, bridge a second logger, or add a file sink. Trusted executable code must not
install thread-local dispatchers, attach alternate dispatchers to tasks or replace
this policy. Global installation failure stops before transport/service creation;
never continue under a pre-existing unknown subscriber. Install from the executable,
not a reusable library. A closure-scoped `with_default` around construction does
not establish a policy for a multi-thread async runtime.
Use a synchronous ExitCode main to install the policy before building/entering
the async runtime; a Tokio main attribute starts its runtime before the function
body and is not evidence of this ordering. Runtime construction failures take the
same fixed startup failure path.

This is suppression at the selected tracing dispatcher, **not a sanitizer**.
Direct stdout/stderr writes, another logging facade, native child output, panic
hooks, crash reports and hostile Rust code lie outside it. Dependencies and future
host code must be audited for those routes before activation. NoSubscriber cannot
undo a diagnostic emitted before installation. There is no generic panic/native/
success-data secrecy guarantee, log-volume/backpressure guarantee or hard shutdown
deadline. The [native lifetime gates](LINUX_OPERATION_SUPERVISION.md) still apply.

Stdout belongs exclusively to SDK protocol frames. Do not print banners, status,
human/JSON CLI results, diagnostics, fixture markers or a synthetic JSON-RPC reply
for an error without a request correlation authority. In particular, a startup peer
error or EOF is a process failure, not a newly invented response. IDs remain wire
correlation values; this policy does not sanitize successful payloads or IDs.

Owned top-level diagnostics use literal bytes on stderr, with no Debug/Display,
error chain/source, arbitrary context, profile/path, token or peer metadata:

| Outcome | Exact stderr line (LF terminated) | Process result |
| --- | --- | --- |
| Host setup/subscriber installation failed, or any SDK startup Result error (including EOF or cancellation before service selection) | `scpctl: MCP stdio startup failed` | Failure (intended exit 1); no new protocol response |
| Active outer waiting JoinError, inner QuitReason::JoinError, unknown future quit variant, or unexpected cancellation | `scpctl: MCP stdio service failed` | Failure (intended exit 1), after required owned cleanup |
| Closed, or a cancellation explicitly requested by the owning host with settled cleanup | No diagnostic | Success only within separately qualified lifecycle/cleanup scope |

Report at most one selected top-level line. Use fallible stderr writing and retain
failure status if the sink fails; do not `unwrap`, panic, recursively report the I/O
error or fall back to stdout. A blocked stderr write can still block: this contract
does not promise timed process exit. A sink failure test can qualify error handling,
not guarantee delivery. Closed alone is not native cleanup evidence, and startup
closure does not imply that an externally supplied cancellation token was cancelled.

Keep original typed SDK errors internal until classification/drop, without invoking
their formatting or traversing source chains. Use a private host outcome/error type
to distinguish setup/startup/service failures; no public core error conversion or
serialized error contract is required. Match known non-exhaustive variants with a
fail-closed fallback. Preserve the original startup error for internal typed tests,
but return ExitCode at the process boundary, never `Result<_, SDK error>` or a box
that Rust Termination can Debug-print. Do not catch panics and relabel them as
sanitized failures. Task panics can already invoke panic hooks before a JoinError
exists; fixed handling of that JoinError cannot retract those bytes.

## Acceptance and next implementation slice

These are requirements, not results of this docs-only increment. Keep policy
subprocesses separate from raw-debug/input-errors/startup-eof and the fixed EOF
observer. Test sentinels are synthetic; do not load profiles, real credentials,
native runtime operations or desktop state.

| Boundary | Required behavioral evidence |
| --- | --- |
| Next slice: startup forwarding | New isolated policy mode: unsolicited idless peer Error with message/data sentinels retains its actual typed ExpectedInitializeRequest error internally; exact single fixed startup line, no raw Termination suffix, empty protocol stdout and unsuccessful actual child.wait |
| Next slice: preselection EOF | No input and modern discover-then-EOF each fail with the same fixed startup line; discover retains its exact correlated result and does not select service. Legacy initialize reaches service and closes normally; do not infer this from discovery alone |
| Next slice: tracing suppression | With `RUST_LOG=trace` and targeted rmcp directives, malformed numeric-method input containing a sentinel still gets the exact idless -32600/Invalid request/no data; active peer Error produces no reply; malformed string arguments get correlated -32601/MCP method not supported then catalog recovery. Exact stderr excludes SDK records at every stage; every stdout line checked through EOF |
| Next slice: level/span reach | Test-only emissions with synthetic fields at TRACE/DEBUG/INFO/WARN/ERROR and spans, on main and spawned worker tasks, yield no bytes under the installed policy. Real parser/request probes above establish SDK paths; absence of DEBUG alone is insufficient |
| Next slice: setup conflict | Separate process preinstalls a silent test subscriber; policy install fails before any SDK input/transport/service/host entry. Fixed startup line and failure; no unknown subscriber debug formatting |
| Later before activation: internal outcome/error writer | Test outer waiting error, inner JoinError and non-exhaustive fallback, expected vs unexpected cancellation, fallible diagnostic sink failure, exact single-line precedence and cleanup ownership. Avoid panic payload assertions as proof of panic sanitization |
| Preservation | Existing raw-debug tests still require original sentinel values; original typed raw startup and EOF observer tests retain their exact diagnostics. Raw characterization is never rewritten into suppression evidence |

Implement the **five next-slice rows only** in the isolated SDK package, starting
with failing subprocess expectations and then a separate policy runner/module.
Reuse pinned NoSubscriber rather than invent a visitor/filter. Add a dedicated
policy binary if needed to keep its ExitCode boundary separate from the raw
fixture's Result main; do not route raw modes through it. The worker/level probe
and setup-conflict injection are fixture-only seams. Keep lifecycle proof via wire
catalog/initialization and actual child.wait, concurrent bounded stderr drainage
and watchdogs; watchdog expiry is failure, not a cleanup guarantee. Refactor only
after GREEN, run the full isolated spike suite plus locked strict Clippy/fmt, and
retain existing wire checkers. No production activation in that increment.

## Independent HTTP and release gates

The [HTTP experiments](PUBLISHED_RUST_SDK_SPIKE.md) remain independent:
with_fixed_http_errors erases all 4xx/5xx bodies, including modern 404 JSON-RPC
envelopes, and is not the final standards-preserving policy. The custom-request
override preserves native code/status with fixed -32601/no data. The negotiation
wrapper recognizes only selected exact finite 400 JSON envelopes up to 64 KiB;
unknown/oversized bodies pass through. Composing those wrappers still erases modern
envelopes in either order. Stdio stderr suppression chooses no HTTP parser/header/
session/body policy and does not authorize those wrappers for production. Preserve
all six custom localhost/bearer HTTP routes and require-ui-approval behavior.

Production stdio remains gated on host authority, lifecycle/admission/cleanup,
typed active outcomes, diagnostic sink and non-tracing output audit, native
Windows/macOS qualification and other existing release checks. Settings import
still follows the controlling [admission contract](CLI_IMPORT_ADMISSION.md) and
[maintenance lifetime audit](CLI_MAINTENANCE_LIFETIMES.md). No SDK upgrade,
maintenance/recovery authority, command, schema or desktop/CC/admin contract changes
are approved by this review. Independent HTTP shipping, other operations and
packaging work remains actionable.
