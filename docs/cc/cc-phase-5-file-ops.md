# Phase 5 — File Operations (SCP + Remote Browser)

## Scope
- CC-driven `scp_upload`/`scp_download`; remote browser session control.
- Reliability, integrity, and safety guarantees for large and sensitive transfers.
- Durable transfer history and browser-session state on cc-server so Admin reconnect/refresh does not lose context.

## Phase 5 decisions locked in
- CC transfer operations use server-generated durable `transferId` values as the canonical identity.
- Remote browser operations use server-generated `browserSessionId` values; the agent may keep local handles internally but must map them back to `browserSessionId`.
- File transfers and remote browser sessions are single-operator owned in Phase 5, matching the PTY ownership model.
- Boundary-backed SCP over CC requires an active tunnel precheck before dispatch; the server should fail fast with an actionable reason when missing.
- Long-running transfers must emit explicit lifecycle + progress updates rather than relying on transport-level ack alone.
- Admin UI can start with a minimal queue/history + browser flow, but protocol reason codes and lifecycle states must be stable first.

## Commands
- `scp_upload`, `scp_download`, `start_scp_upload`, `start_scp_download`.
- Browser: `open_remote_browser`, `list_remote_browser_children`, `stat_remote_browser_path`, `create_remote_browser_directory`, `delete_remote_browser_path`, `close_remote_browser`.

## Protocol / payload contract

### Transfer commands
- `start_scp_upload` / `start_scp_download` are the primary long-running CC transfer commands.
- Request payload should include:
  - `transferId` (server-generated, durable)
  - `serverId`
  - source/destination paths
  - `recursive?`
  - policy flags: `overwritePolicy`, `followSymlinks`, `rateLimitKbps?`, `verifyChecksum?`, `preserveMtime?`, `preserveMode?`
- Legacy/simple `scp_upload` / `scp_download` may remain as thin wrappers if needed, but Phase 5 should treat the transfer-tracked variants as the canonical path.

### Transfer lifecycle updates
- Agent must emit explicit transfer updates keyed by `transferId`.
- Minimum lifecycle states:
  - `queued`
  - `starting`
  - `running`
  - `succeeded`
  - `failed`
  - `canceled`
- Progress payload should support:
  - `bytesTransferred`
  - `bytesTotal?`
  - `percent?`
  - `throughputBps?`
  - `etaSec?`
  - `phase?` (`precheck`, `transfer`, `checksum`, `finalize`)
- Terminal payload should include stable `reason` / `reasonCode` when non-success.

### Remote browser operations
- Each browser command is keyed by `browserSessionId` plus `serverId`.
- Browser responses should be stateless from the Admin point of view except for the durable `browserSessionId`.
- Browser mutation results should be auditable and should return normalized remote paths.

### Stable reason codes
Phase 5 should standardize at least these reason codes:
- `tunnel_not_running`
- `path_not_found`
- `path_already_exists`
- `overwrite_declined`
- `invalid_target_path`
- `invalid_source_path`
- `permission_denied`
- `rate_limited`
- `checksum_mismatch`
- `transfer_timeout`
- `transfer_stalled`
- `resume_not_supported`
- `resume_state_lost`
- `resume_offset_mismatch`
- `browser_session_closed`
- `agent_error`

### Admin reconnect expectations
- Refresh/reconnect must be able to restore transfer history by `transferId`.
- Refresh/reconnect must be able to restore or clearly invalidate remote browser state by `browserSessionId`.

## Agent
- Reuse existing SCP + SFTP with persistent session per browser modal.
- PTY-backed transfers for progress bars and live percentages.
- Normalize paths; confirm overwrite on uploads.

## Transfer reliability (resume/retry)
- Lifecycle payloads must expose:
  - `attempt`
  - `maxAttempts`
  - `resumeSupported`
  - `retryScheduled`
  - `resumeToken?`
  - `resumeOffsetBytes?`
  - `tempPathHint?`
- Phase 5 green-state implementation now includes:
  - the current one-automatic-retry slice for transient start/pre-transfer failures
  - true resumable transfer support for interrupted PTY-backed transfers when partial temp-file state is still trustworthy
- Automatic retry rules:
  - `maxAttempts = 2` is acceptable for the current Phase 5 slice
  - automatic retry is allowed only before durable transfer progress is observed
  - retryable reasons may include transient launch/server conditions such as:
    - `agent_error`
    - `rate_limited`
- Resume rules:
  - `resumeSupported = true` only when all of the following are true:
    - the transfer writes to a temp path first
    - the partial temp artifact still exists
    - the agent can prove the current resume offset
    - checksum/finalize has not yet completed
  - `resumeToken` is an opaque coordination token for the specific interrupted transfer attempt
  - `resumeOffsetBytes` is the next byte offset the resumed attempt should start from
  - resumed attempts must increment `attempt` while keeping the same durable `transferId`
  - no resume after partial progress unless the offset is validated against the actual temp artifact
