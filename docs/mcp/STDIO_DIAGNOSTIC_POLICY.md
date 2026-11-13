# Standalone stdio diagnostics: reviewed boundary

Status: source-backed contract review, 2026-11-13, baseline `ef1e88b`;
five-row isolated policy fixture implemented against review commit `3a7e702`;
internal outcome/writer qualification extends baseline `cfb7c33`.
**Not production-qualified.** Behavioral RED/GREEN evidence is recorded below.
`scpctl` has no MCP command; desktop/custom HTTP hosting and the raw SDK fixtures
remain unchanged.

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

The five initial rows and the internal outcome/writer slice now pass in a separate
policy executable within the limits below; production requirements remain.
Raw-debug/input-errors/startup-eof and the fixed EOF observer retain
their original semantics. Test sentinels are synthetic; no profiles, real
credentials, native runtime operations or desktop state are loaded.

| Boundary | Required behavioral evidence |
| --- | --- |
| Implemented isolated: startup forwarding | New isolated policy mode: unsolicited idless peer Error with message/data sentinels retains its actual typed ExpectedInitializeRequest error internally; exact single fixed startup line, no raw Termination suffix, empty protocol stdout and unsuccessful actual child.wait |
| Implemented isolated: preselection EOF | No input and modern discover-then-EOF each fail with the same fixed startup line; discover retains its exact correlated result and does not select service. Legacy initialize reaches service and closes normally; do not infer this from discovery alone |
| Implemented isolated: tracing suppression | With `RUST_LOG=trace` and targeted rmcp directives, malformed numeric-method input containing a sentinel still gets the exact idless -32600/Invalid request/no data; active peer Error produces no reply; malformed string arguments get correlated -32601/MCP method not supported then catalog recovery. Exact stderr excludes SDK records at every stage; every stdout line checked through EOF |
| Implemented isolated: level/span reach | Test-only emissions with synthetic fields at TRACE/DEBUG/INFO/WARN/ERROR and spans, on main and spawned worker tasks, yield no bytes under the installed policy. Real parser/request probes above establish SDK paths; absence of DEBUG alone is insufficient |
| Implemented isolated: setup conflict | Separate process preinstalls a silent test subscriber; policy install fails before any SDK input/transport/service/host entry. Fixed startup line and failure; no unknown subscriber debug formatting |
| Implemented isolated: internal outcome/error writer | Real nonpanic task-abort JoinErrors injected separately into outer waiting Result and inner QuitReason::JoinError; actual SDK cancellation by external versus explicit owner token; settled synthetic cleanup gates success, cleanup failure wins; literal single-line reporter, immediate/partial/zero-write sink failures, and process no-stdout-fallback. Non-exhaustive fallback reviewed structurally, no future variant constructed. No native cleanup or panic sanitization claim |
| Preservation | Existing raw-debug tests still require original sentinel values; original typed raw startup and EOF observer tests retain their exact diagnostics. Raw characterization is never rewritten into suppression evidence |

The [policy binary](../../spikes/mcp-sdk/src/bin/stdio-policy-fixture.rs) uses a
synchronous ExitCode main, installs the pinned NoSubscriber before building its
multi-thread runtime, and only then constructs stdio_pipes and the adapter. It
never reads RUST_LOG or installs another dispatcher. Private failures distinguish
setup, boxed original typed SDK startup error, and active failure. Boxing retains
the actual enum without formatting it; the process drops it after classification.
The fallible stderr write ignores its I/O error and retains failure status, with
no recursive diagnostic or stdout fallback. Writer faults are qualified below.

The [typed unit test](../../spikes/mcp-sdk/src/bin/stdio_policy/runner.rs) directly
matches ExpectedInitializeRequest(Some(Error(...))) and checks absent ID, code,
message and password data, without invoking the error's Debug/Display. The
[subprocess tests](../../spikes/mcp-sdk/tests/stdio/policy.rs) check exit 1 and the
exact startup line for unsolicited errors and both preselection EOF cases. Modern
discovery is compared as a complete correlated envelope; legacy initialize followed
by EOF closes normally even without notifications/initialized.

Active tests run both lifecycles under RUST_LOG=trace and targeted rmcp directives.
Catalog replies establish actual service selection, then numeric method 17 gets
only the exact idless -32600/Invalid request error; an idless peer Error produces
no reply before the correlated fixed custom-request error and catalog recovery.
A fixture-only host tripwire detects unintended invocation. Stderr must be empty;
the unchanged correlated checker and exact idless checker inspect every stdout
line through EOF. Concurrent stderr drainage starts before input and is bounded
to 64 KiB with a watchdog; actual child.wait is checked, not inferred from pipe
closure. The five-second watchdog is a test failure boundary, not a lifetime promise.

The fixture-only levels mode emits TRACE/DEBUG/INFO/WARN/ERROR events with synthetic
fields, enters/records spans at each level, and checks disabled callsites/spans on
the synchronous main thread and an awaited spawned Tokio worker task. It also runs
the real wire probes. Conflict mode preinstalls silent NoSubscriber; the required
second install fails. A tripwire immediately after installation precedes runtime
construction and therefore transport/input/service/host entry. The child must exit
1 with only the fixed startup line while its parent retains open stdin, so even
waiting for input fails the watchdog. This does not qualify arbitrary preexisting
subscriber behavior before installation, alternate dispatchers, or panic hooks.

