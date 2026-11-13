# Coordinated CLI import: admission and ownership contract

Status: admission design reviewed 2026-11-13; denial-only acquisition fence implemented.
No CLI import, recovery implementation or secret-bearing model ships. Except for
the acquisition fence described below, the matrices specify required future behavior.
Passing the shared desktop archive validators does not admit CLI import.

## Ownership acquisition audit

The source audit includes all tracked Rust references to `ProfileOwner`,
`ProfileRuntime::acquire` and `runtime-owner.lock`, verified against Ripwire uses,
impact and affected results. There are two production callers of
`ProfileOwner::acquire`; all other direct acquisitions are tests/fixtures.

| Entry or capability | Current route | Recovery gap |
| --- | --- | --- |
| `scpctl profile init` | `initialize_profile` → `ProfileOwner::acquire` | Denies recovery evidence before identity loading/creation |
| Linux `scpctl caddy run` | host `run` → `ProfileRuntime::acquire` → `ProfileOwner::acquire` | Denies recovery evidence before output creation or launch |
| Library runtime construction | Public `ProfileRuntime::acquire` delegates to the same owner constructor | Delegates to the fenced owner constructor |
| Library credential adapter | `ProfileSecrets::new` borrows an owner; read/refresh/write use that namespace | Borrow enforces lock lifetime only; no pending-recovery state or maintenance exclusion |
| `server list`, `config show` | Direct file readers, no owner acquisition | Can observe files during future recovery; currently intentional read-only behavior |
| Unit/integration fixtures | Core profile, credentials, sessions, foreground; CLI and Caddy lifecycle tests | Exercise direct/public constructors, conflicts and owner release; now include recovery-artifact admission tests |

`ProfileOwner` fields are private, with no alternate public constructor, Clone or
deserialization. Acquisition creates the directory/lock, takes a nonblocking OS
lock, then inspects recovery-artifact metadata before loading or creating
`profile.json`. That is the central ordinary-ownership fence.
An error drops `OwnerLock` and explicitly unlocks it. The persistent lock file is
never unlinked. Namespace validation requires version 1 and the CLI prefix plus a
UUID v4, and unknown metadata fields fail. This does not detect a copied identity:
manually duplicating `profile.json` duplicates its namespace. Supported migration
must never copy it, and selecting a desktop directory is unsupported. Shared
desktop/CLI profiles and protection against hostile same-user filesystem writers
are outside the current ownership guarantee.

Ripwire reports 32 transitive caller symbols and six affected test files with
unknown runners. Its uses list misses some nested test expressions; the textual
audit supplies those references. The meaningful production blast radius is profile
initialization, runtime/Caddy launch and owner-borrowing credentials. Any later
constructor edit must rerun impact/uses and test these public boundaries.

## Desktop recovery artifacts are foreign to CLI

`src-tauri/src/settings_backup/transaction.rs` owns these desktop artifacts:

| Artifact | Actual desktop interpretation | CLI decision |
| --- | --- | --- |
| `settings-restore-journal.enc` | 12-byte nonce plus AEAD data; AAD `SCP-RESTORE-JOURNAL-1`; private Journal version 1, committed flag, exact file bytes and credential undo values | Foreign recovery evidence; reject ordinary CLI admission without reading/decrypting/replaying it |
| Exact bytes `SCP-RESTORE-DONE-1` at that filename | Completion tombstone; desktop `load_journal` returns no work without a key | Still foreign evidence to CLI; do not recognize it as permission to proceed |
| `settings-restore-recovery-v1` OS account | Random 32-byte key encoded as 64 hex characters, in desktop service `com.example.supercontrolpanel`; outside backup accounts | Never probe, copy, replace or delete it from CLI |
| Orphan recovery key without a journal | Desktop permits a later prepare to replace it | CLI does not search desktop keyring to infer recovery status |
| `gate::BLOCKED`, revision and pending preview | Process-local desktop maintenance and stale-preview state | Neither persistent CLI fences nor cross-process authorization |

The desktop journal has **no target CLI namespace/profile binding**. Its dynamic
recovery-account allowlist is also broader than the archive server-ID parser:
nonempty, at most 256 bytes, no control characters. It must not become CLI's
allowlist. Desktop startup recovers before normal loaders/migrations/cache/MCP/CC;
undo-required journals roll back, committed journals retain the new revision.
Cleanup publishes a tombstone before removal/key deletion. Preserve that behavior.
Copying this implementation into core without a new binding would authorize the
wrong target accounts. An encrypted archive and a local undo journal are different
formats even when both use AEAD.

## Target and source admission matrix

Each row is required **before mutation**. A refusal leaves the source archive,
target identity/settings and both services' secrets intact. Error output uses
fixed classifications, never raw archive records, credentials or native errors.

