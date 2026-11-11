# Settings backup, standards-based MCP, and standalone CLI plan

Status: Work 1 desktop implementation added; native release qualification remains. Work 2 is proposed. Work 3 has a headless workspace, shared config and device-local settings schemas, desktop MCP discovery metadata and shared fail-closed execution policy/read-only shared file projection and config-show CLI, inventory/profile CLI, profile-bound credentials, and shared function-status state/transitions, and PTY session handles/launch/I/O/close with launch-bound exit polling, and a profile-owned runtime context with synchronous output drainage and scoped cooperative foreground cancellation; shared Caddy launch preparation and a Linux-only foreground Caddy CLI host with signal cancellation and file output are implemented; hard shutdown bounds/process-tree supervision, Windows/macOS host qualification, other operation execution and stdio MCP remain.
Assessment and review date: 2026-11-11.

## Requested outcomes and feasibility

| Work | Verdict | Main work / constraint |
| --- | --- | --- |
| Desktop buttons to export/import all settings, including stored passwords | Viable | Versioned encrypted backup; enumerate app-owned credentials; reliable OS-keyring reads and recoverable restore |
| Complete desktop MCP with Streamable HTTP | Viable | Replace custom HTTP routing with standard MCP; preserve tool behavior, authorization, and compatibility |
| Standalone CLI with its own stdio MCP server | Viable, largest effort | Extract a Tauri-independent runtime; define process lifetime and desktop-only capability behavior |

The CLI must work with the desktop app closed. It will execute the shared runtime itself, not proxy requests to the desktop HTTP endpoint. Its MCP mode will serve that runtime over stdio. Ordinary shell commands retain human-readable/JSON output; stdio is specifically the MCP server mode.

The initial feasibility findings came from source inspection and protocol documentation. Implementation progress is recorded below; cross-platform keyring permissions, headless packaging, and SDK integration still require validation before estimates can be firm.

## Current implementation and evidence

- `src-tauri/src/commands/config.rs`: config load/save, plaintext migration, local settings patches, enrollment, and sync actions.
- `src-tauri/src/services/config_service.rs`: app-owned keyring account names, direct `keyring_load_from_os`, write-through `keyring_set_or_delete`, local settings, and initialization.
- `src-tauri/src/stores/keyring_cache.rs`: process-local secret cache. `initialize_keyring_cache` in `services/config_service.rs` currently skips missing credentials and read failures; a backup must distinguish these cases.
- Existing config loaders can migrate and rewrite files, and `save_app_config_action` preserves existing passwords for empty inputs. Neither is a suitable read-only preview or exact-replacement import API. Current file writes also do not provide the proposed coordinated transaction.
- `src-tauri/src/cc/telemetry_queue_crypto.rs`: the queue key is cached in a live cipher; replacing the stored key alone does not safely reconfigure telemetry.
- `package.json`, `playwright.config.ts`, and `tests/`: desktop frontend tests use Playwright; the root package has no `npm test` script. `src-tauri/build.rs` unconditionally invokes Tauri build tooling.
- `src-tauri/src/models/config.rs`: secret-bearing runtime models versus secret-free disk models. Copying `config.json` alone cannot export passwords.
- `src-tauri/src/server_mgmt.rs`: server records; SSH secrets use dynamic `server_ssh_password:{serverId}` accounts.
- `src-tauri/src/mcp/server.rs`: hand-written TCP/HTTP server, bearer token, localhost restriction, custom REST routes.
- `src-tauri/src/mcp/tools.rs` and `resources.rs`: substantial existing operation coverage, but descriptors/results and dispatch depend on Tauri and custom envelopes.
- `src-tauri/src/bin/supercontrolpanelhelper.rs`: existing CLI is an HTTP client requiring the desktop app.
- `scp-core` now shares server inventory, CLI profile ownership/identity, profile-bound credential/cache adapters, function status state/transitions, PTY event payload types, and session handles/launch/I/O/close with launch-bound exit polling. `src-tauri/src/main.rs`, `services/pty_service.rs`, and `services/boundary_service.rs` still host runtime monitoring/IDs, event transport/telemetry, and automated OIDC through Tauri.

If adopted, this proposal replaces the helper-only direction in `MCP_HELPER_CLI_SPEC.md` and `MCP_TRANSPORT_AND_AUTH.md` for the new CLI. Those documents still describe the existing desktop-client helper. Keep that helper functional during migration; desktop control remains useful even after a standalone runtime exists.

## Shared architecture

### First standalone extraction increment (2026-11-11)

The root Cargo workspace now contains `crates/scp-core` and `crates/scp-cli`. The desktop
package is explicitly excluded from this workspace and depends on core by path, so its
existing Tauri build script and build commands stay local to `src-tauri`. The CLI's normal
and build dependency graph contains no Tauri, GTK/WebKit, HTTP client, or frontend build.

`scp-core::servers` owns the existing server types and a read-only inventory reader with
an explicit config directory. Desktop `server_mgmt` re-exports the types with the existing
field names/defaults and retains its configuration gate, paths, and writers. Desktop
command names, event payloads, and the helper's HTTP behavior are unchanged.

`scpctl server list [--json] [--config-dir DIR]` directly reads this profile's inventory.
The default directory is the OS config directory plus `supercontrolpanel-cli`, separate
from desktop configuration. Reads do not initialize a runtime, migrate files, touch the
keyring, or connect to desktop/CC/MCP. Missing inventory is empty; parse/I/O failures use
stderr and exit 1, invalid arguments exit 2. JSON success output is an array of servers.

TDD evidence: all five CLI subprocess tests failed against an empty CLI, and the core
contract test failed because the shared module did not exist. After implementation, five
CLI tests and four core tests pass on Linux. They cover legacy/full server serialization,
profile selection, read-only/missing inventory, malformed and unreadable files, output,
exit codes, help/version, and disabled HTTP settings. The root lockfile pins the headless
workspace dependencies. Linux validation also passed the 101-test desktop Rust suite,
strict workspace Clippy, formatting, and a standalone release build. Ripwire quality delta
reported no gating regressions; its nine new dead-code findings are the test functions,
which Cargo successfully executed. Native Windows/macOS checks remain required.

This establishes an initial dependency/build seam, not full runtime parity. The next
increment below adds profile initialization, a per-profile credential namespace and
runtime-owner lock before any credential access, configuration writes, or long-running commands. Then extract runtime
stores/events and one operation family with foreground child cleanup. SDK interoperability,
desktop Streamable HTTP, stdio MCP, settings commands, shell completion, and release
packaging remain open. The inventory preview deliberately has no `status` command: it
does not yet own runtime sessions and cannot report another process's memory.

### CLI profile identity and ownership increment (2026-11-11)

`scp-core::profile::ProfileOwner::acquire` now creates/validates versioned local
`profile.json` metadata under an exclusive, nonblocking OS file lock. A random UUID v4
namespace is generated once per profile for future credential service/cache keys.
Metadata is published through a private temporary file without overwriting an existing
identity. Malformed/unsupported metadata fails closed; initialization never reads the
keyring or desktop settings. New Unix directories/files use owner-only permissions;
Windows inherits directory ACLs. Existing directory permissions are not changed.

`scpctl profile init [--json] [--config-dir DIR]` exposes this foundation. It returns
the stable `credentialNamespace`, or exit 1 with an actionable ownership conflict.
The guard owns `runtime-owner.lock` until dropped; future runtime/config writers must
hold it for their entire lifetime. The file remains on disk and must not be unlinked.
Kernel ownership releases on normal exit or forced process termination, avoiding stale
PID-file recovery. Read-only inventory does not acquire the lock or initialize metadata.
The file-lock API requires Rust 1.89+. This is per-profile coordination, not host-wide
VPN isolation, and does not introduce desktop/CLI shared profiles.

TDD evidence: the profile integration test initially failed to compile because the
module was absent; the CLI initialization test failed with unknown subcommand/exit 2.
The implemented tests cover stable/distinct identities, invalid metadata preservation,
private Unix permissions, competing subprocess ownership, release on EOF/forced exit,
CLI output/errors, idempotency, and read-only inventory during ownership. Linux checks
passed: 7 CLI tests, 8 core integration tests (plus the ignored fixture exercised by the
ownership test), all 101 desktop tests, strict workspace Clippy, formatting, and a locked
standalone release build. Ripwire quality delta has zero gating regressions; its seven
new dead-code findings are Cargo test functions. Its test gate reports a Markdown heading
as untested and does not model all subprocess coverage; actual Cargo results are the
verification evidence. The desktop's ignored local lockfile needed offline refresh after
the core dependency change. Native Windows/macOS lock behavior still needs testing.

The next increment below adds namespace-aware secret-store/cache adapters. Runtime
stores/events and one operation family with foreground child cleanup remain next.
Settings import must retain the target identity and never copy the archive's namespace.

### Profile-bound credential adapters (2026-11-11)

`scp-core::credentials` now provides an injectable `SecretBackend`, explicit known
credential accounts, a `ProfileSecrets` adapter borrowing a live `ProfileOwner`, and
an owned `CredentialCache` keyed by `(credential namespace, account)`. No CLI operation
can choose a desktop namespace through this adapter. The desktop's fixed identifiers,
global cache, commands, and backup secret-store trait remain unchanged; conversion of
desktop callers is a separate characterization/refactoring slice.

Account mappings cover OpenVPN password/OTP, Boundary password/OIDC password/OIDC OTP/
app token, and dynamic SSH passwords. Dynamic server IDs must be 1–256 ASCII letters,
digits, hyphens, underscores, or dots; invalid accounts fail before touching the backend.
This validates credential addresses only and does not change desktop/inventory parsing.

Only successful reads (including missing entries) populate the cache. A fresh read
evicts the old value before consulting the backend; writes evict first and populate
only after success, so unknown native write outcomes cannot return stale cached values.
Hosts explicitly refresh after external maintenance or clear the cache. `None` deletes
idempotently; `Some("")` is an explicitly empty secret, distinct from missing. Future
adapters for desktop's empty-string-as-delete convention must translate at the boundary.
Values redact Debug output and zeroize their owned buffers when dropped. Typed errors
retain no native error payloads: locked/unavailable storage, invalid account/data, and
ambiguous matches are distinct from `Ok(None)` for a missing credential.

The opt-in `scp-core/os-keyring` feature implements `OsSecretBackend` with keyring v3
and the desktop's persistent native platform features. The default CLI dependency graph
does not include keyring or D-Bus/GUI dependencies. Linux native-feature builds require
the D-Bus development library/pkg-config. Calls are synchronous and may prompt/block;
bounded workers and host interaction/cancellation must be supplied before user-facing
commands or MCP call them. No real credential operations were performed in validation.

TDD evidence: credential integration tests first failed because the core module/API
was absent. Injected-backend tests cover profile/account isolation, explicit account
mapping, cached misses, refresh, errors/retry, failed writes/deletes, empty/missing
semantics, and redaction. Native entry tests use per-entry keyring mocks without
changing the process-global native builder. Native error tests verify sanitized
classification. A compile-fail doctest checks that the owner cannot be dropped while
the profile adapter is still used. Linux validation passes all 24 workspace tests with
native features, the compile-fail doctest, all 101 desktop tests, strict Clippy, formatting,
and a locked standalone release build. The default dependency tree excludes keyring,
D-Bus, Tauri, and webview dependencies. Ripwire quality delta reports zero gating
regressions; remaining findings are new trait/test entry points and the small native
entry-construction boilerplate. Its test gate still flags the unrelated settings-backup
Markdown heading, so Cargo results provide the executed coverage evidence.
Native OS unlock/persistence and Windows/macOS checks
remain unvalidated; these mocks establish the adapter contract, not platform qualification.

The next increment below extracts function-status state and transitions. PTY ownership,
lifecycle context, and an operation family with foreground child cleanup remain next.
Integrate the credential adapter only behind bounded host capabilities; settings import
and CLI credential commands remain unimplemented.

### Shared function-status state and exit transitions (2026-11-11)

`scp-core::runtime` now owns `FunctionStore`, `FunctionStatus`, `PtyOutputPayload`, and
`PtyExitPayload`. Desktop modules re-export these same types, preserving all command
signatures, event names, camelCase payload fields, explicit nulls, and the initial three
idle statuses. The default store initialization, clock, and mutex-error helper moved to
core; desktop consumers retain their existing helper imports. The status map remains
public during migration so current desktop writers use the same mutex/state.

The core's `publish` commits the status before calling a host-provided typed event sink
outside the mutex. Desktop adapters retain Tauri delivery and CC session-ID translation.
`finish_session` applies the existing exit rules and returns owned status events after
releasing its lock; the desktop still runs OpenVPN reconnect handling before emitting
those events and retains its existing exit-telemetry behavior. Normal exits keep their
session IDs for log queries and become stopped/error; Boundary clears its auth session,
preserves App/CLI token-source reporting, and retains the existing verified-token and
unverified-login messages.

Boundary token lookup now runs outside the status lock. Core rechecks the current
session after lookup so a late exit cannot overwrite a replacement session. Long
Boundary error reasons keep the existing 80-byte limit/ellipsis for ASCII and truncate
on a valid UTF-8 boundary for Unicode, fixing the previous byte-slicing panic. Snapshots
are sorted, owned copies of this store's state; no cross-process status view is implied.

