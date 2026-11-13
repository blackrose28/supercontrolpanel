# CLI maintenance and revocation lifetime audit

Status: audit baseline `92a10b2`, updated 2026-11-13 with runtime and credential entry denial
implemented. No new public authority, command, recovery format, keyring probe or
bypass is introduced.
The [admission contract](CLI_IMPORT_ADMISSION.md) remains controlling for archive
validation, target binding and account mapping. Acquisition and stat-only read
fences and the runtime/credential entry slices below are implemented; serialized admission,
active supervision and publisher exclusion remain future work. “Revocation” below
means refusing subsequent ordinary operations after
observing recovery evidence, not erasing values already returned or interrupting
native calls already in flight.

## Source-backed entry and lifetime matrix

Audit scope: tracked core/CLI Rust production sources, public entry points and
fixtures. Ripwire orientation and qualified constructor uses were checked against
textual references and source bodies; bare type uses returned zero and cannot
establish absence. There remain exactly two production direct owner acquisitions:
CLI initialization and core runtime acquisition. The CLI has no credential command
or production `ProfileSecrets::new` caller. Desktop has separate loaders, caches
and [maintenance gate](../../src-tauri/src/settings_backup/gate.rs); importing its
process-global/thread-local reentrancy mechanism would not coordinate CLI processes.

| Entry / owner | Actual lifetime and effects today | Requirement after evidence appears following acquisition |
| --- | --- | --- |
| [`ProfileOwner::acquire`](../../crates/scp-core/src/profile.rs), CLI [`initialize_profile`](../../crates/scp-cli/src/main.rs) | Private owner fields, no Clone/alternate constructor; directory → persistent lock file → nonblocking exclusive lock → two metadata checks → load/create identity. Init holds owner through stdout write. Owner now stores identity, lock, acquisition-time absolute directory and a private atomic monotonic runtime latch | Preserve acquisition precedence. Runtime entries now recheck evidence through the retained owner and latch denial; credential entries share that denial; no replacement identity or unlock/reacquire workaround |
| `ProfileOwner::credential_namespace`, [`ProfileRuntime::credential_namespace`](../../crates/scp-core/src/host.rs) | Return borrowed namespace from memory, infallibly, without I/O; callers can copy the string | Remain identity inspection, never a maintenance permit. Cannot retract a copied namespace; trusted hosts must not turn this string into a recovery bypass |
| `ProfileRuntime::acquire` | Private `SessionStore`, nonce/counter, then owner in declaration/drop order. Production caller is Linux [`caddy_host::run`](../../crates/scp-cli/src/caddy_host.rs) | All sessions/workers must finish cleanup before owner release; no new runtime from revoked owner |
| `ProfileRuntime::spawn` | Reserves fresh ID before native PTY setup; returns `RuntimePty` | Deny before ID reservation, PTY setup, reader/writer creation or child launch; evidence/inspection failure must beat launch errors |
| `ProfileRuntime::write` | Owner entry check before registry lookup then writer write/flush | Deny before lookup and any input, even for unknown ID or zero-length input; do not send a “cleanup command” through ordinary PTY input |
| `run_foreground`, `run_foreground_cancellable` | Both check owner at entry and launch via fenced spawn; cancellable checks before pre-cancel. Scoped output worker joins before return. Internal exit loop calls `monitor.poll` directly | Recheck at entry, including pre-cancelled call; preserve the cleanup routes below. Entry fencing alone cannot detect evidence during blocked I/O; active observation/cancellation is separately gated |
| `ProfileRuntime::close`, `poll`, cleanup guard, runtime drop | Close removes session, kills/waits direct child; poll observes/retires original launch; drop releases sessions before owner | Remain permitted under denial. Do not require ordinary admission for kill/wait/poll/drop or worker join. Preserve payloads, idempotent close and launch-bound polling |
| `RuntimePty { pub reader, private id/monitor }` | Reader can escape runtime lifetime; weak monitor cannot retain child. Host can read output without calling runtime. No strong session handle escapes `ProfileRuntime` | Already issued raw readers cannot be revoked by owner state. Allow teardown drainage/discard; no new reader capability. Active-output secrecy/stop guarantees require a separately designed host adapter |
| [`ProfileSecrets::new`, `read`](../../crates/scp-core/src/credentials.rs) | Borrows owner plus mutable backend/cache. `key` uses namespace and typed account. Owner check in mutable private `key` precedes account validation and cache/backend access; accepted cache hit clones secret/absence | Constructor alone cannot protect later calls. Read now checks owner before account validation/cache lookup/backend; cached Some/empty/None all deny. Already returned/cloned `SecretValue` is outside revocation |
| `ProfileSecrets::read_fresh`, `write` | Fresh evicts key then calls read. Write evicts before OS call, seeds cache only on success; failed native mutation may have unknown outcome | Owner check precedes fresh eviction-to-native and all mutations, including delete. On denial evict affected namespace entries (or clear cache), never seed/return cache; freshness is no recovery authority. Keyring deletion/rollback is mutation, **not permitted ordinary cleanup** |
| `CredentialCache::clear`, `SecretValue` drop | Cache owns zeroizing values; clear drops them. Independently returned clones survive. No global cache | Clear/drop always permitted without stat/keyring; no promise to erase external clones or OS/allocator copies |
| [`SecretBackend`, `OsSecretBackend`](../../crates/scp-core/src/credentials/os.rs) | Trusted low-level adapter accepts namespace/account strings; public/default constructible, no owner. Native calls may prompt/block. Native errors still classified to the original four `SecretError` variants | Not a security sandbox against arbitrary Rust callers. Host must retain owner/ordinary permit through every accepted call and join, avoid raw adapter routes, and never release on timeout. Profile adapter now returns distinct fixed `RecoveryRequired`; native mapping never synthesizes recovery denial. Raw backend has no profile policy |
| [`read_shared_config`](../../crates/scp-core/src/config.rs), [`read_servers`](../../crates/scp-core/src/servers.rs) | CLI show/list only production callers. Each does stat-only recovery fence then private [`read_json`](../../crates/scp-core/src/config_file.rs). No directory creation/lock/identity/keyring | Deny observed evidence before absence/parse/I/O. Future publisher exclusion must hold a read lease across stat → file read → parse/projection, not only the preflight |