| Condition | Required result |
| --- | --- |
| Another process owns target lock | Fail immediately; no competing preview/apply/recovery or runtime |
| Missing target identity, clean separate CLI directory | Explicit profile initialization may generate identity before import; archive validation/preview must not initialize it implicitly |
| Malformed/missing identity with pending recovery evidence | Refuse ordinary admission; never generate a replacement namespace to bypass recovery |
| Existing valid target identity | Retain exact `profile.json` bytes and namespace through preview, commit, rollback and restart; derive all account services from this owner |
| Any desktop journal path object, including symlink/directory/empty/corrupt/tombstone | Refuse ordinary admission; retain artifact; recover with desktop in its original profile, not CLI |
| Future CLI journal pending, corrupt, unknown-version, mismatched target, or uninspectable | Refuse ordinary ownership/configuration/credential/runtime operations; only separately qualified recovery may acquire restricted maintenance authority |
| No recovery evidence and valid identity | Eligible for the remaining validation/policy gates; absence alone does not authorize import |
| Source archive missing/extra sections/accounts, bad metadata/server record, tampering, unsupported schema or excessive size | Reject using full desktop v1 archive contract in its existing order, before mapping |
| Source identity/namespace, arbitrary service/account or extraction filename offered as mapping authority | Reject; only fixed section names and typed destination accounts may enter a plan |
| Read-only inspection while known recovery is pending | Future coordinated import must reject ordinary settings/inventory reads too; a separate redacted recovery-status query may report only state/reason |
| Ordinary runtime active or maintenance unavailable | Refuse import; do not silently stop it or wait while holding runtime mutexes |
| Destination changes or owner/namespace differs since preview | Expire preview; require fresh destination reads and confirmation |

## Account and settings mapping matrix

Validate the complete source inventory first: seven static desktop accounts plus
each `server_ssh_password:{id}`, including explicit absent declarations. Do not
validate a filtered map against CLI's smaller inventory and call it a complete
backup. Mapping produces backend-only typed actions, never a public Archive/Changes
DTO or an arbitrary `(service, account)` supplied by the archive.

| Source or target item | Proposed first CLI import policy |
| --- | --- |
| Five configured static accounts: OpenVPN password/OTP, Boundary password/OIDC password/OIDC OTP | Explicit one-to-one `CredentialAccount` variants in the **target** namespace; selected `Some("")` sets an empty value, selected `None` deletes, unselected keeps |
| Dynamic SSH account with valid CLI ID | Exact case-sensitive ID/account suffix; no truncation, substitution or renaming; selected action uses target namespace |
| Desktop-valid colon ID in any source or destination inventory participating in server replacement or secrets restore | Reject that selection before any backend call, even if its value is absent; no dropping credentials, colon removal or aliasing. Unselected server inventories may remain untouched when no action depends on them |
| Credentials selected without server records | Require exact matching source/destination ID sets and CLI-compatible IDs before mapping |
| Server list replaced | Include explicit deletes for removed destination SSH accounts even when secrets are unselected; show this dependency and snapshot all affected rollback values |
| Source `boundary_app_token` | Validate/preserve inside encrypted archive but never restore it; CLI's existing enum variant does not establish portability |
| Destination Boundary app token | Keep unless selected shared config changes controller/auth identity, then explicitly delete with rollback and preview; follow desktop identity comparison, including trailing-slash handling for address |
| Source `cc-telemetry-queue-key` | Validate inventory/value bounds but never map to a CLI variant/backend call; no CLI telemetry or queue migration |
| Destination desktop telemetry key, recovery key and other desktop accounts | Untouched; CLI must never open the fixed desktop service |
| CLI credential absent versus unreadable | Fresh successful `Ok(None)` alone means absent; Unavailable/InvalidData/Ambiguous and any required read failure abort admission, never delete/keep-as-success or fall back to cache |
| Selected `config.json` | Strip the five legacy plaintext credential fields before file staging; preserve other stored fields and existing path policy; do not run imported commands |
| Selected `sync-local.json`, portable mode | Keep MCP/CC disabled, rotate imported MCP token, clear copied CC token/device identity for explicit re-enrollment, preserve target paths by default or apply explicit validated overrides; no automatic activation |
| Same-device identity recovery requested for CLI | Reject in first CLI policy; CC is desktop-hosted and cross-host token recovery is unqualified. Existing desktop same-device option stays available and unchanged |
| Unselected settings | Keep exact target section; no source defaults, path overrides or identity reset applied to it |

Profile identity is distinct from CC enrollment identity and MCP access tokens.
Even a later qualified same-device option must retain the target CLI namespace.
Unavailable **required** credentials abort the operation even when the intended
action is keep or delete. No required rollback value may come from stale cache.
Scope required reads explicitly in the plan; unrelated orphan OS accounts are
never enumerated. Complete export still requires all declared source reads.

## Coordinated apply and recovery release gates