TDD evidence: core runtime tests first failed on the absent module; a desktop migration
test failed on separate store/payload types before re-exports; the shared lock-helper
test failed before that helper moved. Tests cover exact JSON, default statuses, publication
after commit/outside locks, all regular/Boundary exit outcomes, history preservation,
token lookup outside locks, replacement-session races, Unicode boundaries, and poisoned
mutexes. Desktop migration and existing CC/OpenVPN/tunnel tests exercise the adapters.
Linux validation passes 34 workspace tests with native features, the credential lifetime
doctest, all 102 desktop tests, strict workspace Clippy, formatting, and a standalone
locked release build. Ripwire quality delta reports zero gating regressions; its new
dead-code findings are test entries plus the string-formatting helper reached by tests.
The static test gate still exits 4: moving shared types/the generic lock helper exposes
a broad pre-existing desktop caller surface (113 symbols) that its graph does not mark
covered, despite the executed core/desktop tests. This is not a clean coverage gate;
native UI/PTY/CC interaction smoke tests and Windows/macOS qualification remain open.

This increment established a status/event boundary, not a standalone process runtime.
The next increment below adds session handles/launch/I/O/close. Monitoring, child
supervision, cancellation/shutdown, host runtime IDs, and CLI runtime commands remain
open. No GUI/keyring/native dependency was added by the status-store slice.

### PTY session ownership and launch cleanup (2026-11-11)

`scp-core::sessions` now owns the desktop-compatible `SessionStore` and `PtySession`,
with injectable `portable_pty::PtySystem` launch, input/flush, removal, and close.
Desktop re-exports the same types and delegates these operations; its settings gate,
ID generation, watcher delivery, reader/monitor threads, Tauri events, SCP completion,
function-exit callbacks, and CC telemetry/session mapping remain host adapters. Public
store/handle mutexes remain temporarily available for existing desktop consumers; this
is not yet an encapsulated CLI runtime API.

Launch prepares the reader and writer before starting a child and checks duplicate IDs
before platform work. Registration rechecks atomically without replacing an existing
session. If registration loses a race or its mutex is poisoned, the unregistered owner
attempts to kill and reap its child outside the registry lock. Last `PtySession` owner
drop also attempts cleanup, recovering a poisoned child mutex; retained `Arc`s retain
ownership. Explicit close keeps `closed_by_command`, exit-code conversion, missing-session
idempotency, watcher removal, and existing I/O error strings. No command/event schema changed.

A full workspace run exposed transient profile-lock retention when another test forks
while a profile descriptor is open. A deterministic duplicated-descriptor regression
failed before the fix. An internal lock guard now explicitly unlocks before closing,
including metadata-initialization errors, so fork/exec inheritance cannot extend guard
ownership. The persistent lock file is still never unlinked.

TDD evidence: the initial six core session tests failed with the absent-module RED; the desktop
migration test then failed on the old/new store type mismatch before re-export/delegation.
The expanded eight-test suite covers open/reader/writer/spawn failures, early duplicate rejection, concurrent
registration with loser cleanup, poisoned publication/child mutexes, host/platform work
outside the registry mutex, write/flush/wait errors, watcher removal, repeated close,
final-owner cleanup, and a real Linux PTY input/output/close round trip. The profile-lock
regression separately demonstrated RED/GREEN. Linux validation passed all 43 workspace
all-feature tests plus the compile-fail doctest (one ignored fixture is exercised by
its subprocess test), all 103 desktop tests (17 existing warnings), strict workspace
all-feature/all-target Clippy, formatting, and a locked standalone release build.

Ripwire quality delta has zero unacknowledged gating regressions after one narrowly
recorded acknowledgment: the unchanged relocated default constructor resembles unrelated
desktop-store initialization. New tests and trait-dispatched helpers may appear as dead
code. The static test gate still exits 4 with 135 unmapped impacted symbols, largely
pre-existing desktop callers; do not treat it as a clean coverage gate. Cargo execution is the validation evidence.

Cleanup here covers the portable-pty child only and may block in native kill/wait.
It does not prove descendant-tree cleanup, elevated Windows helper supervision, bounded
cancellation/shutdown, signal handling, or foreground CLI ownership. No Windows/macOS
native PTY/keyring/elevation qualification was performed. The core adds the desktop's
existing portable-pty dependency, with no Tauri/HTTP/frontend dependency. CLI commands
remain inventory/profile preview only; no status/runtime or stdio MCP command is exposed.
Next extract host runtime context/IDs and identity-safe monitoring/shutdown, then one
operation family with foreground supervision and platform cleanup gates before exposing
standalone runtime commands. Settings commands/import, full config extraction, standard
Streamable HTTP, stdio MCP, and packaging remain pending.

### Launch-bound PTY exit polling (2026-11-11)

`spawn_pty` now returns `SpawnedPty`: the reader and a `SessionMonitor` bound atomically
at launch to that specific session through a weak reference. `poll` returns Running,
Detached, or an owned exit payload. It checks registry identity before polling the
original child and rechecks identity atomically before removal. A monitor cannot poll
or remove a replacement registered under the same forced ID, including replacement
while `try_wait` is in progress. Weak observers do not keep children alive after the
store's final ownership is dropped. Platform polling/cleanup runs outside the registry
mutex, and event delivery remains outside all core locks.

Watchers now belong to each `PtySession` rather than an ID-only global map. Removal and
exit retirement clear only that object's subscribers after releasing the registry lock;
a stale monitor cannot clear replacement subscribers. Desktop watcher adapters delegate
to core. Adding a watcher to a missing session now returns `session not found: {id}`
instead of retaining an unowned subscription; the only desktop caller already treats
registration failure as an aborted automation. Broadcasting to a missing session stays
a no-op. Existing direct master/child/writer access remains available to desktop adapters.

The desktop keeps its 100ms polling thread, reader thread, nonce/ID generation, settings
gate, Tauri/CC output/exit delivery, transfer completion, and function-exit callbacks.
Normal status strings, exit-code conversion, `wait_error: ...`, and command close
payloads are unchanged. The unused ID-only desktop removal wrapper was deleted.
The core launch return type and watcher-storage layout changed; all in-repository
consumers were updated. Ripwire's edit-check reports unchanged arity/publicness but
does not verify return types; source review and both Cargo builds verify that migration.

TDD evidence: the initial three tests failed on the missing monitor/poll/watcher API.
The deterministic two-barrier regression pauses `try_wait`, removes the old entry,
launches a replacement, and subscribes a replacement watcher before resuming polling.
It fails with ID-only retirement and passes with the atomic identity recheck. The
12-test PTY suite also covers natural exit delivered once, wait-error payloads,
replacement before polling, weak observer lifetime, watcher release, existing launch/
I/O/close failures, and the real Linux PTY round trip. Linux validation passed all
47 workspace all-feature tests plus the compile-fail doctest (one ignored subprocess
fixture is exercised by its ownership test), all 103 desktop tests, strict workspace
all-feature/all-target Clippy, formatting, and the locked standalone release build.

Ripwire quality delta has zero unacknowledged gating regressions. The constructor-only
acknowledgments were refreshed for the smaller independent SessionStore initializer,
which now resembles two unrelated store constructors; no runtime/test findings were
suppressed. New test entries still appear as dead code. The static test gate still
exits 4 with 135 unmapped impacted symbols, primarily desktop callers, rather than a
clean coverage gate. Executed Cargo tests are the verification evidence.

This guarantees identity-safe child polling/registry retirement, not generation-fenced
host event delivery. Output, telemetry, function/transfer callbacks, and queued UI events
still use existing ID-only contracts; an event already returned by core can race a later
reuse of that ID. A future runtime context must serialize lifecycle/publication or carry
internal generation identity through adapters before claiming full replacement safety.
No CLI runtime command or status view was added. Native child polling/kill/wait may still
block; descendant-tree cleanup, elevated Windows helpers, bounded shutdown/cancellation,
signals, and native Windows/macOS qualification remain open.

The next increment below establishes a CLI-local runtime context with private session
ownership and no session-ID reuse. Foreground supervision and publication adapters remain
next; keep native keyring calls behind host supervision.

### Profile-owned runtime context (2026-11-11)

`scp-core::host::ProfileRuntime` acquires the CLI profile owner and keeps its session
store private. Its field drop order releases all strong child ownership before releasing
the OS profile lock. Hosts receive only a reader, an immutable ID accessor, and a private
weak monitor through `RuntimePty`; they cannot retain strong session handles, supply a
forced ID, or mutate the registry through this API. Runtime drop still attempts the
existing direct-child cleanup even if the host retains the reader/monitor.

Launches require mutable runtime access. Each runtime gets a random UUID nonce and each
launch reserves a checked, monotonically increasing counter before platform work, including
failed launches. IDs cannot repeat within that runtime; random nonces separate subsequent
owners and independent profiles. Counter exhaustion fails before native work. This avoids
ID reuse in the new context without changing desktop IDs, events, or command signatures.
It does not order output against exit, fence queued desktop events, or implement a host
event sink. `write`, `close`, and launch-bound `poll` delegate to the tested session core;
close and wait-error payloads retain their existing contracts. A monitor from a previous
runtime detaches when passed to the new runtime.

TDD evidence: three runtime integration tests first failed on the absent host module.
They cover exclusive ownership, lock retention throughout platform calls and final-child
cleanup, reader/monitor lifetime, stable profile identity after cleanup, fresh launch IDs
within/across owners, stale monitors, repeated close, missing-session input, launch failure
and retry, and wait-error retirement. A deliberate early-unlock mutation (placing the
owner before the session store) fails deterministically during cleanup polling; restoring
the intended drop order passes. Linux validation passed all 50 workspace all-feature tests
plus the compile-fail doctest (one ignored subprocess fixture is exercised by its ownership
test), strict workspace all-feature/all-target Clippy, formatting, and a locked standalone
release build, and all 103 desktop Rust tests (17 existing warnings). Ripwire quality
delta has zero gating regressions; it lists six new API/test dead-code entries and two
minor test-fixture churn findings. The static test gate still exits 4 with 97 unmapped
impacted symbols, including existing desktop paths: this is not a clean coverage gate.
Executed Cargo suites supply the validation evidence; no acknowledgments were added.

This is an ownership boundary, not a foreground runner. It starts no reader/poll threads,
performs no secret access, and adds no CLI runtime/status command. Native poll/kill/wait
can still block and cleanup remains direct-child best effort. Descendant trees, elevated
Windows helper supervision, signal/cancellation handling, native Windows/macOS tests,
credential supervision, and event ordering remain open. Next implement a focused host
foreground lifecycle with real-child tests, then an operation family once its supervision
contract is established. Full config extraction, settings commands/import, Streamable
HTTP, stdio MCP, and packaging remain pending.

### Synchronous foreground output and completion (2026-11-11)

`ProfileRuntime::run_foreground` now launches one child through the existing private
ownership boundary, streams raw PTY bytes to a host-supplied `Write`, flushes the sink,
and only then polls for and returns the launch-bound exit payload. Output is streamed
with bounded copy buffering; partial writes and interrupted I/O use standard-library
copy behavior. Nonzero child exits retain their normal payload rather than becoming
host errors. No new CLI command, dependency, background thread, desktop adapter, status
mutation, or event delivery was added.

The mutable runtime borrow keeps profile ownership throughout the call. A private
cleanup guard attempts close on Rust unwinding, including host sink panics; normal
read/write/flush/poll failures explicitly close and retain the primary error, appending
any cleanup error. The guard retains no strong session handle. Successful completion
retires the session through the existing monitor and leaves the runtime/profile usable.
Callbacks execute outside the core registry/status locks. Abort/forced process exit
cannot run Rust drop guards and is not covered by the unwind guarantee.

This is a deliberately synchronous completion contract, not cancellable supervision.
It drains to EOF before polling; a descendant retaining the PTY can therefore delay
completion after the direct child exits. EOF alone does not imply child exit: subsequent
polling waits for that child. Reader, sink, poll/kill/wait can block without a deadline;
there is no concurrent input forwarding, signal handler, output worker, status integration,
or bounded shutdown. Do not expose this as an arbitrary-command CLI or use it for native
keyring calls. Desktop reader/telemetry/publication races remain unchanged.

TDD evidence: the two real-child tests initially failed to compile because the method
was absent. A Rust subprocess fixture produces over 128 KiB plus a final marker and
exits 7, exercising native PTY drainage, partial sink writes, flush, exit conversion,
session retirement, profile retention, and ownership release after runtime drop. Real
children also exercise sink write/flush errors, panic unwinding, and subsequent launches.
The fake-backend regression verifies kill/wait before return/unwind, ownership throughout
cleanup, read errors, and combined output/cleanup errors. Removing the unwind guard's
close deterministically fails the kill assertion; restoring it passes. The injected
reader-failure case was separately confirmed RED before adding the faulty reader fixture.

Linux validation passes 53 workspace all-feature tests plus the credential compile-fail
doctest (two ignored fixtures are exercised by subprocess tests), all 103 desktop tests
with 17 existing warnings, strict workspace all-feature/all-target Clippy, formatting,
and the locked standalone release build. Ripwire quality delta has zero gating
regressions; new findings include test/Drop entries and the new method's parameter count.
No acknowledgments were added. The host-file static test gate exits 4 with eight unmapped
impacted symbols, including existing desktop paths; this is not a clean coverage gate.
Executed Cargo suites supply the validation evidence.

Next implement a focused cancellable host lifecycle with real-child tests, deciding
worker ownership and how blocked readers/sinks and EOF are handled without releasing
the profile prematurely. Preserve this drain-before-exit contract for completed output;
explicitly define truncation on cancellation. Then add an operation family after its
supervision contract is established. Native Windows/macOS, descendant trees/elevated
helpers, keyring supervision, full config/settings extraction, Streamable HTTP, stdio
MCP, and packaging remain open.

### Scoped foreground cancellation and worker ownership (2026-11-11)