The lower-level [`SessionStore`, `PtySession`, `spawn_pty`, `SessionMonitor`](../../crates/scp-core/src/sessions.rs)
are public shared desktop primitives: registry contains strong Arcs, `remove` can
return one, child/master/writer mutexes are public for desktop compatibility.
`ProfileRuntime` privately contains its store and exposes only a reader/weak monitor.
Do not retrofit CLI recovery policy onto those shared primitives or claim they
require CLI ownership. Desktop/CC/admin/MCP contracts stay unchanged.

## File and host boundary inventory

The ordinary core/CLI profile reads are exhausted by these paths:

- `profile.rs::load_or_create_identity`: reads `profile.json` only under acquired
  owner, after the acquisition fence; new identity uses private staged persistence.
- `config_file.rs::read_json`: `read_to_string` reached only from the two fenced
  readers above. Metadata checks inspect journal path objects, never contents.
- `sync-local.json`: shared schema/patch helpers in
  [`local_settings.rs`](../../crates/scp-core/src/local_settings.rs) perform no file
  load/save. No current CLI local-settings reader, backup reader or migration.
- [`build_caddy_command`](../../crates/scp-core/src/caddy.rs) tests folder/Caddyfile
  existence before owner acquisition; it does not read Caddyfile contents.
  Executable resolution and Caddyfile consumption belong to the child. Linux host
  installs signals, acquires runtime, then exclusively creates the raw output file,
  then runs/reports. Non-Linux host fails before creating profile/output. The new
  output is a write handle, not an ordinary settings read; recovery appearing
  between acquisition and launch could leave an empty output file after denial.