TDD: all five subprocess tests first failed against a success-only placeholder
(missing replies or incorrect successful exit), and the typed unit test separately
failed at the missing original error. Logs: /tmp/scp-policy-red.log and
/tmp/scp-policy-typed-red.log. GREEN preceded refactoring: shared active matrix,
exact discovery envelope, removal of a racy conflict-test write, and boxing the
368-byte SDK error after strict Clippy identified the private enum size.
Full locked spike: **53 tests**, including 24 stdio integration tests and the typed
policy unit test. Strict all-target Clippy, fmt and whitespace checks pass. Raw
DEBUG tests still require original synthetic parser/request passwords and peer
message/data, and raw startup/EOF observer tests retain exact diagnostics.
No dependency, lockfile, schema, production core/CLI/desktop or HTTP changes.
Unchanged root/native/UI/MSRV/release suites were not rerun; no cgroup reprobe.

### Internal outcome and diagnostic writer qualification (2026-11-13)

The private runner now awaits synthetic owned cleanup before returning any active
outcome. Closed succeeds only with successful cleanup; Cancelled also requires an
explicit owning-host request. Cleanup failure yields the fixed service failure,
even for Closed or requested cancellation. The ordinary executable still passes
false and owns no native work. No new cancellation command or host authority is
exposed. A channel-gated cleanup test proves no completion before cleanup release;
a failed cleanup overrides otherwise successful completion. This is not a native
teardown, cancellation deadline or crash-retention guarantee.

Separate tests create actual Tokio JoinErrors by aborting pending nonpanic tasks,
check is_cancelled and !is_panic, then inject them at the exact two SDK result
layers consumed by the runner: Err(JoinError) and Ok(QuitReason::JoinError).
Both fail after owned cleanup, regardless of the owner-request flag, and report
exactly one service line. These are boundary-injection tests, **not evidence that
an actual RunningService's private task handle was aborted**. A real initialized
SDK service is separately cancelled through its externally supplied token and
through its owning service token. Both yield actual QuitReason::Cancelled; only
the explicitly requested owner case succeeds after synthetic cleanup. Token state
alone does not establish who requested shutdown. Panic hooks may emit before any
JoinError exists; none of these tests qualifies panic sanitization.

Pinned rmcp 3.10.0 exposes exactly Closed, Cancelled and JoinError in its
non-exhaustive enum. All known variants are covered. The runner's only success
arms name Closed and explicitly requested Cancelled with successful cleanup;
the wildcard returns service failure for every other value. That fallback is
source-reviewed and compiled downstream. An unknown future SDK variant cannot be
constructed honestly with this pin, so no runtime coverage of such a variant is
claimed. No unsafe construction, counterfeit SDK variant or dependency change.

The literal reporter now takes a private Write sink; ordinary reporting still uses
stderr.lock after runtime drop. Unit tests select setup/startup/service lines
(including an original typed startup error with synthetic context), and require
silence on success. Immediate I/O errors, partial writes followed by errors, zero
writes and partial-then-zero writes retain ExitCode::FAILURE. Exact prefix and
write-call counts exclude recursive reporting; the injected sink's error text is
never formatted, and no flush/panic is triggered. A separate subprocess injects a
refusing sink after the actual unsolicited startup peer error: actual child.wait
is exit 1, stdout is empty through EOF and stderr is empty. Sink failure can lose
the entire line or leave a partial prefix; this qualifies error handling, not
successful delivery or a bound on blocking.

Behavioral RED: six unit rows fail against the minimal outcome seam and success-only
writer placeholder (/tmp/scp-outcomes-red.log), including skipped cleanup,
incorrect cancellation and incorrect exit status. The process writer test separately
fails because the original reporter still writes its startup line
(/tmp/scp-writer-process-red.log). GREEN precedes refactoring the duplicate
subprocess startup exchange into one helper and adding real SDK cancellation
coverage. The original typed startup test remains unchanged. Final verification
and static-analysis results are recorded in the
[SDK evidence](PUBLISHED_RUST_SDK_SPIKE.md#isolated-active-outcomes-and-diagnostic-writer-faults-2026-11-13)
and [plan increment](SETTINGS_BACKUP_STREAMABLE_HTTP_STANDALONE_CLI_PLAN.md#isolated-active-outcomes-and-writer-faults-2026-11-13).

This completes the focused isolated acceptance slice, with the non-exhaustive
runtime limit stated explicitly. Production activation still requires host authority,
lifecycle/admission/cleanup and non-tracing output audits. Independent HTTP shipping
policy, operations, serialized import/recovery and packaging remain actionable.

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
production integration of typed active outcomes and diagnostic sinks, non-tracing
output audit, native Windows/macOS qualification and other existing release checks. Settings import
still follows the controlling [admission contract](CLI_IMPORT_ADMISSION.md) and
[maintenance lifetime audit](CLI_MAINTENANCE_LIFETIMES.md). No SDK upgrade,
maintenance/recovery authority, command, schema or desktop/CC/admin contract changes
are approved by this review. Independent HTTP shipping, other operations and
packaging work remains actionable.