- Resume must be rejected with stable reasons when state is not trustworthy:
  - `resume_not_supported`
  - `resume_state_lost`
  - `resume_offset_mismatch`
  - `checksum_mismatch`
- No resume after:
  - checksum verification has already failed
  - finalize/rename produced ambiguous destination state
  - the temp artifact was deleted or no longer matches the expected offset
- Timeouts:
  - `transfer_stalled`: no PTY output/progress activity observed for 60 seconds during the transfer phase; transfer is failed deterministically and resume remains unavailable.
  - `transfer_timeout`: hard total transfer runtime limit of 30 minutes; transfer is failed deterministically and resume remains unavailable.
  - Heartbeat `transfer_progress` events may be emitted while running, but they must not clear previously known progress fields in the server/UI state.

## Integrity and atomicity
- Checksum verification: optional `sha256` verification after transfer; failures mark as `failed: checksum_mismatch`.
- Atomic writes: upload to temp path (e.g., `<dest>.scp_tmp.<uuid>`) then `rename` into place on success to avoid readers seeing partial files.
- Timestamps/permissions: preserve `mtime` and mode when requested (best-effort on target OS/filesystem).

## Path rules and safety
- Overwrite policy: default `prompt` (explicit confirmation); `force` flag allows overwrite without prompt; `skip` keeps existing.
- Symlink handling: default `no_follow` for uploads, `follow` for downloads; explicit flag to allow following on upload (with audit note).
- Sparse files: preserve sparseness on supported filesystems (best-effort), otherwise fall back to dense copy.
- Windows paths: normalize separators and reserved names; reject invalid targets with explicit errors.

## Quotas and rate limits
- Admission/saturation backpressure is enforced server-side with `nack { code: 'rate_limited', rate_limited: true, retryAfterMs }` when file-op capacity is saturated.
- Daily byte-budget enforcement is now implemented at transfer start with a deployment-wide fallback tenant:
  - `tenantId = "default"` unless `CC_DEFAULT_TENANT_ID` overrides it.
  - budget source: actual persisted `bytesTransferred` from completed transfers since the current UTC day began.
  - `CC_FILE_OP_DAILY_BUDGET_BYTES <= 0` or unset disables the quota.
  - warning threshold defaults to 80% via `CC_FILE_OP_DAILY_BUDGET_WARNING_PCT`.
  - hard stop at 100% returns `nack { code: 'quota_exceeded', retryAfterMs, tenantId, usedBytes, budgetBytes }`.
- Uploads and downloads currently consume the same shared daily budget pool.
- Per-transfer bandwidth throttling via `rateLimitKbps` remains a protocol field; active byte-per-second shaping is still deferred.

## Boundary tunnel precheck
- For Boundary-backed servers, ensure the tunnel is running before starting `scp_*`; else return `failed: tunnel_not_running` with an action hint to start the tunnel.

## CC Server
- Persist transfer rows, lifecycle updates, and browser-session ownership/state in SQLite.
- Authorize browser and transfer actions with the same device-control model used in earlier phases.
- Relay progress/history to the Admin UI without losing terminal state on reconnect/refresh.
- Keep stable reason codes and response shapes consistent so future quota/rate-limit extensions stay backward-compatible.
- Enforce the current deployment-wide daily transfer byte budget before dispatching new transfer starts, while keeping the contract extensible for future real tenant models.
- Audit transfer lifecycle and browser mutations without storing secrets or sensitive file contents.
- Persist policy-review fields for file-op auditability: `overwritePolicy`, `followSymlinks`, `verifyChecksum`, plus stable `reasonCode` and sanitized `reasonDetail`.
- Persist enough resume metadata to truthfully restore/interrogate interrupted transfers:
  - latest `attempt`
  - `resumeSupported`
  - `retryScheduled`
  - `resumeToken?`
  - `resumeOffsetBytes?`
  - whether checksum/finalize already ran

## CC / Admin UI
- Two-pane browser; transfer queue with progress + history.
- Boundary-backed SCP requires active tunnel (precheck above).
- Transfer history shows running, success, and error entries with stable `reason` codes.
- Minimal first UI is acceptable, but it must support durable history restore and explicit overwrite confirmation UX.
- Resume-capable UX:
  - show whether an interrupted transfer is resumable
  - allow operator-triggered resume on the same durable `transferId`
  - clearly distinguish:
    - retry from zero
    - resume from partial offset
    - not resumable