- Remaining shared core operations prepare commands/operate supplied handles;
  tests read fixture settings/output/PIDs. These are not hidden CLI profile loaders.
  Desktop file/secret loaders and SDK synthetic fixtures are separate authorities.

Core owns profile evidence classification, the per-owner runtime latch and admission
at public core operations, cache semantics and private session cleanup. Host owns
signal handling, raw output, active-loop observation/cancellation, native worker
lifetime, user interaction, reporting and eventual profile release. Checking only
CLI command dispatch leaves public core calls unprotected. Checking only spawn
leaves existing input/cache access unprotected. Moving checks to generic sessions
would change desktop behavior without establishing profile identity.

## Denial, inspection and cleanup precedence

Implemented runtime state is monotonic for one acquired owner: usable →
revoked; runtime and credential entries consult the same latch. Inspect the exact desktop filename first, then reserved CLI filename;
only NotFound is absence. Any object or other error revokes. Once revoked, removing
an artifact or making a later stat succeed must not resume that owner. No reset,
public injected inspector, public boolean bypass or ordinary recovery constructor.
Retain original lock and namespace until normal cleanup/drop. Fresh acquisition
still uses existing lock-first ordering and retained-artifact fence; the latch is
not durable crash retention and cannot prevent manual artifact deletion.

Keep existing acquisition errors (directory/open/lock, competing owner, recovery,
then identity) and read errors (recovery before settings absence/parse/I/O).
Runtime entry denial uses fixed text:
`CLI profile recovery evidence is present or cannot be inspected; ordinary runtime operations are unavailable`.
It precedes session lookup, counter exhaustion, native setup and pre-cancellation.
Credential entry denial returns `SecretError::RecoveryRequired`, displayed as
`CLI profile recovery evidence is present or cannot be inspected; ordinary credential operations are unavailable`.
It precedes account validation, cache access/eviction-to-native and backend calls;
denial drops the entire affected namespace cache, including cached absence.
A clean pre-cancelled call still returns Cancelled without launch. Namespace/ID
inspection, close/poll, cache clear/drop and worker join require no evidence check
and must not turn cleanup into this denial error. Do not expose journal content,
link target, native inspection error, secret or arbitrary path in new diagnostics.

For active foreground work, current I/O/cleanup failures take precedence over
cancellation. `ForegroundCleanup::finish` returns the primary error and appends
`cleanup failed: ...` if close fails; `join_foreground_output` joins even after a
failed close, preserving stream failure plus cleanup failure. Preserve that order.
A future active-recovery supervisor must request teardown, join even when cleanup
fails, and report recovery failure only after settlement; I/O/cleanup failures
remain explicit rather than being replaced by Cancelled/success. Panic guards
still attempt close. Do not weaken existing raw native error contracts under the
guise of this fixed inspection boundary. No timer proves native completion.

Do not insert checks into close or poll to detect active revocation: those are
cleanup operations, and `supervise_foreground` bypasses public poll in its internal
loop. A separate explicit observation path is required for active supervision;
blocked reads/writes/waits remain uninterruptible in the current design. Even
successful direct-child cleanup does not qualify descendants or guardian death.

`SessionStore::close` removes the registry entry before native kill/wait and ignores
the kill result; wait failure returns an error. `SessionMonitor::poll` can retire an
entry with `wait_error: ...` and no exit code. Final `PtySession::drop` retries
kill/wait best-effort and discards failures. Therefore an Exited variant or empty
registry alone is not proof of confirmed native teardown. This audit preserves
those existing contracts and does not upgrade them to retained-ownership safety.

## Future cooperating publisher: exclusion without deadlocks

A process-local latch or mutex cannot serialize the two ordinary readers in other
CLI processes. Taking a shared lock on `runtime-owner.lock` also conflicts with
Caddy's exclusive owner, breaking today's clean reads during runtime. The stat-only
check cannot become a lease merely by checking again after reading. No publisher
may ship using these mechanisms as evidence of exclusion.

