# Linux operation supervision: qualification gate and proposed contract

Status: design only, 2026-11-11. No production supervisor, cgroup adapter, output
limit, watchdog, or new cleanup guarantee is implemented. The foreground Caddy
preview retains its documented cooperative direct-child behavior.

## Capability spike on this devbox

As uid/gid 1002 (`boxes`), `/proc/self/cgroup` selected
`/boxes_workload.slice/boxes-workload-bf4b2f257575420397a13f2bc0f66c2c.scope`.
`findmnt` reported a cgroup2 mount with `rw` and `nsdelegate`. The scope directory
was root-owned mode 0755; `cgroup.procs` and `cgroup.kill` existed but were not
writable by this user. Attempting `mkdir` for one uniquely named
`scpctl-qualification-<random UUID>` child returned **EACCES (13)**. No directory
was created, process migrated, control file written, or process signalled.

This is a missing delegation gate, not evidence that cgroup v2 is unavailable on
Linux. Neither a mount option nor interface-file existence qualifies containment.
Do not retry this spike in each continuation on an unchanged host. Qualification
requires a separately provisioned delegated test environment; this increment does
not request privilege escalation or change managed-machine permissions.

The [kernel cgroup v2 documentation](https://docs.kernel.org/admin-guide/cgroup-v2.html)
specifies inherited membership on fork, subtree-wide `cgroup.kill`, and
`cgroup.events` populated notifications. Migration permissions constrain delegation;
a private child directory alone does not prevent a same-user workload from moving
elsewhere within its delegation. `populated=0` excludes zombies and does not prove
reaping. These interfaces inform the candidate below, not a shutdown-time guarantee.

## Proposed ownership and execution boundary

- A private guardian owns the exclusive CLI profile lock and outlives the command
  frontend. It stays outside the operation cgroup. Only the guardian may authorize
  profile release; the PTY/log helper receives no owner-lock descriptor, keyring
  access, desktop settings, or authority to start another operation.
- Before allowing operation execution, the guardian checks delegation, creates a
  dedicated subtree, establishes control handles and the cancellation path, and
  confirms the helper's membership. A trusted bootstrap must wait for an explicit
  start authorization before launching Caddy or forking operation descendants.
  Starting Caddy and then migrating its PID is insufficient. All operation and
  output-worker descendants must inherit the contained boundary.
- Qualification must prove the workload cannot migrate out of that boundary or
  pass live output descriptors to uncontained processes. Same-user writable cgroup
  ancestors, privileged helpers and hostile workloads are unresolved gates, not
  assumptions satisfied by `setsid` tests. No PID enumeration/process-group fallback
  or automatic privilege acquisition is allowed. Unsupported capability fails
  before Caddy launch; prelaunch artifacts are reported and never auto-deleted.
- The contained helper owns the PTY and performs all raw reads and log
  create/write/flush work. The guardian never performs those potentially blocking
  calls. Log setup retains exclusive new-file creation and private Unix mode.
  A proposed first limit is **64 MiB of raw bytes per invocation**, fixed before
  launch. The helper checks the remaining budget before writes; an over-limit chunk
  is not written. Exceeding it fails the operation and requests teardown, retaining
  an explicitly incomplete log. No rotation, silent truncation, or success result.
  This proposed limit does not apply to the existing unbounded preview.

## Control acknowledgments, deadlines and results

The proposed private control protocol has a version, invocation nonce and ordered
states: `contained-ready` → `start-authorized` → `execution-exited` /
`cancel-requested` / `io-failed` → `output-stopped` → `cleanup-confirmed`.
Frames are capped at 4 KiB with a fixed bounded number of pending frames. They carry
only state, byte counts and exit/error codes; no transcript, raw output, arbitrary
native error strings, or credentials. Malformed, oversized, reordered, stale or
missing acknowledgments and disconnected peers initiate failed cleanup. No blocking
control send may prevent the guardian from servicing cancellation/deadlines.

Candidate monotonic deadlines are 2 seconds for ready acknowledgment, 2 seconds
for a requested cooperative stop, 2 seconds for final output drain after execution
exit, and 1 second to observe containment teardown after escalation. Repeated
signals only advance monotonic cancellation; they never bypass ownership checks.
Expiry escalates to teardown of the dedicated operation subtree. These are proposed
escalation triggers to qualify, **not maximum return or release times**. A native
call in an uninterruptible state can outlive them.

A completion result requires both execution exit and successful output drainage
and flush. Cancellation may discard pending bytes and skip flush. An I/O/log-limit
or protocol failure takes precedence over cancellation or a zero exit; cleanup
failure is attached without replacing that primary failure. Partial logs remain.
Only confirmed subtree emptiness, helper exit/reaping, and stopped output workers
permit release. A kill request, elapsed timer, helper result or vanished direct
child alone is insufficient. Escalated output loss is never successful completion.

## Crash and retained ownership gate

If cleanup cannot be confirmed, the guardian must retain the lock and expose its
identity, invocation state, last acknowledgment and failed cleanup phase to its
launching host over the private control connection. A frontend disconnect requests
cleanup; it does not transfer ownership to an untracked task. A frontend may return
a failure with retained ownership only when that guardian is observable through an
independent, surviving host. No such host adapter exists today.

Guardian death itself releases an ordinary OS lock. An on-disk marker alone cannot
keep that lock held; the current `ProfileOwner` does not enforce crash recovery for
operation trees. Therefore host/guardian crash safety requires an external lifecycle
owner or a validated recovery interlock in **every** profile-acquisition path, plus
an observable cleanup/recovery workflow. This remains a release gate even if a
future devbox permits cgroup creation. Do not ship a claim of fail-closed retained
ownership using the current lock alone.

Required native qualification includes prelaunch denial, membership before exec,
session-escaping descendants, attempted migration/descriptor escape, blocked log
I/O, log overflow, launch failure, control-channel saturation/loss, repeated signals,
frontend and guardian death, kill failure, delayed native termination, reaping and
profile conflicts throughout cleanup/recovery. Windows/macOS need equivalent native
ownership/containment qualification. Keyring/elevation hosts remain gated.

The [CLI import admission review](CLI_IMPORT_ADMISSION.md) audits every current
profile acquisition route and proposes a denial-only settings-recovery artifact
fence. That prerequisite is still unimplemented and does not prove guardian/tree
recovery, retain a lock after death, or authorize reuse of desktop undo journals.
Operation recovery needs its own qualified evidence and lifecycle authority.