## Acceptance Criteria
- Upload/download shows continuous progress and final status; retry/resume contract fields are exposed and used as designed.
- Transfer history survives Admin refresh/reconnect and remains keyed by stable `transferId`.
- Atomic rename prevents partial files from appearing at destination.
- Checksum verification (when enabled) detects corruption deterministically.
- Overwrite / checksum / path-safety behavior is enforced with stable failure reasons and remains reviewable through stored transfer metadata/audit context.
- Browser ops reflect remote FS changes within one refresh.
- Boundary-backed SCP without an active tunnel fails fast with `tunnel_not_running` and an actionable hint.
- Interrupted PTY-backed transfers can resume only when the temp artifact and resume offset are still trustworthy; otherwise they fail with stable non-resumable reason codes and do not silently retry from an invalid state.

## Risks
- Large file memory pressure → stream + chunked progress; avoid buffering whole files.
- Stale tunnel detection → explicit precheck and clear retry guidance.

## SLOs (Phase 5 File Ops)

- Phase 5 minimum observability slice exposes a lightweight operational summary/export path rather than full alerting infrastructure.
- Export path:
  - `GET /v1/file-transfers/slo-summary?deviceId?&windowHours?`
- Minimum summary fields:
  - throughput:
    - `throughputP50Bps`
    - `throughputP95Bps`
  - progress cadence:
    - `progressUpdateHzP50`
    - `progressUpdateHzP95`
  - finalize / atomic-rename tracking:
    - `finalizeAttemptCount`
    - `finalizeSuccessCount`
    - `finalizeSuccessRate`
  - sample metadata:
    - `windowStart`
    - `windowEnd`
    - `completedTransferCount`
- Summary values may be computed from persisted transfer rows plus lightweight per-transfer SLO fields:
  - progress event count / first progress timestamp / last progress timestamp
  - whether the transfer ever entered `finalize`
- This Phase 5 slice is for operator visibility only; paging/alerts and long-term time-series retention remain follow-up work.

## Test plan (Phase 5 File Ops)

- Resume/retry
  - Verify lifecycle payloads expose `attempt`, `maxAttempts`, `resumeSupported`, and `retryScheduled`.
  - Verify resumable payloads also expose `resumeToken?` and `resumeOffsetBytes?` when partial state is resumable.
  - Induce a retryable start/pre-transfer failure; verify:
    - attempt 1 reports `retryScheduled = true`
    - attempt 2 is executed automatically
    - final success/failure reports `attempt = 2`, `maxAttempts = 2`
  - Interrupt a PTY-backed transfer after partial progress; verify:
    - resumable state reports `resumeSupported = true`
    - resume uses the same durable `transferId`
    - resumed attempt continues from the reported `resumeOffsetBytes`
  - Destroy or invalidate the temp artifact; verify resume is rejected with:
    - `resume_state_lost` or `resume_offset_mismatch`
- Checksum verification
  - Corrupt a transferred file; verify `checksum_mismatch` detection and rollback (no partial file at destination).
- Atomic writes
  - Monitor destination during upload; assert temp file present, final file appears only after rename; no partial reads possible.
- Overwrite policy
  - `prompt`: verify UI confirmation required; `force`: overwrite without prompt; `skip`: existing file preserved.
- Symlink handling
  - Upload with `no_follow` (default): symlink rejected. With `follow` flag: target followed and audited.
  - Download with `follow` (default): symlink target downloaded.
- Sparse files
  - Transfer a sparse file; verify sparseness preserved on supported FS; dense fallback on unsupported.
- Windows path normalization
  - Test reserved names (`CON`, `NUL`), invalid chars; verify rejection with explicit error.
- Quotas and rate limits
  - Configure a small `CC_FILE_OP_DAILY_BUDGET_BYTES`; verify:
    - transfer starts below 80% budget succeed normally
    - transfer starts at/above 80% include a `quota_warning`
    - transfer starts at/above 100% are rejected with `quota_exceeded`
    - retry guidance points to the next UTC day via `retryAfterMs`
- Boundary tunnel precheck
  - Attempt SCP with tunnel stopped; verify `failed: tunnel_not_running` with action hint.
  - Start tunnel, retry; verify success.
- Remote browser session
  - Open browser, list/stat/create/delete; verify SFTP session persists across ops and closes on modal close.
- Progress and history
  - Transfer 50 MB file; verify progress updates ≥ 1 Hz and final history entry with success/error status.
- SLO summary endpoint
  - Verify `GET /v1/file-transfers/slo-summary` returns:
    - throughput P50/P95 for recent completed transfers with throughput samples
    - progress cadence P50/P95 for transfers with at least two progress samples
    - finalize attempt / success counts and success rate