Candidate for a later design/implementation: a **separate persistent cross-process
read/maintenance lock**, plus matching per-profile in-process shared/exclusive
leases (do not assume native locks conflict across threads/handles identically on
all OSes). Readers hold shared leases across inspection and the complete read;
publisher retains exclusive runtime owner, acquires exclusive maintenance before
any destination snapshot/recheck/journal publication, and retains it through
settlement/cache revision publication. It refuses active sessions/accepted workers,
never silently cancels them. Ordinary operations fail promptly on contention;
maintenance also fails/retries rather than waiting while holding runtime locks.
Cleanup never needs either admission lease. No upgrade from a held shared lease.

Required order: runtime owner for publisher → local admission lease → cross-process
maintenance lease → short cache/session locks. Readers take only the two admission
leases. Never acquire an owner while holding admission; never join/native-wait or
prompt while holding cache/session locks. A required credential worker may retain
an admission lease while native I/O runs, so maintenance contention must be explicit
and bounded without detaching that worker. Preview confirmation must release
maintenance then reacquire/revalidate; no lease across human think time. Apply and
uncertain rollback retain exclusion; no worker outlives its authority.

This candidate **requires changing current no-lock/no-created-file read behavior**
on existing profile directories. A missing rendezvous cannot be treated as an
unlocked read while a publisher can concurrently create it. Both readers and
publishers must open/create the same persistent lock before access, never unlink
it; an absent profile can return absence without file access at a defined point
before concurrent initialization. Provisioning, symlink/alias handling, directory
replacement, access modes/read-only directories, native Windows/macOS lock behavior
and no-profile races require tests and explicit documentation before enabling
publication. Older binaries that ignore the new lease cannot cooperate: no mixed
version/shared-profile guarantee. Preserving strictly side-effect-free reads and
introducing serialized publication without a pre-existing stable coordinator are
incompatible promises. This is a later compatibility review, not an implemented
lock filename/protocol or a reason to enable mutation now.

## Smallest enforceable follow-up and acceptance plan

Completed increment: **runtime entry denial after acquisition only**. The owner
retains the selected profile directory and a private monotonic revocation latch;
a crate-private check fences runtime spawn/write and both foreground entry points.
It reuses the exact metadata predicate; production always uses symlink_metadata.
Retained path selection must refer to the acquisition-time directory even if a
caller later changes working directory; qualify relative paths without weakening
existing acquisition error precedence. Symlink/alias hostility remains out of scope.
Keep signatures, identity inspection, cleanup, desktop primitives and command/event
payloads intact. This can be implemented and reviewed without any maintenance or
recovery authority, new lock protocol, credential error API or import command.
It does not promise active-operation monitoring; credential/read serialization
and authenticated recovery remain separate gates. No live keyring or cgroup probe.

The runtime and credential acceptance rows below now pass following behavioral RED; later rows remain gates:

| Scope | Concrete acceptance cases / evidence |
| --- | --- |
| Implemented: late artifact | Acquire clean runtime, launch a fake existing session, publish each exact artifact, then call spawn/write and both foreground entries. Fixed denial; fake setup/input counters unchanged; settings/identity/artifacts byte-identical. Both true/false cancellation inputs deny; clean precancel remains Cancelled |
| Implemented: metadata and latch | Bytes/empty/directory/DONE and Unix existing/dangling links; injected PermissionDenied/Other at each inspection position via private seam with exact order. After first denial remove evidence and prove same owner still denies; independent clean profile unaffected; no keyring access |
| Implemented: cleanup | With late evidence and after latch, close still returns original payload/None, poll still Running/Exited/Detached as appropriate, drop kills/waits while lock retained. Known/unknown IDs do not bypass write denial. Reader/weak monitor cannot extend owner. Existing cleanup-error/panic and blocked-worker join tests remain green |
| Implemented: precedence/preservation | Recovery beats invalid session/native setup and ID exhaustion using private unit access where necessary. Preserve ordinary acquisition conflict, missing-directory reads, owned-profile reads, namespace/ID inspection, relative profile selection across a later cwd change (isolated subprocess), and non-Linux host denial. Do not assert ID values as a new external format |
| Implemented: credential gate | Existing adapter then late evidence: cached present/empty/absent reads, fresh read, set/delete and invalid account all deny before backend. Counts unchanged; cache evicted; clear/drop allowed; latch persists after evidence removal. Existing native failure classification, namespace isolation and compile-fail owner borrow remain green |
| Later: exclusion | Channel/barrier-controlled in-process and subprocess readers pause inside actual file read while publisher attempts exclusive access; no publication until last reader settles. Reverse case denies promptly before file access. Test acquisition failure, first lock creation, missing profile, read-only directory, alias/links and process death; no sleeps as proof of ordering |
| Later: active lifetime | Evidence during drain, poll loop, blocked sink and native credential worker: no detach/early unlock; close and join remain possible. Assert recovery vs I/O/cleanup/cancel precedence. Test reader escape honestly; do not claim asynchronous revocation from entry tests |
| Later: authority | Separate restricted authenticated target-bound recovery and fault injection/crash restart across every publication/rollback/cleanup stage. No ordinary owner/adapter from recovery lease; full source validation/mapping still required |