`ProfileRuntime::run_foreground_cancellable` adds cooperative host-requested cancellation
without changing the synchronous API or any CLI/desktop command. It accepts a PTY system,
`(cols, rows)`, command, `Write + Send` sink, and borrowed `AtomicBool`. A pre-cancelled call
returns `ForegroundOutcome::Cancelled` without launching. Otherwise it uses the existing
private launch boundary, moves only the reader and borrowed sink/flag onto one scoped
output worker, and checks cancellation/worker completion every 20ms on the calling thread.
The flag must remain set until return; callers must not reset it during a call.

On observed cancellation, the supervisor attempts direct-child close/kill/wait **before**
joining the output worker. No worker is detached. The runtime's mutable borrow retains the
profile lock through launch, child cleanup, worker completion, errors and Rust unwinding.
Worker panics are joined and resumed on the caller, with cleanup during unwind. A shared
cleanup-result helper preserves primary errors and appends cleanup errors for both APIs.
Callbacks and native calls remain outside the registry/status locks. No strong session
handle, new dependency, CLI command, status mutation or event adapter was introduced.

`Completed(exit)` retains full drain/flush-before-exit behavior, including nonzero exit
codes. Cancellation explicitly permits unread/buffered output loss and skips flush when
the worker observes the flag; I/O already underway may still finish. Cancellation returns
only `Cancelled`, not a complete transcript or a fabricated normal exit. I/O and cleanup
errors take precedence over cancellation. After output EOF the supervisor still observes
cancellation while polling the direct child's exit. Completion can win a concurrent flag
change after the supervisor's last check.

This is not bounded shutdown: cancellation cannot interrupt launch, a blocked read/write/
flush, or native poll/kill/wait. Closing the direct child normally wakes its PTY reader,
but a descendant retaining the PTY can still delay the join indefinitely. The profile
stays owned throughout that delay. A stuck sink must be released by its host; production
hosts still need an interruptible/bounded output destination and platform supervision
before claiming deadlines. Abort/forced exit bypasses drop guards. No input forwarding,
signal handler, descendant-tree/elevated-helper cleanup, or native keyring worker is added.

TDD evidence: real-child tests first failed to compile for the missing API/outcome.
They validate >128KiB drainage/final marker/flush/nonzero exit, cancelling an idle child,
subsequent launch/flush errors, and a deliberately blocked sink retaining profile ownership
until released. The 20s real-child watchdog remains. Fake tests cover pre-cancel without
platform work, launch failure, cancellation after EOF, kill/wait before worker join, read/
write errors, worker panic and combined output/cleanup failure. The combined-error test
exposed and fixed premature guard cleanup hiding the cleanup error. Removing the close
before worker join makes the ordering test fail; restoring it passes. Existing synchronous
failure cases now run against both paths through a shared test helper.

Linux validation: 58 workspace tests plus the credential compile-fail doctest pass with
all features; two ignored fixtures are exercised by subprocess tests. Strict all-target/
all-feature Clippy, formatting and the locked standalone release build pass. Desktop
also passes all 103 existing Rust tests (17 existing warnings). Ripwire quality delta has zero gating
regressions, with new test dead-code/parameter findings and minor churn; no acknowledgments
were added. Its host-file test gate exits 4 with 75 unmapped impacted symbols, including
unmodified desktop paths guessed by name; this is not a clean coverage gate or evidence
of actual desktop dependencies. Executed Cargo suites provide validation evidence.

Next connect cancellation to a concrete foreground operation's host lifecycle, with
signal/EOF policy and a suitable output destination; choose a narrow operation family
from existing source rather than exposing arbitrary commands. Keep platform deadline,
process-tree/elevation and real Windows/macOS/keyring qualification explicit release
gates. Configuration/settings extraction, Streamable HTTP, stdio MCP and packaging remain.

### First operation family: shared Caddy launch preparation (2026-11-11)

Caddy is selected as the first narrow foreground candidate based on
`services/caddy_service.rs`: it launches `caddy run --config Caddyfile` in a selected
working folder. Unlike OpenVPN or Boundary/SSH flows, this launch has no application
credential lookup, elevation helper, browser auth or terminal-input protocol. External
Caddy configuration can still launch descendants or require privileged network access;
this choice does not establish process-tree supervision or platform qualification.

`scp_core::caddy::build_caddy_command(&str)` now owns the existing trimmed-folder and
Caddyfile existence checks and literal PTY command construction. Desktop calls it before
replacing a running session. The command name, arguments, working directory, errors,
function/session IDs, status and event adapters are preserved. The helper reads metadata
only: no config/profile creation, keyring access, file writes, shell, launch or status
publication. It deliberately preserves the desktop's existence-only validation; Caddy
validates config contents when run, and files can change after preparation. Explicit
folder input avoids pulling all device-local configuration into this increment.

TDD: three tests first failed to compile because the core module/API did not exist,
then passed after extraction. They cover whitespace/absent folders, missing Caddyfile,
a file supplied as a folder, no creation/modification, and exact executable/arguments/
literal working directory with spaces, Unicode and shell metacharacters. No Caddy
installation, network listener or actual external configuration is required by these
tests. Source review confirms the desktop command alias still calls the same service;
Ripwire misses that alias and reports no desktop callers or affected tests. Its
`--affected=build_caddy_command` selects the new core tests, and the service edit-check
reports unchanged public contract. Linux validation passes 61 workspace tests plus the
credential compile-fail doctest (two ignored subprocess fixtures are exercised by tests),
all 103 desktop Rust tests (17 existing warnings), strict all-feature/all-target workspace
Clippy, formatting, diff whitespace checks and the locked standalone release build.
Ripwire quality delta has zero gating regressions and three new test dead-code findings;
Cargo executed all three tests. No acknowledgments were added. Native Windows/macOS
qualification remains open.

The next focused slice should implement the Caddy foreground **host** boundary, rather
than another generic PTY primitive: define and test signal registration lifetime,
monotonic cancellation, stdin EOF/client shutdown, and raw PTY output routing before
exposing `scpctl caddy ...`. No input forwarding is needed for this operation; stdin EOF
still needs an explicit cancellation policy. PTY bytes must not be mixed into JSON stdout
or silently buffered without a limit. A locked stdout writer can block indefinitely;
select/test a host output destination and failure policy rather than claiming the scoped
worker makes stdout interruptible. Retain profile ownership through every worker join.
Use a real direct-child fixture for signal/EOF and output failures, without launching a
user's Caddyfile during tests. If hard deadlines require a different platform/process
supervisor, implement or explicitly gate that capability before claiming bounded
shutdown; do not silently weaken the plan. Runtime status stays absent until genuine
supervised runtime state exists. Native keyring, elevation, Windows/macOS qualification,
settings extraction/import, Streamable HTTP, stdio MCP and packaging remain open.

### Linux Caddy foreground host preview (2026-11-11)

`scpctl caddy run --foreground --folder FOLDER --output NEW_FILE [--json]` now connects
shared Caddy launch preparation to `ProfileRuntime::run_foreground_cancellable`. This
is a focused Linux host adapter in `crates/scp-cli/src/caddy_host.rs`; the desktop's
commands/events and shared core APIs are unchanged. Inventory/profile commands keep
their existing behavior. The explicit folder avoids extracting device-local settings.
Windows/macOS fail before creating a profile/log or launching; qualification is open.