Future import uses exclusive per-profile maintenance for the full destination
snapshot → confirmation recheck → journal/apply → cache publication lifecycle.
No ordinary owner capability or credential adapter can be constructed from a
restricted recovery lease. Failure after journaling revokes ordinary in-process
access as well as blocking the next process; checking only the next acquisition
is insufficient. Recovery authority must not be an ordinary owner plus a public
`ignore_recovery` boolean. Current `ProfileSecrets` lifetime borrowing alone cannot
enforce those requirements.

A separately versioned CLI journal must authenticate its target namespace,
transaction identity, schema/policy version, bounded fixed filenames and typed
account inventory; recovery verifies the existing target identity before opening
the target namespace. Reserve `scp-cli-restore-journal.enc` as CLI recovery evidence
for the first fence; this reserves a filename only, **not a journal wire format**.
Recovery-key addressing must be profile-specific and outside imported account sets;
its representation, durable publication/cleanup, and restricted API need another
review before implementation. Reject foreign/mismatched/corrupt journals without
trying other services. Never regenerate identity or auto-delete artifacts to recover.

Keep the existing 32 MiB desktop encrypted-input bound, KDF bounds, 10,000 server
limit and 65,536-byte credential limit. Before any key/file write, also bound the
serialized CLI undo snapshot and staged data (proposed 32 MiB each); reject excess,
never truncate rollback. Source/destination server unions can exceed one archive's
inventory, and exact file-byte snapshots can exceed a parsed archive's size, so
per-value limits alone are insufficient. Native filesystem/keyring calls can
block: cancellation, bounded worker ownership and no lock release around live
mutating workers require host qualification; a timer is not evidence of completion.

Mutation remains disabled until fault-injection tests cover every journal/key,
credential, file-publication, rollback, committed-marker and cleanup stage; actual
subprocess death/restart must prove identity binding and no ordinary access during
uncertain recovery. Preserve private writes/ACLs, explicit absent/empty behavior,
cache refresh only after a settled revision, redacted preview, confirmation and
destination fingerprint checks. Archive/Changes stay off IPC/MCP/CC. This contract
does not resolve guardian death, process-tree containment or shared-profile safety;
those remain independent [supervision gates](LINUX_OPERATION_SUPERVISION.md).

## First prerequisite: acquisition fence (implemented)

A small **denial-only** fence is implemented inside `ProfileOwner::acquire`, after taking
the OS lock and before `load_or_create_identity`. Inspect the two exact journal
paths with `symlink_metadata`, without opening/reading them. Only `NotFound` means
absent; any object or other inspection error denies ownership with a fixed recovery
error. Check both names even though the CLI format/writer does not yet exist.
Do not recognize desktop tombstones, probe keyring, recover, remove/rename a journal,
or expose an override. Directory/lock creation may already have occurred; identity,
settings, credentials and artifacts must remain unchanged. Denial must release the
OS lock without deleting it. Ordinary cross-process ownership conflict keeps its
existing precedence.

Strict RED/GREEN tests cover:

- Each artifact name with bytes, empty file, directory and Unix symlink (including
  dangling link); the desktop tombstone is denied without a content exception.
- Existing identity remains byte-identical; missing identity stays missing; fixed
  errors omit artifact contents/targets. No credential backend is consulted.
- Direct core owner and runtime constructors deny; real CLI `profile init` exits 1
  with no stdout. Linux Caddy denies before output creation or launch.
- No-artifact initialization, stable namespace, owner conflicts and lock release
  continue to work. Test inspection-error denial portably through an injectable
  inspection seam or a reliable native fixture; do not claim chmod denies root.

This first fence intentionally does not enable import or recovery. A subsequent
gate must cover stat-only denial for existing read-only file queries without
initializing/locking profiles, in-process maintenance/revocation, and restricted
recovery authority before mutation becomes eligible. Cooperating processes must
hold the same lock when publishing future journals; stat checks do not prevent a
hostile external process racing filesystem changes. Filesystem alias/TOCTOU and
native Windows/macOS behavior need explicit qualification, not a portable-safety
claim from Linux tests.

Implementation evidence (2026-11-13): behavioral RED failures showed direct owner
and runtime acquisition succeeding, real init creating identity, and real Caddy
launching despite evidence. A private acquisition inspection seam also first
allowed injected PermissionDenied/Other errors, then denied them with the fixed
error. The seam is inaccessible to library callers; production always uses
`symlink_metadata`. No chmod/root assumption is used. Both names are tested with
bytes, empty files, directories, completion bytes, Unix existing/dangling symlinks,
existing/missing identity, settings preservation and retained/unlocked lock files.
The full all-feature locked core/CLI workspace passes **155 tests** (seven ignored
subprocess fixtures exercised by parents); strict Clippy, formatting and diff
checks pass. Ripwire reports no gating regressions or suppressions; remaining
findings are six Cargo-test reachability rows and three minor churn/module-size
rows. Matrix refactoring removed the initial new-test complexity/nesting findings.
Native Windows/macOS, desktop, SDK spike, UI, live keyring, release and
MSRV checks were not rerun for this acquisition-only change. No dependency, schema,
command/event, recovery format or native supervision changes.