Use existing [session fake/call-order tests](../../crates/scp-core/tests/sessions.rs),
[foreground blocked sink](../../crates/scp-core/tests/foreground.rs),
[owner/artifact matrix](../../crates/scp-core/tests/profile.rs),
[credential counted backend](../../crates/scp-core/tests/credentials.rs) and
[read preservation tests](../../crates/scp-core/tests/read_recovery.rs).
Run impacted tests first, then locked all-feature workspace, strict Clippy, fmt,
diff and Ripwire edit/quality checks for that code slice. Runtime tests were added
for this implementation; the original audit was docs-only. Native platform,
release/MSRV, live-keyring
and [operation crash-retention](LINUX_OPERATION_SUPERVISION.md) gates remain open;
SDK/HTTP/logging and packaging can advance independently.

## Runtime entry implementation evidence (2026-11-13)

Behavioral RED showed late evidence allowing fake spawn/input and returning session
lookup errors; the isolated cwd fixture also failed. Private seam RED showed
inspection errors accepted and no ordered checks; the counter test reached native
setup. GREEN reuses the exact two-name absence predicate and always supplies
`fs::symlink_metadata` in production. Any object or non-NotFound error latches the
owner's private atomic state; a latched owner skips later inspection. No reset,
public seam, constructor, Clone or signature change exists.

The retained directory uses `std::path::absolute` after directory/open/lock,
recovery inspection and identity loading, preserving existing acquisition error
precedence. It does not canonicalize or qualify hostile aliases/replacement.
An isolated subprocess proves relative selection across a later cwd change.
Both foreground entries check before dispatch (spawn also checks), with recovery
before pre-cancel, ID reservation/exhaustion and native setup. Known/unknown IDs
and empty input deny before lookup/write/flush. Inspection and entry acceptance
are not serialized maintenance leases and cannot stop races with external writers.

Tests cover both names, bytes/empty/directory/DONE and Unix links/dangling links,
private PermissionDenied/Other at each position/order, removal with persistent
denial, independent profile, byte preservation, no additional fake setup/input,
namespace/ID inspection, allowed close/poll/drop and existing native error payloads.
Existing cleanup failures, panics, cancellation and blocked scoped-worker joins
remain required regression checks. An empty registry/Exited is still not native
teardown confirmation. The owner remains held through existing cleanup/join paths.
No active recovery observer or credential gate was added in that runtime increment. Issued readers and secret
clones cannot be retracted; blocked native calls cannot be interrupted. A Caddy
entry denial after output creation may leave a new empty output file.

The credential entry increment below completes the next slice. Read/publisher
serialization, restricted authenticated recovery, archive mapping and active
supervision remain gates before import or full in-process maintenance can ship.