The command validates the folder, registers SIGINT/SIGTERM/SIGHUP flag actions before
PTY/worker launch, takes the profile owner lock, and exclusively creates the output
file with owner-only permissions (subject to umask). No signal handler performs I/O,
cleanup or locking. The shared cancellation flag only changes false→true during the
invocation; repeated signals do not force exit. Registration guards survive runtime
cleanup and result reporting, and unregister their actions on return/partial setup
failure. `signal-hook` does **not** restore previous/default dispositions after
unregistration ([upstream lifetime contract](https://docs.rs/signal-hook/0.4.6/signal_hook/));
this private adapter belongs to a process that exits immediately after this invocation.
It must not be embedded into desktop/MCP or reused as a general runtime adapter.

Raw output goes to a new file, never stdout/stderr or an unbounded memory queue.
Existing files/symlinks/special nodes are rejected by exclusive creation; no parent
folders are created for the log. Completed output is drained/flushed; cancellation
may discard unread bytes. Output write/flush failures remain errors and attempt
direct-child cleanup before profile release. Ordinary file I/O can block and logs
can grow without limit; no rotation, disk quota or durable crash transcript is
promised. Failure after setup may leave a local profile and empty/partial log.
The runtime retains the profile throughout every worker join and direct-child cleanup.

Stdin is deliberately unused, including EOF: this noninteractive foreground command
has no input forwarding or protocol client. The separate future stdio host still
requires EOF/client shutdown cancellation. JSON stdout contains only a completed
result with actual nullable `exitCode`, or `{"outcome":"cancelled"}`; no raw reason,
credential or PTY transcript is included. Setup/I/O failures have no completion
result. Success exits 0, usage exits 2; cancelled/nonzero/unknown child exits and
host failures exit 1. This is not a background start/status/stop API.

**Supervision limit:** this Linux preview is cooperative, with no hard shutdown
bound. Native launch/read/write/flush/kill/wait, or descendants retaining the PTY,
can prevent return. Signals request direct-child close/kill/reap through the shared
core; there is no graceful Caddy stop handshake, descendant/elevated-helper cleanup,
forced-exit escalation, or crash/abort guarantee. This does not satisfy the eventual
bounded platform-supervision release gate. A platform supervisor and native Windows/
macOS qualification remain required before claiming those capabilities.

TDD: command tests first failed because `caddy` was unknown; host tests then failed
to compile for the missing signal/output adapter. All pass after implementation.
Subprocess fixtures use the current test executable as the real PTY direct child;
no installed Caddy, user config, listener, keyring or desktop is used. Tests cover
all three signals/repeated cancellation, stdin EOF policy, pre-cancel without a
child, >128 KiB plus final-marker output, nonzero exits, partial output writes,
write/flush failures, owner-lock retention/release, child reaping, private log mode,
required arguments, validation before file creation, and existing-log preservation.
An end-to-end CLI test uses an exec-only PATH shim to replace the child with the
Rust fixture, checks the literal Unicode/metacharacter working directory, and
parses stdout as a single JSON result while verifying the complete raw log.

Linux validation: all 68 workspace tests plus the credential compile-fail doctest
pass with `cargo test --locked --workspace --all-features`; five ignored fixtures
are exercised as subprocesses. Desktop Rust tests pass (103; 17 existing warnings).
Strict workspace all-feature/all-target Clippy, workspace format, diff whitespace
checks and the standalone release build pass. Ripwire reports unchanged CLI `run`
contract; affected-test inference misses subprocess dispatch, covered by the CLI
integration suite. Quality delta has zero gating findings after one narrowly scoped
false-clone acknowledgment: the Caddy log's exclusive private-file creation is not
the desktop telemetry append helper's contract. Thirteen remaining new-code
reachability findings concern test/fixture/Drop methods (Linux tests/guards execute;
the unsupported-platform test awaits native execution), plus two minor churn rows.
Native Windows/macOS checks were not performed.

Next: strengthen this operation's host supervision rather than adding unrelated
runtime commands: qualify real Linux Caddy signal/exit behavior and descendants with
controlled fixtures; design a platform supervisor/output policy if hard shutdown
bounds are required. Windows/macOS stay explicitly gated until their adapters and
native acceptance checks exist. Native keyring calls remain unexposed pending host
supervision. Focused settings extraction/import, standard desktop Streamable HTTP,
other operation families, stdio MCP and independent packaging remain actionable.

### Linux Caddy CLI lifecycle acceptance (2026-11-11)

`crates/scp-cli/tests/caddy_lifecycle.rs` adds acceptance coverage through the actual
`scpctl` executable, keeping the production adapter and contracts unchanged. A temporary
PATH shim immediately execs this integration-test executable as the PTY's direct child.
No installed Caddy, user configuration, listener, desktop or keyring is used.

The tests check INT/TERM/HUP delivered to the real CLI with stdin already closed,
cancelled JSON plus exit 1, direct-child reaping and profile release. Zero completion
checks exit 0, one JSON value, empty stderr and a final raw-log marker. The earlier
CLI test retains nonzero/large-output coverage. Competing ownership fails before log
creation or child launch and preserves profile metadata. A missing executable fails
without completion JSON, leaves an empty log, preserves metadata and releases ownership.
All release checks retain the persistent owner-lock file.

RED/GREEN: the first new acceptance run failed at child readiness and the final-log
marker because the new child fixture was absent; adding the fixture made all three
tests pass. The launch-failure/conflict case passed immediately against existing behavior.
No production fix was required. CLI subprocesses have a finite polling watchdog and
are killed/reaped on assertion failure; the direct-child fixture itself exits after six
seconds if orphaned by a failed test. Those safeguards apply only to tests and provide
no production shutdown deadline or descendant-tree guarantee.

Validation: all 71 workspace tests and the credential compile-fail doctest pass with
`cargo test --locked --workspace --all-features`; six ignored fixtures are exercised
by subprocess tests. Strict all-feature/all-target workspace Clippy, formatting and
diff whitespace checks pass. Ripwire selects the new acceptance file for its helper
and reports its intended new private contract. Quality delta has zero gating findings
after a scoped acknowledgment of the short watchdog polling idiom shared with a
private helper in a separate test binary. Five remaining new-code reachability rows
are the three executed tests, subprocess fixture and RAII cleanup method. Desktop
tests were not rerun for this test/documentation-only slice; the previous 103-test
desktop validation still applies, and no production files changed.

Next: use controlled descendant fixtures to establish the existing cleanup limits,
then choose a concrete platform-supervisor/output policy before claiming bounded cleanup.
Actual installed Caddy behavior and Windows/macOS qualification remain open. Settings
extraction/import, desktop Streamable HTTP, other operation families, stdio MCP and
independent packaging remain actionable under the same release gates above.

### Controlled Linux descendant lifecycle evidence (2026-11-11)

The real-CLI acceptance fixture now spawns a descendant that calls `setsid`, retains
the inherited PTY descriptors and waits for a file-based release handshake. Readiness
is published only after detachment. The test checks the descriptor through `/proc`,
uses a test-process-only Linux subreaper to adopt the orphan, and explicitly releases
and waits for the exact descendant PID. An RAII guard kills/reaps only confirmed child
PIDs on failure; fixtures have finite watchdogs. The Linux-only dev dependency on
`libc` uses the already locked version for these test syscalls; production dependencies
and behavior are unchanged. No installed Caddy, user configuration or listener is used.

The evidence distinguishes completion from cancellation:

- **Completion:** after the direct child exits (confirmed as a zombie in `/proc`),
  the CLI remains active with the profile locked and the descendant's PTY open.
  Releasing the descendant allows EOF, direct-child reaping, completed JSON/exit 0,
  and profile release. The test then reaps the adopted descendant.
- **Cancellation:** SIGTERM to the CLI reaps the direct child and returns cancelled
  JSON/exit 1 with the profile released **while the detached descendant is still
  alive with an inherited, hung-up PTY descriptor**. Only the test's subsequent release
  handshake ends that descendant. Profile release is not process-tree cleanup evidence.

Source inspection: `portable-pty` 0.8.1 creates a controlling-terminal session for the
direct child. Its Unix child killer sends SIGHUP to that PID, briefly polls, then falls
back to direct-child kill; its reader maps PTY `EIO` to EOF. Core completion drains
before exit polling, whereas cancellation removes/closes the direct-child session
before joining the output worker. These paths explain why simply retaining a slave
descriptor does not necessarily block cancellation in this setup. Other native calls,
blocked sinks and PTY arrangements remain outside any shutdown bound.

RED/GREEN evidence: the initial test failed at descendant readiness before the fixture
existed. After adding it, an exploratory assertion that cancellation must stall failed:
the CLI had already returned. A completion probe instead waited for the descendant's
six-second watchdog, disproving the same return expectation for normal completion.
The final assertions encode both observed paths, with explicit release rather than
watchdog-driven success. This characterizes existing behavior; no production fix is
claimed. All 72 workspace tests and the credential compile-fail doctest pass with
all features; seven ignored fixtures are exercised through subprocesses. Strict
all-feature/all-target Clippy, formatting and diff checks pass. Desktop production
code is unchanged; the prior 103-test desktop validation was not repeated.
Ripwire selects the acceptance test file, reports the existing fixture contract as
unchanged and the new test as private/new. Quality delta has zero gating regressions;
the three remaining new reachability findings are the executed test, subprocess
fixture and RAII destructor. Extracting zombie observation and release/reap assertions
removed the new test's initial verbosity/nesting findings without changing assertions.

#### Proposed next supervision boundary (design only)

Keep the current preview explicitly cooperative. Do not attempt to upgrade its
guarantees by adding a timer around scoped Rust threads or killing one process group:
this fixture deliberately leaves the original session, and a blocked sink still
borrows the profile-owned runtime. The next implementation needs a separate private
Linux operation supervisor with these reviewable requirements:

1. Acquire the profile before launch and retain ownership until both execution and
   output workers are confirmed stopped. No public daemon/status/stop contract is
   introduced. If cleanup cannot be confirmed, report failure without advertising
   successful cleanup or allowing a replacement runtime to acquire that profile.
2. Contain the operation helper and all its descendants before allowing execution.
   Spike a dedicated delegated cgroup v2 as the Linux containment candidate, with an
   explicit capability check and fail-before-launch behavior when unavailable. Do
   not silently fall back to a PID/process-group enumeration or acquire privileges.
   This capability is proposed, not implemented or qualified on this devbox.
3. Move PTY drainage and potentially blocking log I/O into that contained helper,
   keeping the supervisor's control path independent of file writes and using a
   bounded control channel without raw output. Preserve exclusive/private log
   creation and JSON/stdout separation. Set an explicit log-size limit and failure
   policy before exposing bounded output; do not silently truncate a successful log.
4. Use explicit start/readiness/result/cleanup acknowledgments and monotonic
   cancellation. Specify grace/drain/kill deadlines and failure precedence before
   coding. A deadline may trigger containment teardown, but cannot by itself prove
   that a blocked native operation stopped or that releasing ownership is safe.
   A retained cleanup supervisor must remain observable to its launching host;
   forced host death requires an external ownership/watchdog policy and qualification.
5. Qualify session-escaping descendants, blocked output, launch failure, supervisor
   loss, repeated signals and teardown failure before changing release claims.
   Windows/macOS stay gated until their own containment/ownership implementations
   pass equivalent native tests; native keyring/elevation remain unexposed meanwhile.

The next slice checked the candidate on this devbox: cgroup v2 is mounted, but
uid 1002 has no writable scope control files and creating a dedicated child cgroup
fails with EACCES (13). Nothing was migrated or signalled; no privileges were sought.
The exact evidence and proposed helper/guardian, bounded control/log policy,
acknowledgments, escalation deadlines, failure precedence and crash/recovery gates
are recorded in [Linux operation supervision](LINUX_OPERATION_SUPERVISION.md).
That design is unimplemented. In particular, guardian death releases an ordinary
profile lock; safe crash retention needs an external owner or a recovery interlock
in every acquisition path. Do not retry unchanged native gates or claim bounded
release. Windows/macOS foreground hosts remain explicitly unsupported.

### Shared configuration schema and read-only projection (2026-11-11)

With native containment unavailable here, this increment advances independent
settings extraction. `scp-core::config` now owns the existing `AppConfig`, runtime
and disk-projection types, auth enums, defaults and `AppConfigForDisk::from`.
The desktop model module re-exports them at its original import path. Their bodies
are unchanged; Windows helper launch/status structs remain desktop-local. Desktop
loading/migration, keyring access, backup, writers and command/event contracts retain
their behavior. Core/CLI gain no Tauri/HTTP/build dependencies.

`read_shared_config(explicit_directory)` reads only `config.json`, returning
`Option<AppConfigForDisk>`. Missing files return `None`, not synthesized defaults.
It neither creates/acquires a profile nor reads local settings/keyring, migrates,
writes, starts services or acquires an owner lock. Legacy credential/device-path
fields and the legacy OpenVPN command are discarded using the existing disk
projection. Runtime models themselves remain secret-bearing. Free-text arguments,
names and URLs are not content-redacted, so this API is not an unrestricted MCP
resource or a guarantee that output is safe to publish. It ignores unknown fields
and provides one file's projection, not a lossless backup or consistent multi-file
snapshot. File I/O is synchronous/unbounded; symlinks and concurrent-writer behavior
are unchanged from ordinary file reads. Do not use this API as a restore validator.

A private JSON reader shares missing/read/parse handling with server inventory;
existing inventory diagnostic wording and behavior are preserved. Parse errors use
only context and line/column, never Serde value-bearing details.

TDD: the new core contract target first failed to compile because `scp_core::config`
did not exist. Five tests then passed for exact desktop disk shape/defaults/legacy
alias, absent profile/file, legacy secret/path exclusion without rewriting files or
touching an owner sentinel, malformed schema/JSON and I/O failures without value
echoes, and preservation of arbitrary free-text settings. Refactoring consolidates
actual duplicate read/error handling; the existing inventory/CLI tests cover that
shared path. No installed Caddy, keyring, listener or user settings are used.

Validation after refactoring: `cargo test --locked --workspace --all-features`
passes 77 tests plus the credential compile-fail doctest (seven subprocess fixtures
are ignored for direct harness execution). The desktop `cargo test --locked` suite
passes 103 tests with its 17 existing warnings. Strict workspace all-feature/all-target
Clippy, workspace formatting, the changed desktop file's formatting, and diff checks
pass. A desktop-wide formatting check still reports existing issues in unrelated
files; they are untouched. Ripwire selects the core config/inventory tests, confirms
the inventory signature unchanged and the moved conversion/new reader at their new
locations, and reports quality-delta gating 0. Its five non-gating reachability rows
are the five executed new tests. No quality acknowledgments were added. Native
Windows/macOS execution remains unqualified; their existing schema/cfg code is
preserved exactly.

### Read-only CLI shared configuration command (2026-11-11)

`scpctl config show [--json] [--config-dir DIR]` now calls `read_shared_config`
directly. The global profile-selection/default isolated root behavior is unchanged.
JSON success emits the disk-projection object, or `null` for a missing file/directory;
human output labels pretty-printed JSON or reports absence. Missing input succeeds
without creating a profile or synthesizing defaults. Existing optional fields use
the desktop schema defaults. Errors produce stderr only and exit 1; usage errors
exit 2. Parse errors retain sanitized context/line/column diagnostics.

The command does not acquire an owner lock, initialize metadata, read local settings
or credentials, migrate/write, connect to desktop/CC, or activate MCP/services.
A concurrent profile owner does not block it. Known credential/device-path fields
are excluded and the legacy OpenVPN command is cleared. Free-text settings remain
potentially sensitive; help, human output and README say so. Human output uses JSON
escaping for control characters. Single-file synchronous/unbounded I/O, ordinary
symlinks/concurrent-writer semantics and ignored unknown fields are unchanged. This
is not settings export/import, a consistent snapshot, or a generally safe MCP resource.

Strict RED: five actual CLI subprocess tests all failed with unknown `config`
subcommand/exit 2 before implementation. GREEN: all five pass for exact projection
and defaults, legacy secret/path exclusion, unchanged files, malformed local/profile
files ignored, JSON/human missing-file behavior without initialization, value-bearing
schema/JSON errors without echoes, I/O errors, usage/help, global flags and isolated
default root, control-character escaping, free-text preservation and a live profile
lock held by the test process while the CLI subprocess reads. No native keyring,
installed Caddy, user configuration or listener is used.

Validation: `cargo test --locked --workspace --all-features` passes 82 tests plus
the credential compile-fail doctest; seven ignored fixture entries are exercised
through subprocesses. Strict all-feature/all-target workspace Clippy, workspace fmt,
diff whitespace checks and a locked standalone release build pass. Only CLI code
changed; core and desktop production sources/dependencies remain untouched. Ripwire
confirms `run` has its unchanged signature and one CLI-entry-point caller; its
subprocess coverage is not modeled, so executed CLI tests supply that evidence.
Quality delta has gating 0, two minor ambient-churn rows for the dispatcher/command
enum, five new-test reachability rows and one new-test verbosity row. The integration
test retains its full legacy fixture, exact output and file-preservation assertions;
no quality acknowledgments were added. Native Windows/macOS execution remains
unqualified; this command has no platform-specific production branches.

### Shared device-local settings schema (2026-11-11)

`scp-core::local_settings` now owns the unchanged `SyncSettings`, `SyncSettingsPatch`
and five MCP default/token helpers formerly in desktop `services/config_service.rs`.
Explicit re-exports preserve desktop service imports. `ConfigPaths`, file resolution,
loaders, normalization, initialization, writers, migration, keyring and activation
remain desktop-local. No dependency, command, event, CLI/MCP output or network behavior
changed. The schema contains MCP/CC tokens, device identity and paths; it is not a
public output DTO, redaction mechanism, restore validator or policy validator.

Both `Default` and missing-field deserialization generate a fresh lowercase, simple
UUID v4 MCP token. `Default` uses CC auth mode `token`; an omitted serialized field
continues to deserialize to the empty string. Explicit empty strings/arrays, false
and port zero remain unchanged by the schema; the desktop loader retains its existing
normalization. Unknown fields are ignored, unknown policy/category/auth strings are
preserved, and patch fields remain `Option`: omission and JSON null mean `None`,
while explicit empty/false/zero values remain `Some`. Validation and service policy
must be supplied separately; this extraction deliberately does not normalize them.

TDD: five common contract tests first passed against the original desktop schema.
The new core target failed with the missing-module RED, then passed after moving the
exact definitions. A desktop identity test separately failed with distinct-type
mismatches before desktop re-exports; it passes after adoption. The common contract
is compiled by core and desktop to cover exact serialized fields/defaults, generated
token shape/freshness, legacy omission behavior, Clone, explicit empty/zero values,
all 18 patch fields, null semantics, unknown values and schema/type/port errors.
No settings files, native keyring, listener or external executable are used.

Validation: locked workspace all-feature tests pass 87 tests plus the credential
compile-fail doctest; seven ignored subprocess fixtures are exercised by their tests.
The locked desktop suite passes 109 tests with its 17 existing warnings. Strict
workspace all-feature/all-target Clippy, workspace formatting, formatting of the
changed desktop file, diff whitespace checks and a locked standalone release build
pass. The desktop identity check is rerun after final test cleanup. A source comparison
also confirms the moved production definitions are unchanged. Ripwire impact shows
config/backup/MCP/CC/UI consumers; test selection finds the common contract but does
not fully model include-based desktop coverage. Its full test gate exits 4 with
177 unmodeled/untested impacted symbols across the existing desktop caller surface;
this is not a clean static coverage gate. Executed core/desktop suites provide
the regression evidence. Quality delta has zero gating/preexisting regressions; its eight
non-gating new reachability rows are executed tests and their test modules. No quality
acknowledgments were added. Native Windows/macOS execution remains unqualified;
the extracted definitions contain no platform-specific branches.

### Shared MCP discovery catalog (2026-11-11)

`scp-core::mcp_catalog` now owns the unchanged `McpToolDescriptor` and
`list_mcp_tools`. Desktop `mcp/dto.rs` and `mcp/tools.rs` explicitly re-export them
at the original paths. All 40 names, titles, categories, read-only flags, ordering,
serialization and owned/Clone semantics are preserved. No dependency, dispatcher,
policy, sensitivity, resource, command/event or transport behavior changed; no CLI
command, token or settings output was added. This discovery catalog is not an
execution allowlist or a completed operation registry: argument/result schemas,
capabilities, sensitivity and handlers still need separate characterization.

TDD: two common contracts first passed against the original desktop catalog. The
new core integration target then failed with an unresolved shared module; after
the exact extraction, it passed. A desktop adoption test failed with distinct-type
mismatches before re-exports and passes after them. The same JSON fixture checks
all descriptors and wire fields in both hosts; tests also cover unique names,
Clone and independently owned calls. A desktop source-based boundary test compares
advertised names to literal dispatcher arms. It does not execute actions or prove
policy, dynamic dispatch, handler behavior or schema correctness.

Linux validation: locked workspace all-feature tests pass 89 tests plus the
credential compile-fail doctest; seven ignored subprocess fixtures are exercised
by their tests. The locked desktop suite passes 113 tests with 17 existing warnings.
Strict all-feature/all-target workspace Clippy, workspace fmt, diff whitespace checks
and a locked standalone release build pass. Production source comparison confirms
the moved definitions are unchanged. Existing unrelated desktop formatting remains
untouched. Native Windows/macOS execution remains unqualified; these definitions
contain no platform-specific branches.

Ripwire identifies HTTP discovery, the desktop discovery command and category lookup
as direct callers; the original catalog reaches 15 symbols. Test selection now finds
the common contract, but after staging the full static test gate exits 4 with 13
untested/unmodeled impacts (including one Markdown heading) and two selected test
files. It does not fully model re-exports/include-based tests. This is not clean
static coverage. Executed suites provide regression evidence. Quality delta has zero
gating/preexisting regressions, six new test/module reachability rows, and one
non-gating verbosity row for the exact 244-line moved catalog. No acknowledgments
were added; preserving the characterized representation avoids mixing a rewrite
with extraction.

Next focused slice: establish shared fail-closed execution policy separately from
the permissive persistence schema. Inspect `is_sensitive_tool`, `tool_category`,
`enforce_tool_policy` and their callers with Ripwire. Unknown operation names and
unknown policy strings must not authorize execution; do not copy today's unknown
category bypass or unknown-policy permissiveness. Characterize sensitivity explicitly
without treating `readOnly` or category as a substitute. Keep discovery output stable.

The actual desktop operation for local patches is `sync_settings_save_partial`
(not `config_set_sync_settings`). Its dispatcher deserializes `SyncSettingsPatch`
and calls `save_sync_settings_action`, which can change allowed categories, sensitive
policy, token, listener and CC settings. It is not in today's sensitive list. Tests
must prevent a remote caller from escalating its own authorization through this path
before shared execution policy is adopted. Desktop-local settings changes must stay
available. Also inspect resource calls that delegate into tool dispatch and the direct
Tauri MCP command; do not assume the HTTP entry point is the only consumer. Scope a
reviewable policy increment from source/tests rather than adding a transport here.

Coordinated CLI settings import, standard desktop Streamable HTTP, stdio MCP, other
operation families and independent packaging remain actionable. Native supervision
and actual Caddy/keyring/elevation qualification stay gated rather than blocking
independent increments.

### Remote settings self-escalation guard (2026-11-11)

Before extracting execution policy, the desktop MCP dispatcher now routes
`sync_settings_save_partial` through a tested, injected-writer boundary. The
patch must be an object containing only `repoRoot`, `openvpnExecutablePath`,
`openvpnOvpnPath`, `boundaryCliPath` and `caddyFolder`. Every other key, including
unknown future controls or null-valued control keys, rejects the entire patch
before the save action. Thus MCP callers cannot use this tool to change their own
allowed categories, sensitive policy, token, listener, CC identity or enrollment.
Mixed allowed/disallowed patches never partially write. Validation errors do not
echo input values. Allowed path omission/null/value semantics, save errors and
existing MCP-token response redaction remain. The allowlist is explicit and does
not infer sensitivity from discovery flags or category.

The guard covers HTTP and direct Tauri MCP calls through `call_mcp_tool`; resources
also delegate into that dispatcher but expose no settings writer. The local desktop
`save_sync_settings` command and permissive shared persistence schema are unchanged.
No tool names, descriptors, command/event payloads, CLI commands, dependencies,
transport or token generation changed. Config-mutation category policy still applies
before dispatch. This is a boundary restriction, not protection against arbitrary
code execution by callers already granted runtime/configuration tools.

TDD: four tests first failed on the absent seam. Extracting the prior parser/writer
then demonstrated behavioral RED: a CC control reached the writer, and an empty
array was accepted by Serde's struct deserializer. After object/field validation,
all four pass. Tests cover every current MCP/CC/enrollment field, unknown fields,
null controls, mixed patches, no writer calls on rejection, malformed/missing data,
all five allowed fields, omission/null, sanitized errors, save errors and token
redaction. Tests inject the writer; no app, listener, settings file, keyring or
external executable was used. They do not claim HTTP/UI end-to-end coverage.

Linux checks pass: 117 desktop tests (17 existing warnings), 89 workspace all-feature
tests plus credential compile-fail doctest (seven ignored subprocess fixtures),
strict workspace all-feature/all-target Clippy, workspace and changed-file fmt,
locked release CLI build and diff whitespace check. Native Windows/macOS remains
unqualified; the boundary has no platform branches. Ripwire reports 11 downstream
symbols for the dispatcher, unchanged signature/no incompatible callers, and a new
private seam. Static affected selection does not recognize the inline tests, despite
Cargo executing them. Quality delta has zero gating/preexisting regressions and six
new non-gating rows (four test entries, their module and module verbosity), no acks.
The staged static test gate exits 4 with 12 untested/unmodeled impacts (including
a Markdown heading), zero selected test files; this is not clean static coverage.

Next focused increment: extract/adopt a shared fail-closed execution validator,
separate from the persistence schema and discovery catalog. Reject unknown operation
names and policy values, preserve explicit sensitivity and known category behavior,
and test the actual dispatcher boundary. Keep this remote settings guard under
regression coverage. No shared policy implementation has been added yet. Standard
Streamable HTTP, coordinated CLI import, other operation families, stdio MCP and
packaging remain independent actionable work; unchanged native supervision gates
must not trigger blocked successors.

### Shared fail-closed MCP execution policy (2026-11-11)

`scp-core::mcp_policy::validate_mcp_tool_policy` now validates execution separately
from discovery and the permissive settings schema. Its explicit operation/category
mapping accepts only the current 40 names. Only exact `require-ui-approval` and
`allow-with-token` policy strings are supported. All configured category entries
must be exact known values (`read-only`, `runtime-control`, `config-mutation`,
`file-transfer`); an unknown entry invalidates the whole list, including read-only
calls whose own category is enabled. Duplicate known entries are harmless. The
operation's category must be enabled under both policies. The existing 11 explicitly
sensitive operations are unchanged; sensitivity is not inferred from discovery
`readOnly` or category. Approval remains unavailable and sensitive calls under
`require-ui-approval` fail closed. Typed errors implement Error/Display with fixed
messages and no reflected operation/policy/category values or HTTP-specific context.

Desktop `enforce_tool_policy` now accepts an injected one-shot settings loader and
delegates to the shared validator. The production dispatcher invokes it with the
existing desktop loader before its action match. Thus HTTP, direct Tauri MCP and
all five resource mappings remain covered. Load failures propagate. The pure
validator denies empty categories and rejects blank policies without normalization;
the desktop loader still normalizes empty category lists and blank policies to
defaults. Unknown/empty persistence values and null patch semantics are unchanged.
Discovery output, local desktop actions, listener authentication, command/event
payloads, remote settings guard and CLI behavior remain unchanged. No dependency,
transport, profile, credential or runtime changes were made. Validation alone does
not implement handlers, authorize a CLI capability or sandbox shared-token callers.

TDD: the new core target first failed on an absent module. An interim copy of the
old policy passed characterization but failed three behavioral tests: unknown
operations, unknown policies and mixed known/unknown categories all returned Ok.
The fail-closed implementation passes all six core tests. The existing discovery
JSON fixture checks all 40 category mappings and the exact sensitive list; tests
cover empty lists, duplicates, case/whitespace mismatches, both policies, read-only
calls, and fixed error diagnostics. Three new desktop tests first failed on the
missing injected-loader signature; after adoption they check rejections, one load,
load errors and static ordering before the action match. These execute the actual
policy gate without user files/services. The ordering test is a source check, not
HTTP/Tauri/resource or UI end-to-end coverage. The previous four injected-writer
remote self-escalation tests remain green.

Linux validation: the locked desktop suite passes 120 tests with 17 existing
warnings. Locked workspace all-feature tests pass 95 tests plus the credential
compile-fail doctest; seven ignored subprocess fixtures are exercised by their
acceptance tests. Strict workspace all-feature/all-target Clippy, workspace fmt,
changed desktop-file rustfmt, locked release CLI build and diff whitespace check
pass. Focused policy/gate tests also pass after the error-message refactor. Native
Windows/macOS remains unqualified; the new policy has no platform-specific code.

Ripwire identifies 12 downstream symbols from the existing gate, 11 from dispatch,
including HTTP, direct Tauri and resource consumers. Static affected selection does
not recognize the included/inline tests. Public dispatcher signature is unchanged;
the private gate changes from AppHandle to injected loader (same arity, so edit-check
reports unchanged), and the compiler/tests verify its updated caller. An initial
quality gate flagged the new Display match as a clone of existing credential error
formatting; separating fixed error messages from Display removes that gate without
an acknowledgment or changes to credential code. Final quality delta has zero
gating regressions, two minor existing-symbol findings (dispatcher churn and two
lines of verbosity), and 12 non-gating new-symbol rows (test/trait reachability and
one test clone). The staged static test gate exits 4 with 12 untested/unmodeled
impacts (including one Markdown heading) and two selected test files with unknown
runners. Both are covered by the executed core test target; inline desktop tests
are not selected. This is not clean static coverage or end-to-end execution proof.

Next focused increment: decouple the existing five MCP resource descriptors and
URI-to-tool mapping from Tauri, with exact shared contracts and desktop adoption.
Inspect `McpResourceDescriptor`, `list_mcp_resources`, `read_mcp_resource` and their
callers first. Resource metadata categories are not execution categories; every
resolved resource must still enter the tool dispatcher policy gate. Preserve wire
output and unknown-URI behavior, and add regression evidence that resource mapping
cannot bypass unknown-policy/category rejection. This prepares the standard MCP
adapter without enabling CLI operations or changing the HTTP transport. After that,
shared operation argument/result schemas and the standard desktop Streamable HTTP
adapter are actionable. Coordinated CLI import, other operation families, stdio MCP
and independent packaging also remain; unchanged native supervision gates must not
trigger blocked successors.

### Shared MCP resource catalog and resolver (2026-11-11)

Extracted the five existing descriptors and exact URI-to-tool mappings into
`scp-core::mcp_resources`. `McpResourceDescriptor` retains its owned String fields,
Serialize/Clone and camelCase contract; listing preserves all names, categories,
order and values. Desktop DTO/list paths re-export the shared items. The pure
resolver returns `Option<&'static str>`, accepting only exact URIs with no trimming,
case folding, query/fragment interpretation or fallback. It has no execution or
settings access. Resource categories (`status`, `inventory`, `config`) are metadata,
not execution-policy categories.

The unchanged public desktop resource reader now uses a private injected-dispatch
adapter and the shared resolver. Known resources invoke `call_mcp_tool` once with
an empty JSON object; envelopes and dispatcher errors propagate unchanged. Unknown
resources do not invoke dispatch and retain `unknown MCP resource: {uri}`, including
the unmodified supplied URI. All five operations still pass the existing policy
gate, requiring enabled `read-only`. Unknown policies and any unknown category
entry reject them. No CLI resource exposure, transport, dependency, credential,
profile, runtime or command/event contract change is included.

TDD: the core contract target first failed on the absent shared module; desktop
tests failed on the absent module/dispatch seam (also revealing a test-import
collision corrected during wiring). Four shared tests now cover exact discovery
JSON, owned clone behavior, all five mappings and unknown/near-match URIs; the same
contract runs through desktop re-exports. Desktop adapter tests execute all five
mappings with empty arguments, one invocation, unchanged success envelopes and
errors, plus unknown-URI no-dispatch/error preservation. A separate test resolves
all five resources then executes the actual dispatcher policy gate with both
policies, unknown policy/category settings, resource labels used as categories and
disabled read-only. Production adapter-to-dispatcher wiring is a source check,
complemented by the existing policy-before-action source check. These are not
HTTP/Tauri/UI end-to-end or handler execution tests. No user files, keyring,
listener or installed external executable is used by the new tests.

Linux validation: locked desktop tests pass 128 tests (17 existing warnings);
locked workspace all-feature tests pass 99 tests plus the credential compile-fail
doctest, with seven ignored subprocess fixtures exercised by acceptance tests.
Strict workspace all-feature/all-target Clippy, workspace/changed-file formatting,
locked release CLI build and diff whitespace checks pass. Native Windows/macOS
remains unqualified; this extraction has no platform-specific code.

Ripwire's pre-edit impact identified 13 downstream symbols for listing, 11 for
reading and 14 for the descriptor, including HTTP and direct Tauri consumers.
Edit-check reports the public reader unchanged, and moved list/type as new symbols;
re-exports and compiler/tests verify compatibility. Static affected selection found
no tests. The staged test gate exits 4: 12 untested/unmodeled impacts (including a
Markdown heading), two selected core files with unknown runners, both executed by
the core target. Inline/included desktop coverage is not selected. Quality delta
has zero preexisting/gating regressions, no acknowledgments and 11 non-gating new
test/module reachability findings. Initial duplicate test checks and existing-module
verbosity findings were resolved by keeping resource gate tests together in their
own module, using one focused source-wiring assertion, and separating wire and
ownership checks. This is not a claim of clean static coverage or transport E2E.

Next focused increment: shared operation argument/result schemas with desktop
adoption and exact wire contracts, beginning with the existing MCP tool response
envelope and a small read-only operation family after inspecting source/callers.
Keep runtime execution, transport/auth and CLI enablement separate. Standard desktop
Streamable HTTP, coordinated CLI import, other operation families, stdio MCP and
independent packaging remain actionable. Native supervision/keyring/elevation gates
are unchanged; do not repeat the cgroup spike or spawn blocked successors while
independent work remains.

### Shared custom MCP response envelope (2026-11-11)

`scp-core::mcp_dto` now owns the existing `McpToolCallEnvelope`, with the same
Serialize-only/camelCase declaration and public bool/String/JSON/Vec<String>/u64
fields. Desktop `mcp::dto` re-exports it; the `mcp::*` path and all consumers remain
compatible. No constructor, defaults, Deserialize, normalization or validation was
added. The dispatcher still constructs success messages, warnings and timestamps
and propagates policy/action errors separately. Opaque data, false/empty/null
values, warning ordering/duplicates and the full timestamp range remain representable.
This is a pure DTO extraction, with no transport, execution, auth, runtime, profile,
credential, CLI capability or dependency changes. Standard MCP result conversion
and dedicated operation schemas remain separate work.

TDD: three shared fixture/edge-case tests first pass against the original desktop
DTO. The core target fails on the missing module, and a desktop identity test also
fails before extraction. After moving the unchanged definition, all three contracts
run against core and desktop; the identity test verifies both desktop public paths
accept the core type directly without conversion. Tests use synthetic values only.
They cover serialization/type ownership, not executed tool handlers or HTTP/Tauri/UI
transport behavior; the existing resource adapter/error and policy tests still run.

Linux validation: locked workspace all-feature tests pass 102 tests plus the
credential compile-fail doctest; seven ignored subprocess fixtures are exercised
by acceptance tests. Locked desktop tests pass 132 tests with 17 existing warnings.
Strict workspace all-feature/all-target Clippy, workspace/changed-file formatting,
locked release CLI build and diff whitespace checks pass. Native Windows/macOS
qualification remains open; this extraction has no platform-specific code.

Ripwire identified 15 downstream symbols across seven files before editing;
`--uses` and affected selection found no modeled sites/tests. Edit-check reports
the moved DTO as new; the compiler and public-path identity test verify re-exports.
Quality delta has zero existing-symbol/gating regressions, no acknowledgments and
six non-gating new test/module reachability findings. The staged test gate exits 4
with one unrelated Markdown heading and three selected contract/fixture/target
files with unknown runners, all exercised by the core target. Inline/included
desktop tests are not modeled. This is not clean static coverage or transport E2E.

Next focused increment: extract one small read-only operation's argument/result
DTOs with exact shared and desktop wire fixtures, after inspecting its consumers
and current parsing. Function-status get/list are candidates, but preserve current
missing/wrong-type argument errors, optional/null fields, collection ordering and
status semantics; do not silently harden parsing or introduce a CLI handler. Keep
DTO extraction separate from standard desktop Streamable HTTP integration (which
requires current official SDK/protocol sources). Coordinated CLI import, other
operation families, stdio MCP and packaging also remain actionable. Existing native
supervision/keyring/elevation gates are unchanged; do not repeat the cgroup spike.

### Shared function-status MCP contracts (2026-11-11)

`scp-core::mcp_dto` now supplies the borrowed, Serialize-only
`McpFunctionStatusGetArgs` and its `from_args(&Value)` parser. This is the existing
custom dispatcher lookup moved verbatim: a string `functionId` is required; missing,
null, wrong-type and non-object inputs all return `functionId is required`.
Empty/whitespace/control/Unicode strings remain accepted without normalization;
extra fields remain ignored. No Deserialize, defaults or ID validation was added.

`McpFunctionStatusGetResult` aliases the existing shared runtime `FunctionStatus`;
`McpFunctionStatusListResult` aliases its vector. Desktop DTO/public paths re-export
all three and the two dispatcher branches adopt them. The list still ignores all
arguments and serializes the cloned store values in HashMap iteration order with
no stable-order guarantee. Get still returns `function status not found` for an
absent key. Policy still runs first, and locking, poisoned-lock errors, lookup,
status transitions and envelope construction remain desktop-owned and unchanged.
Bare object/array results retain all six fields, null options, strings and full u64
timestamps. The schema document's stale `item`/`items` wrappers are corrected to the
actual implementation; no wire wrapper was removed from production.

TDD: result fixtures first pass against the existing runtime type. Core then fails
on missing shared types, and the desktop contract/identity target also fails on
missing types. GREEN initially runs four core contracts and five desktop tests;
refactoring consolidates duplicate fixture loading without dropping assertions.
The final three contracts run in core and desktop, plus a desktop identity test
proves runtime/core/desktop result types and argument re-exports assign without
conversion. Synthetic tests cover the exact bare result fixture, explicit nulls,
empty lists/strings, supplied list order/duplicates, optional values, arbitrary
state text, Unicode/control characters, u64 max and every missing/wrong-type input.
They do not execute HTTP/Tauri handlers or establish UI/transport E2E coverage.

Validation: locked workspace all-feature suite passes 105 tests plus the credential
compile-fail doctest (seven ignored subprocess fixtures are exercised by acceptance
tests); locked desktop suite passes 136 tests with 17 existing warnings. Strict
workspace all-feature/all-target Clippy, locked release CLI build, workspace and
changed/shared-file formatting, and diff whitespace checks pass. Windows/macOS
native qualification remains open; no platform-specific code changes here.

Ripwire pre-edit dispatcher impact reports three direct callers (HTTP, resource,
Tauri) and 11 downstream symbols; affected selection finds no modeled tests.
Edit-check preserves its signature and reports the parser as new. Quality delta
has zero gating findings/acknowledgments, one minor ambient churn finding on the
dispatcher and six non-gating new test/module reachability findings. A duplicate
fixture-loading finding was resolved by sharing the fixture in one contract test.
Static selection does not model the included desktop tests; compiler and suites
provide the concrete checks, not a claim of complete static or E2E coverage.
The staged test gate exits 4: three selected contract/fixture/target files have
unknown runners (all exercised by the core target), and 12 modeled untested paths
include the 11 downstream dispatcher symbols and an unrelated SETTINGS_BACKUP
Markdown heading. Full desktop tests do not establish transport E2E coverage for
those paths; this is not a clean static coverage verdict.

Next focused slice: choose another small read-only argument/result family (for
example `app_get_overview`), inspect its handlers/consumers, then extract/adopt
contracts with shared and desktop fixtures. Do not add CLI handlers incidentally.
Standard desktop Streamable HTTP is independently actionable after contract work;
choose the SDK/protocol from current official sources before implementation.
Coordinated CLI import, other operation contracts, stdio MCP and packaging remain
open. No transport/auth/profile/runtime/credential/dependency changes or new CLI
capabilities ship here. Native supervision/keyring/elevation gates remain unchanged;
do not retry the cgroup spike on this unchanged host.

### Shared app-overview MCP result contract (2026-11-11)

`scp-core::mcp_dto::McpAppOverviewResult` now carries the existing bare
`app_get_overview` result: `appVersion: String`, `activeFunctions: usize` and
`knownFunctionCount: usize`. It derives Serialize only, with camelCase and all
three fields always present; no defaults, Deserialize, validation or count
normalization was added. Desktop DTO/public paths re-export the shared type and
the dispatcher uses it in place of an inline JSON object. Version still comes
from desktop package metadata. Counts still come from the locked FunctionStore:
active means `session_id.is_some()` (even empty IDs or non-running states), and
known means every stored status. All arguments, including non-objects, remain
ignored; no empty-argument parser/schema restriction was introduced. Policy runs
first; locking, poison errors and envelope construction remain unchanged.

The helper's `status` command sends `{}` and prints the custom envelope without
reading individual overview fields. HTTP and direct Tauri paths use the same
dispatcher; no resource maps to overview. `MCP_TOOL_SCHEMAS.md` had a stale example
with `mcpEnabled`, `activeFunctionCount`, `activeSessionCount` and `functions`;
it now documents the actual three fields. This corrects documentation drift,
not a production wire change. No CLI overview command, transport, dependency,
authorization, runtime, profile or credential behavior was added.

TDD: the exact fixture first passes against the old JSON expression. Core and
desktop RED then fail on the missing shared result type. GREEN passes two shared
contracts in core and the same two included desktop contracts plus a public-path
identity test. Tests preserve exact fields, Unicode/control text, empty version,
zero counts and the full host usize range. A deliberately inconsistent count pair
proves the DTO does not impose relationships on host-supplied values. Refactoring
replaces repetitive identity-test value construction with a compile-time identity
function signature that proves all three public paths are the same type. These
are synthetic wire/type contracts; they do not execute the actual handler,
counting expression, HTTP/Tauri integration or UI flows.

Validation: locked workspace all-feature suite passes 107 tests plus the credential
compile-fail doctest (seven ignored subprocess fixtures are exercised by acceptance
tests). Locked desktop suite passes 139 tests with 17 existing warnings. Strict
workspace all-feature/all-target Clippy, locked release CLI build, workspace and
changed/shared-file formatting, and diff whitespace checks pass. Native
Windows/macOS qualification remains open; no platform-specific code changed.

Ripwire dispatcher impact still reports three direct callers (HTTP, resource,
Tauri) and 11 downstream symbols, with zero modeled affected tests. Edit-check
reports its signature unchanged. Quality delta after refactoring has zero gating
findings/acknowledgments, one minor ambient dispatcher churn finding and five
non-gating new test/module reachability findings. Included desktop tests and
synthetic serialization contracts do not establish handler coverage. The staged
test gate exits 4: it selects the three new contract/fixture/target files and the
existing function-status contract, all exercised by workspace tests, but cannot
infer their runners. Its 12 untested paths include the 11 downstream dispatcher
symbols and an unrelated SETTINGS_BACKUP Markdown heading. This is not a clean
static coverage verdict or transport/UI E2E proof.

Next focused slice: inspect the `openvpn_get_status` / `boundary_get_status`
read-only result family (existing bare status-or-null lookup) and preserve its
ignored arguments, missing-status null and runtime result semantics in shared
DTO aliases/fixtures with desktop adoption. Alternatively, standard desktop
Streamable HTTP is independently actionable after checking current official
SDK/protocol sources. Other operation contracts, coordinated CLI import, stdio
MCP and packaging remain open. Do not retry the cgroup spike on this unchanged
host or claim stronger supervision/keyring/elevation guarantees.

### Shared OpenVPN/Boundary MCP status results (2026-11-11)

`scp-core::mcp_dto` now exposes `McpOpenVpnGetStatusResult` and
`McpBoundaryGetStatusResult`, both aliases of `Option<runtime::FunctionStatus>`.
Desktop DTO/public paths re-export them and the two existing dispatcher branches
use type annotations before serializing. These aliases add no wrapper, parser,
validation, normalization or new derives. Missing status remains bare JSON null;
present status keeps all six runtime fields, including null options, arbitrary
states/IDs/token sources, empty strings and the full u64 timestamp. Host lookups
still select `openvpn-connect` / `boundary-auth` and clone under the same lock.
All arguments, including non-objects and supplied `functionId`, remain ignored.
Policy still runs first; poison errors and success envelope construction are
unchanged. No CLI status command, transport, dependency, credential, profile,
runtime or platform-specific behavior was added.

Consumer inspection found helper status commands send `{}` and print envelopes
without inspecting status fields. HTTP/direct Tauri calls share the dispatcher;
the existing OpenVPN/Boundary resource URIs delegate to these same tools with
`{}`. Both `MCP_TOOL_SCHEMAS.md` examples incorrectly showed a `status` wrapper;
they now document the existing bare status-or-null result and ignored arguments.
This fixes documentation drift without removing a production field.

TDD: the exact fixture first passed against local aliases of the original
`Option<FunctionStatus>` representation. Core RED then failed on missing shared
aliases; desktop RED failed on missing shared/re-exported types. GREEN passed
one shared fixture contract in core and the same included desktop contract plus
a compile-time public-path identity test for both aliases. The fixture covers
null, all six fields, null/empty optional strings, arbitrary noncanonical ID and
state, Unicode/control text, zero and u64::MAX timestamps. Its length assertion
prevents zip truncation from hiding fixture cases. No further refactor was needed.
These synthetic wire/type tests do not execute fixed-ID selection, ignored-argument
handling, locking, policy, HTTP/Tauri integration, helper output or UI flows.

Validation: locked workspace all-feature suite passes 108 tests plus the credential
compile-fail doctest; seven ignored subprocess fixtures are exercised by acceptance
tests. Locked desktop suite passes 141 tests with 17 existing warnings. Strict
workspace all-feature/all-target Clippy, locked release CLI build, workspace fmt,
changed desktop/shared contract rustfmt and diff whitespace checks pass. Native
Windows/macOS qualification remains open.

Ripwire reports three direct dispatcher callers (HTTP, resource, Tauri), 11
downstream symbols and zero modeled affected tests. Edit-check reports the
signature unchanged. Quality delta has zero gating findings/acknowledgments,
two minor dispatcher findings (ambient churn and three added body lines) and
four new non-gating test/module reachability findings. These are understood
limits of the focused annotations and included-test modeling, not proof of
handler coverage. The staged test gate exits 4: it selects the two new contract/
target test files and the existing function-status contract (all exercised by the
workspace suite), but cannot infer their runners. Twelve untested paths include
the 11 downstream dispatcher symbols and an unrelated SETTINGS_BACKUP Markdown
heading. This is not a clean static coverage verdict or transport/UI E2E proof.

Next focused increment: inspect `servers_list` / `server_get` handlers and
consumers, then share their argument/result contracts with exact shared and
desktop fixtures if the existing inventory type is appropriate. Preserve list
ordering, permissive string lookup, exact errors, ignored list arguments and
policy/loader behavior; do not add CLI operations incidentally. Standard desktop
Streamable HTTP is independently actionable after checking current official
SDK/protocol sources. Coordinated CLI import, other operation families, stdio
MCP and packaging remain open. Do not retry the unchanged-host cgroup spike or
claim stronger cleanup/keyring/elevation guarantees.

Proposed Cargo workspace boundaries:

```text
scp-core          models, config repository, secret store, runtime stores,
                  operations, PTYs/transfers, backup, policy, domain events
scp-mcp           standard tool/resource schemas and protocol adapter
                  depends on core; transport features selected by host
desktop           Tauri commands/events/dialogs/webviews + HTTP MCP host
scp-cli           shell commands + headless host + stdio MCP host
```

Names are provisional. Avoid a large rewrite: extract one tested operation family at a time. Keep existing frontend command/event names stable.

Replace `AppHandle` dependencies in shared behavior with explicit config paths, runtime context, secret-store access, event sinks, cancellation, and platform capabilities. Desktop converts domain events into existing Tauri events; CLI converts them into terminal output or MCP notifications. Keep telemetry/CC hooks behind host adapters rather than making the core import Tauri or connect automatically to CC.

One operation registry defines names, argument/result schemas, categories, sensitivity, capability requirements, and handlers. Both MCP transports and CLI commands use these operations, but not every local operation must be exposed through MCP. Pass a trusted host-created execution context for policy and runtime ownership; never derive authority from caller-supplied tool arguments. Transport-specific tokens and session metadata belong to adapters, not business logic.

Put Tauri and its build script in the desktop package only. A separate CLI binary inside the current package is insufficient isolation. The first extraction gate is a core/CLI dependency graph and build that require no Tauri, GTK/WebKit, frontend build, or Tauri build script. Platform keyring and SSH libraries may still require native dependencies; document these separately.

## Work 1: complete settings export/import

Implementation: see [desktop backup design, usage, recovery and validation](SETTINGS_BACKUP.md). The v1 importer deliberately blocks Boundary token and telemetry key replacement until the recovery constraints below can be verified. Windows/macOS native credential-store, filesystem behavior and cross-platform release KDF benchmarks remain release qualification work.

### Backup contents and scope

Inventory every persisted setting and reachable app-owned credential before implementation, including any UI preferences found outside the three JSON files. Include a manifest describing included, absent, unavailable, and deliberately unselected sections; never report a partial export as complete.

| Section | Contents and restore behavior |
| --- | --- |
| Shared settings | All `config.json` fields and `servers.json`, preserving stable server IDs |
| Device-local settings | All `sync-local.json` fields, including paths, MCP settings/token, CC settings/identity/token, enrollment state |
| Configured keyring secrets | OpenVPN password/OTP, Boundary password, OIDC password/OTP, per-server SSH passwords |
| Other app-owned credentials | Boundary app token and telemetry queue encryption key; retain in the encrypted archive, but exclude from portable restore and apply the recovery constraints below |
| External dependencies | Manifest of referenced OVPN, certificate/key, SSH identity, and Caddy files; identify missing paths on restore |

The full encrypted export includes all app-managed settings and app-owned credentials above. Default portable import restores user configuration and configured passwords, but does not automatically activate copied CC identity or MCP access. Offer explicit same-device recovery options for identity/token restoration; generate a new MCP token on portable import and require CC re-enrollment by default. Preserve original values inside the backup so full recovery remains possible.

“All settings” does not mean copying the user's entire OS credential store. Read only the app's known service/accounts. Enumerate dynamic SSH accounts from the snapshotted server IDs; the current API has no portable enumeration of orphaned keyring entries. State this limit rather than promising to recover credentials for deleted servers. Boundary CLI-managed credentials, SSH-agent keys, and external files are separate assets; list their scope explicitly. A settings backup initially records external file references rather than silently bundling arbitrary directories or private keys. Do not claim that importing settings alone makes a new machine connection-ready.

Do not restore live PTYs, connection state, telemetry queues, or cached logs. Preserve the destination telemetry key by default. Replacing it can strand an existing destination queue, and the running queue cipher may still use the old key. Same-device key recovery must stop telemetry and verify compatibility with retained queue data; if that cannot be established, block key replacement. Queue backup/migration is a separate feature. Boundary tokens can expire or be controller-specific; validate them before reuse and fall back to login.

Portable import must also preview executable paths, command arguments, sync roots, and privilege-related settings as executable configuration. Preserve target-local paths by default and let the user remap foreign paths. When their sections are selected, restore MCP and CC disabled until explicitly activated after import; retain the source enable flags in the archive rather than silently acting on them. Unselected destination sections remain unchanged.

### Format and export flow

1. Add **Export settings** and **Import settings** buttons to Config.
2. Export asks for a destination and backup passphrase, with confirmation. Credentials are included; the UI shows section counts, never secret values.
3. Collect a consistent snapshot through a configuration transaction gate used by **all** app writers, including desktop commands, MCP, CC, sync, migrations, and credential updates. Cross-process coordination is required before any shared-profile mode ships. Do not hold runtime-store mutexes while an OS-keyring unlock prompt waits for the user; use a dedicated maintenance gate and bounded/cancellable work. Read the OS store directly, bypassing cache. Missing entries are valid; locked, denied, and other read failures are explicit failures for a full backup, with retry or clearly labeled partial export.
4. Encrypt a versioned structured payload using authenticated encryption and a password KDF. Proposed choice: Argon2id plus AES-256-GCM, using reviewed libraries, random salt/nonce, authenticated version/KDF metadata, and bounded KDF parameters. Benchmark parameters across supported platforms. The existing AES-GCM dependency alone is not a passphrase scheme.
5. Write encrypted bytes through a restricted-permission temporary file in the destination directory, flush durable data, and use a platform-appropriate atomic replace. Confirm overwrite, reject unsafe symlink targets, and preserve an existing backup on failure. Cover Unix permissions and Windows ACLs. No plaintext intermediate files, exported credential values sent to the frontend, secret logs, or passphrase in command arguments. A passphrase entered in the desktop UI necessarily crosses Tauri IPC: use a narrowly scoped command, suppress argument logging, and clear UI/backend buffers promptly where practical.

The header records format version, KDF parameters, salt, and nonce. The encrypted manifest records source app/schema/platform versions, timestamp, selected sections, settings, and credential presence/value mapping. Before running the KDF, bound header length, ciphertext size, KDF memory/time/parallelism, and supported algorithm identifiers; authenticate the same header bytes as associated data. Use fresh cryptographic salt/nonce per export. Validate supported payload schemas after authentication, migrate supported older schemas explicitly, and reject unsupported newer schemas without dropping fields. Avoid compression in v1. Backup encryption is portable; it must not rely on the source machine's keyring for decryption.

### Import and failure recovery

1. Decrypt and authenticate fully before mutation; wrong password, corruption, or unsupported schema must leave the machine untouched.
2. Validate schemas, unique server IDs, referential integrity, bounded sizes/KDF cost, and credential-account mappings. Never allow the archive to specify arbitrary keyring service/account names or extraction paths.
3. Show a redacted preview: sections, server counts, changes, foreign-platform paths, identity choices, and unresolved external files. Initial implementation uses replace for selected sections, with explicit confirmation; merge is deferred to avoid ambiguous secret/server-ID conflicts.
4. Compile the preview into explicit `keep` / `set` / `delete` credential actions. For a selected complete section, source absence means deletion to match the source; unavailable or unselected entries mean keep, never delete. Server records and their credentials form a dependent restore group: require matching destination IDs if importing credentials alone. In server-list replacement, include removal of credentials for removed destination servers in the preview and rollback snapshot. Do not route this through the normal save API's empty-means-preserve semantics. Require affected runtime operations, reconnect loops, and browser sessions to stop before replacing their settings; do not silently terminate active work. Recheck the destination revision and archive identity at commit so a stale preview cannot overwrite intervening changes.
5. Snapshot current affected settings and secrets for rollback, then stage validated files. Failure to read any required rollback credential blocks mutation. Apply credentials through OS persistence; publish the new cache/config revision only when the transaction commits, or reload the recovered state after rollback. Commit config files atomically per file while the maintenance gate prevents readers from observing mixed revisions.
6. There is no transaction spanning files and OS keyrings. Durably write a protected encrypted recovery journal **before** the first mutation, with explicit transaction stages and idempotent recovery. Its random encryption key must be durably stored in a dedicated local OS-keyring account outside the restore set and excluded from backups; verify retrieval before mutation. Do not depend on an in-memory key or the backup passphrase remaining available after a crash. On startup, resolve the journal before ordinary config loading/migrations or listeners/connections. If the journal key is locked/unavailable, stop in a recoverable maintenance state. Retain the journal/key until commit or rollback is durably complete, then clean them up. A failed rollback must be reported with actionable recovery information, never as success.
7. Refresh UI/cache and deliberately reconfigure runtimes after a successful commit. Imported settings must not silently enable network listeners, reconnect CC, or launch commands during preview/restore.

Preview must not write config, migrate secrets, enroll devices, or start runtime services. Gate recovery only when a pending local transaction exists.

Keep secret-bearing backup operations out of MCP by default. If later exposed, require a distinct explicit policy and local file destination; never return decrypted backups or stored passwords through ordinary config tools/resources.

## Work 2: standards-based desktop MCP over Streamable HTTP

### Protocol and SDK decision

Use the official Rust `rmcp` SDK with a maintained HTTP framework rather than extending the hand-written HTTP parser. Pin a published SDK version only after verifying its supported protocol revisions, Rust toolchain, dependency compatibility, and cross-platform builds. Do not depend on an unpinned main branch.

The official specification checked for this review is revision **2026-07-28**, which differs materially from 2025-era MCP. Target that revision plus **2025-11-25 compatibility**, subject to the SDK/client spike. Do not mix their lifecycle rules:

- Modern requests use per-request metadata and `server/discover`; legacy clients use `initialize` / `notifications/initialized`. Both HTTP and stdio need tests for each supported lifecycle.
- Modern HTTP uses POST with JSON or request-scoped SSE, required metadata/routing headers validated against the body, and no protocol sessions or GET/DELETE stream lifecycle. Legacy HTTP uses its own initialization, optional sessions, GET streaming (or 405), and DELETE behavior.
- Cancellation differs: modern HTTP closes the request SSE stream, legacy HTTP uses explicit cancellation rather than treating disconnect as cancellation, and stdio uses `notifications/cancelled`. Long-lived modern updates use `subscriptions/listen`. [Transport rules](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http), [legacy rules](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).
- Implement required version-specific semantics through the SDK, and advertise only verified capabilities. SSE alone does not constitute MCP compliance.

If the published SDK or required clients cannot pass the chosen revision tests, release only the verified revision with an explicit compatibility matrix and defer the other; do not hand-build a second protocol engine. The app's custom REST routes are separate from the deprecated 2024 HTTP+SSE transport, which is outside v1 scope.

Scope “complete MCP” as an interoperable tools/resources server with the app's supported operations, standard errors, lifecycle, and progress/events. Optional prompts, sampling, elicitation, and task extensions are only added when needed; do not advertise unsupported features.

### Implementation requirements

- Mount standard JSON-RPC MCP at `/mcp`; implement tools/list, tools/call, resources/list, resources/read and supported resource templates/subscriptions where useful.
- Convert custom envelopes into standard MCP results; distinguish protocol errors from operational tool failures. Preserve tool names and resource URIs where practical.
- Generate public input/output schemas from dedicated DTOs, not secret-bearing runtime models; use shared types only where safe. Validate structured results against output schemas and mark operational failures with standard tool error semantics. Tool annotations are client hints, not enforcement. Enforce arguments and authorization at execution time. Discovery must reflect allowed operations and host capabilities without leaking secrets.
- Deliver status/transfer updates using the selected revision's notification/subscription mechanisms. Provide bounded status snapshots/polling when subscriptions are unavailable; keep progress scoped to an in-flight request, not a completed start call. Bound queues, apply backpressure, and avoid one slow subscriber blocking PTY reads.
- Reuse localhost binding and bearer authorization. Validate Origin and Host/authority, handle IPv4/IPv6 correctly, and avoid permissive CORS. Test requests with and without Origin. Transport sessions are not authorization credentials.
- Document the local bearer-token setup honestly. Any broader OAuth/discovery interoperability is a separately tested capability; a static token is not a general remote OAuth implementation.
- Bound request/body sizes, timeouts, concurrency, connection queues, and event buffers. Move blocking PTY/SFTP/filesystem work off async executor threads. Support cancellation and orderly shutdown without automatically retrying side-effecting calls.
- Preserve category allowlists and audit redaction across HTTP, stdio, resources, errors, and logs, including CC agent tokens as well as MCP tokens. Do not assume transport-level field-name redaction protects the core. Reject unknown policies/categories rather than accidentally treating them as permission to execute. MCP config mutation must not grant itself additional categories, change its approval policy, rotate its own access credentials, or activate CC identity without separate local authorization. Existing `require-ui-approval` currently blocks; initially keep that fail-closed behavior and label it accurately. A real desktop approval queue can follow with timeout, cancellation, and request binding.
- Retain custom REST routes under a documented deprecation period for the existing helper, sharing operation handlers. Remove only after migration and compatibility tests; keep health/status diagnostics separate from the MCP protocol.
- Update Config with endpoint/token handling and tested client setup examples.

Build an operation matrix with existing desktop command, current HTTP tool, CLI command, capability, sensitivity, ownership, and acceptance test. Record deliberate exclusions instead of equating parity with exposing everything. Finish the known operation gaps from that matrix: Boundary target search, OpenVPN executable detection, RDP launch where supported, SSH credential management, and any newly discovered config/runtime gaps. Raw PTY control requires explicit policy and session ownership; decide its scope rather than exposing arbitrary terminal input by accident. Treat the existing single bearer token as one shared local principal: caller-provided client names and protocol session IDs cannot isolate mutually untrusted agents. Multi-principal access would require distinct credentials and ownership checks.

## Work 3: standalone CLI and stdio MCP

### Operation lifetime contract

Separate accepted/running/completed results. Start tools return a runtime identifier plus session/transfer identifier; status queries report eventual outcomes. Request cancellation stops only work still owned by that request, not an already-started VPN or tunnel; those use explicit stop operations. Desktop HTTP disconnect must not tear down app-owned work after the start response. In contrast, shutdown of the CLI stdio host cleans up everything it owns. A lost start response is an unknown outcome: inspect state before retrying and never use JSON-RPC request IDs as durable deduplication keys. Define duplicate-start and stop-already-stopped behavior per operation.

### Proposed commands

```sh
scpctl status --json
scpctl server list
scpctl openvpn start --foreground
scpctl server tunnel start --server-id ID --foreground
scpctl server ssh --server-id ID
scpctl scp upload --server-id ID --local FILE --remote PATH
scpctl settings export --output settings.scpbackup
scpctl settings import --input settings.scpbackup --preview
scpctl mcp serve --stdio
```

`scpctl` is provisional (check naming conflicts before release). Offer version/help, stable exit codes, JSON output, completion, explicit config/profile selection, and safe stdin/file/terminal secret input. Never accept passphrases/passwords as routine command-line values.

### Runtime ownership and process lifetime

- One-shot queries and completed transfers exit after producing results. A fresh `scpctl status` process has no view of another foreground CLI process; report configured capabilities and this process's runtime ID/state explicitly. Use the existing helper to inspect desktop-owned state. Cross-invocation CLI status/control requires the deferred daemon.
- Long-running VPN/tunnel/Caddy commands stay in the foreground in v1, own their child processes, and handle termination cleanly. Starting a tunnel and then exiting cannot honestly return a usable persistent runtime.
- Boundary-backed one-shot SSH/SCP must create, await, and clean up a temporary tunnel within the same invocation, or use a tunnel explicitly supplied as an external dependency. Do not require a second same-profile CLI owner to start the tunnel while the first holds the owner lock. The stdio host can reuse tunnels it already owns. Tests cover readiness timeout and cleanup when either connection or transfer fails.
- The stdio server is a long-running headless runtime owner; sessions survive between MCP calls while that process lives. Status reports this runtime's state, not a fabricated view of the desktop's memory.
- On stdin EOF, signal, or client shutdown, cancel operations and clean up owned sessions, transfers, browser handles, and children within a bounded timeout. Test platform-specific elevated OpenVPN ownership carefully: the existing `terminateOpenVpnWhenHelperExits` setting can conflict with foreground lifetime. Require compatible child ownership for CLI-managed VPN or reject the start. Crash/forced-termination cleanup requires platform process supervision; document limits rather than guaranteeing cleanup after every kill.
- If independent `start`, `status`, and `stop` invocations must manage persistent background work, add an explicit CLI-owned daemon in a later phase. That daemon must not depend on the desktop. Do not accidentally invent persistence through orphan processes.
- Use separate runtime/profile identifiers and an exclusive runtime-owner lock per profile in v1; multiple MCP clients commonly spawn separate stdio processes. Fail with an actionable conflict when a second process tries to own that profile. Protect config writes with cross-process locking and detect tunnel/listener port conflicts. Do not share PTY ownership across desktop and CLI or let a CLI shutdown stop desktop children.
- Default to a deterministic separate CLI config root/profile, with explicit profile selection and settings import for migration. Each CLI profile gets a stable, locally generated credential namespace that is not copied from the archive. A different config directory alone does not isolate the current process-global cache or fixed keyring accounts. Key caches by namespace/account and keep existing desktop identifiers unchanged. Defer sharing desktop files until cross-process coordination and cache invalidation are implemented.
- Separate profiles do not isolate machine-wide VPN routes/DNS, elevation, or external Boundary CLI credential stores. Define a host-wide conflict policy for VPN ownership and choose isolated external CLI state where supported; otherwise surface the sharing and require explicit use. Do not promise concurrent VPN isolation merely because locks/ports differ.

### Host capability differences

| Capability | Headless CLI approach |
| --- | --- |
| Config, servers, SCP/SFTP, tunnels, Caddy | Shared core operations |
| Interactive SSH/password Boundary auth | CLI terminal/PTYS; MCP cannot borrow protocol stdin for password prompts |
| Browser-based Boundary login | System browser only where available; otherwise explicit unsupported/manual flow |
| Automated OIDC using hidden Tauri webview | Desktop-only initially; headless replacement is a separate validated project |
| OpenVPN elevation | Preserve Windows helper/UAC and Unix sudo behavior through platform adapters; clearly fail when interaction is unavailable |
| RDP launch | Optional native-client capability on supported interactive hosts |
| OS keyring | Same supported OS stores, with explicit locked/unavailable errors; Linux headless environments may lack a secret service |
| CC agent/telemetry | Remains desktop-hosted initially; CLI must not duplicate enrolled device identity or emit duplicate telemetry |

Expose capability availability/reasons rather than promising identical behavior on a machine without a browser, desktop, keyring, or elevation mechanism. Document this matrix as the deliberate limit of parity.

### stdio MCP contract

- Use the same SDK adapter, operation schemas, policy, and redaction as desktop MCP, hosted directly on the headless runtime.
- Standard newline-delimited JSON-RPC on stdin/stdout; diagnostics only on stderr. Capture child output and convert it into explicit results/events so it cannot corrupt the protocol stream.
- No listening HTTP port or desktop token discovery in stdio mode. Authorization comes from the launching user's process context plus explicit local operation policy; tokens remain relevant to HTTP.
- Never prompt on protocol stdin or block indefinitely on an invisible approval UI. For sensitive actions require an explicit configured allow policy, otherwise fail closed with an actionable error. This applies even though a client can launch the process.
- Support concurrent requests, cancellation, progress, and bounded output; keep raw terminal/secret output out of ordinary status resources.
- Produce a separately installable binary that runs without Tauri, a webview, frontend assets, or the desktop app. External programs such as OpenVPN, Boundary, SSH/SCP, and Caddy remain prerequisites for their respective operations.

## Delivery order and TDD gates

Every code increment must follow repository RED → GREEN → REFACTOR: write a behavioral test, run and verify the expected failure, implement minimally, then rerun relevant tests and the existing suite. Before editing existing symbols, run `ripwire . --impact="<symbol>" --legend=compact` and `ripwire . --uses="<symbol>" --legend=compact`; report direct callers, affected paths, and risk, and warn before high-risk edits. No symbol impact analysis or runtime refactoring was performed during this documentation-only assessment.

1. **Inventory and spikes.** Capture settings/credential/operation/capability matrices; baseline current tests. Specify the backup schema, secret-action semantics, transaction/recovery-key design, and local-only policy controls before implementation. Verify SDK published-version support using one real client for each target protocol era and both transports. Verify direct keyring access and a minimal headless build on supported OSes. Resolve dependency/toolchain issues before committing to extraction boundaries.
2. **Backup foundation and desktop UI.** Introduce injectable secret/config repositories, coordinated writers, and backup format; implement encrypted export, preview, recoverable import, and Config buttons. Complete this first user-visible feature without waiting for the full CLI extraction.
3. **Core extraction.** Move config, policy/registry, stores/events, PTY lifecycle, and each operation family behind host adapters. Characterization tests preserve desktop command/event contracts throughout. Build the core with no Tauri dependency.
4. **Desktop Streamable HTTP.** Add SDK-based MCP adapter and HTTP hosting with compatibility routes; validate protocol behavior and permissions before finishing operation gaps. This needs the policy/registry seam, not completion of every PTY/CLI extraction; it may ship earlier once that seam is ready.
5. **Standalone CLI.** Add direct core commands, foreground lifetime, profile handling, and OS adapters. Verify it works with the desktop closed and HTTP disabled.
6. **stdio MCP.** Host the same adapter on the CLI runtime; validate process lifecycle, clean stdout, long-running operations, and cross-transport schema/policy parity.
7. **Release and migration.** Package and smoke-test Windows/macOS/Linux CLI artifacts independently of the desktop, update README/AGENTS and older MCP docs, publish client examples and the capability matrix, then deprecate the helper only when replacements cover its documented usage.

### Required acceptance tests

- Backup round-trip includes every configured secret, dynamic server credentials, and local settings; source can be a different platform. Configured keyring secrets are written to the target OS store, never plaintext JSON. Device-local MCP/CC tokens currently live in `sync-local.json`; preserve their documented storage contract and portable-restore rules rather than claiming this work migrates all tokens into the keyring.
- Locked keyring versus missing credential, stale cache, wrong passphrase, tampering, malformed/oversized archive, unknown schema, unsafe account mapping, and restrictive file permissions are covered.
- Inject failure after every import stage, including journal/key creation, keyring write/delete, file commit, rollback failure, and process interruption; verify recovery and cache consistency. Cover a locked journal key on restart, stale preview, concurrent writers, source-absent versus unavailable secrets, removed-server credential cleanup, and preservation of unselected sections.
- Portable restore leaves imported CC/MCP sections inactive, preserves target queue encryption and unselected settings, cannot trigger imported commands, and keeps preview free of config/keyring mutations.
- UI export/import cancellation, passphrase confirmation, preview, selected-section behavior, external-path warnings, and success/failure presentation are tested with Playwright and mocked Tauri IPC/native dialogs where appropriate; supplement with real desktop smoke tests. Use `npx playwright test`, `npm run build`, and `cargo test --manifest-path src-tauri/Cargo.toml` as the current baseline, recording environment-dependent failures. Do not invent a root `npm test` command or assume React/Vitest.
- MCP tests use an official SDK client plus representative real clients: discovery/initialization by revision, schemas/results/errors, malformed input, authorization/Origin, request streaming, cancellation, event backpressure, concurrent calls, and shutdown.
- CLI integration tests close/disable the desktop, exercise direct SSH/tunnel/transfer fixtures, assert exit codes and JSON output, test profile locking/conflicts and credential isolation, and verify owned child cleanup. Also run desktop and CLI concurrently to verify CLI shutdown cannot stop desktop children or change desktop secrets.
- MCP tests reject remote policy escalation and unknown policy values, check token rotation invalidates subscriptions/access as designed, and distinguish request cancellation from lifetime of an accepted background operation. Include duplicate starts and loss of the start response.
- stdio subprocess tests assert every stdout line is valid protocol JSON, PTY/subprocess output never leaks into stdout, EOF cleanup works, and sensitive calls cannot bypass policy.
- Cross-platform CI checks compile and exercise the extracted core/CLI independently of GUI libraries. Real OS-keyring/elevation/browser smoke tests supplement mocks; unavailable platform checks must be documented rather than claimed as passed.

### Explicitly deferred from the first release

No daemon, background cross-invocation CLI control, shared desktop/CLI profile, full remote OAuth deployment, 2024 HTTP+SSE transport, raw PTY API, headless webview OIDC replacement, external-file bundling, queue migration, or restore merge mode. A functioning desktop approval queue remains separate from the initial fail-closed policy. These can be added after the three requested outcomes work within the stated capability limits.

## Risks and decisions to validate

The main risk is hidden Tauri coupling, especially webview OIDC, elevated Windows OpenVPN, PTY telemetry, and lifecycle hooks. Next are non-transactional keyring restore, portable paths/device identity, and conflicts between independent runtime owners. Staged extraction and explicit host capabilities contain these risks.

Proposed defaults are: encrypted complete export; portable import with optional same-device identity recovery; referenced external files listed rather than bundled; foreground CLI runtime first; stdio MCP owns its runtime; desktop-only webview OIDC initially; modern MCP plus tested 2025 compatibility. These are planning assumptions, not implemented restrictions, and can be revised before their respective implementation stages.

No fixed delivery estimate is justified until the extraction/SDK/platform spikes finish. Relative effort: backup is medium, standard HTTP MCP is medium-to-large, and a truly independent CLI with runtime parity is large. This should be several reviewable changes, not one combined rewrite.

## External references checked

- [Official MCP transport overview](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports)
- [Streamable HTTP binding](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http)
- [stdio binding](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio)
- [Legacy 2025-11-25 transport contract](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
- [Protocol versioning and compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)
- [Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)

Recheck pinned SDK releases and actual client support at implementation time. Current specification support in upstream source is not proof that a particular published SDK release or user client implements every revision.