Validation: locked all-feature workspace 169 passed, eight ignored subprocess
fixtures exercised by parents (including compile-fail ownership doc test); strict
all-target/all-feature Clippy, fmt, diff and Markdown target/fragment checks pass.
Ripwire unchanged-signature edit checks pass; quality delta has zero gating
findings/suppressions. No desktop/UI/SDK/native Windows/macOS/live-keyring/MSRV/
release rerun or cgroup probe is claimed.

## Credential entry implementation evidence (2026-11-13)

`ProfileSecrets::key` now checks the retained owner before account validation,
cache lookup or eviction-to-native and backend access. Its receiver becomes
private `&mut self` so denial can evict every entry in that owner's namespace;
other namespaces remain cached. All three public methods keep their signatures.
`read_fresh` checks before eviction and again through `read`. No public inspector,
constructor, authority or reset is introduced. Production reuses the owner's exact
ordered two-name symlink_metadata predicate and private monotonic runtime latch.
Credential observations also deny later runtime entry; runtime/inspection denial
also denies credentials. Artifact removal and cache clear never reset that owner.

`SecretError::RecoveryRequired` is a new public enum variant, with the fixed text
above. This intentionally requires external exhaustive matches to handle recovery;
all tracked producers/consumers were audited. Only the profile adapter produces it;
native error conversion remains unchanged. Missing is still `Ok(None)`, empty is
still present, and native failures retain their classifications. No production CLI
keyring command or `ProfileSecrets::new` caller exists. Raw backend and desktop
APIs remain trusted building blocks; this is no sandbox for arbitrary Rust hosts.

Behavioral RED: late evidence returned cached data/absence and invalid account
errors instead of denial (`/tmp/scp-credentials-red.log`). GREEN covers both names,
existing adapter followed by evidence, cached present/empty/absent, read/fresh/set/
delete and invalid-account precedence, unchanged backend counts, namespace-wide
cache eviction, independent profile cache preservation, clear/drop and removal
latch. Empty/DONE/directory and Unix link/dangling objects remain untouched.
Private inspector PermissionDenied/Other at each position feed the same latch;
existing ordered inspection tests remain green. Mock backend calls assert the
owner lock remains held through reads, sets, deletes and native failures. The
compile-fail borrowing test and all existing runtime/cleanup tests remain green.

Scope is entry observation only, with no serialized snapshot/publication or
external-writer exclusion. No active monitor, native interrupt, escaped reader or
cloned-secret revocation, durable/crash latch or recovery authority is established.
Clear/drop and joins remain cleanup; keyring deletion and rollback remain mutation.
No live keyring/cgroup probe, desktop/CC/admin/MCP contract or schema change.

Validation: locked all-feature workspace 176 passed, eight ignored subprocess
fixtures exercised by parents (including compile-fail ownership); strict all-target/
all-feature Clippy, fmt and whitespace pass. Ripwire edit checks show no incompatible
calls; manually reviewed enum addition/private receiver change are beyond its
signature lens. Quality delta exits 0, gating 0 and acked 0: ten new-symbol Cargo
or trait reachability rows plus one intentional duplication row for the mock's two
native methods. No existing-symbol regression. Native Windows/macOS, live keyring,
MSRV/release, SDK/desktop/UI and cgroup qualification are not rerun.
The staged static test gate exits 4: 13 unknown-runner test modules (all executed
by Cargo) and 148 untested reachability rows, including unchanged desktop/CC and
documentation-derived paths. This is not a claim that the index proves native or
trait/subprocess coverage; direct source review establishes the adapter boundary.

Next focused increment: a bounded source-backed production stdio diagnostics/error
policy review, using the pinned SDK and existing raw-debug/startup-error evidence.
Specify a concrete fixed startup diagnostic and SDK subscriber boundary with an
acceptance matrix before enabling any production stdio host. Do not claim a generic
sanitizer or alter the raw characterization fixtures. HTTP shipping policy, other
operation contracts and packaging remain independently actionable. Reader/publisher
serialization and authenticated restricted CLI recovery remain gates, not reasons
to stop independent work. Continue in a NEW same-devbox thread after each completed
increment while actionable work remains, with notifications and no parent wait.
