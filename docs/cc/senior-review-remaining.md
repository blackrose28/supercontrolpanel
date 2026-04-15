# CC Senior Review — Remaining Items

This file tracked the items from the senior review that remained after the earlier Phase 5 file-ops work. It is now aligned to show that the Phase 5 senior-review scope has been completed for the currently agreed slice.

## Already addressed

- Resume/retry scaffolding fields added:
  - `attempt`
  - `maxAttempts`
  - `resumeSupported`
  - `retryScheduled`
- Boundary tunnel precheck implemented for Boundary-backed file operations.
- End-to-end transfer progress is visible in CC admin.
- Upload-to-directory path resolution fixed.
- PTY checksum comparison bug fixed.
- Progress-bar flicker caused by heartbeat progress frames fixed.
- Overwrite-policy UX in CC admin improved.

## Review status

### 1) Quotas / byte budgets

Status: ✅ Implemented (deployment-wide default-tenant slice)

Implemented:

- Daily transfer-byte budget is enforced at transfer start.
- Current tenant model is a deployment-wide fallback tenant:
  - `tenantId = "default"` unless overridden by env.
- Actual persisted `bytesTransferred` from completed transfers since the current UTC day began is used for accounting.
- 80% warning threshold is supported.
- 100% hard-stop threshold rejects new transfer starts with:
  - `nack`
  - `code: quota_exceeded`
  - `retryAfterMs`
  - `tenantId`, `usedBytes`, `budgetBytes`, `thresholdPct`

Notes / remaining follow-up:

- Uploads and downloads currently share one budget pool.
- True multi-tenant quota partitioning is still future work if product needs it.

### 2) Server-side rate limiting / saturation nack

Status: ✅ Implemented

Implemented:

- CC server now rejects saturated file-op admission with:
  - `nack { rate_limited: true, retryAfterMs }`
- Active-capacity admission is enforced for:
  - transfer starts
  - remote browser opens

Notes:

- This is distinct from generic request-count rate limiting.
- Current implementation uses configurable active-capacity thresholds.

### 3) SLO tracking / monitoring

Status: ✅ Implemented (lightweight summary/export slice)

Implemented:

- Throughput tracking:
  - P50
  - P95
- Progress update frequency monitoring
- Atomic rename/finalize success-rate tracking
- Lightweight operational summary/export path:
  - `GET /v1/file-transfers/slo-summary?deviceId?&windowHours?`

Notes:

- This is a lightweight observability slice, not a full time-series/alerting system.
- Current metrics are derived from persisted transfer rows plus lightweight per-transfer SLO fields:
  - `progressEventCount`
  - `firstProgressAt`
  - `lastProgressAt`
  - `finalizeSeen`

### 4) Chunk/stall timeout contract

Status: ✅ Contract defined and minimally implemented

Implemented:

- `transfer_stalled`
  - emitted when no PTY output/progress activity is observed for 60 seconds during transfer
- `transfer_timeout`
  - emitted when total transfer runtime exceeds 30 minutes
- Docs aligned so heartbeat progress frames do not clear previously known progress

Notes:

- This is a practical Phase 5 contract, not a full chunk-resume system.
- If needed later, chunk-aware timeout semantics can be layered on top without breaking reason-code stability.

### 5) True resume / retry behavior

Status: ✅ Implemented for the agreed Phase 5 slice

Implemented:

- Interrupted PTY-backed transfers can resume on the same durable `transferId`.
- Resume metadata is persisted on cc-server:
  - `attempt`
  - `maxAttempts`
  - `resumeSupported`
  - `retryScheduled`
  - `resumeToken`
  - `resumeOffsetBytes`
  - `tempPathHint`
- Resume requests validate ownership, transfer direction, token/path/offset state, and only proceed when the partial artifact state remains trustworthy.
- Admin UI exposes operator-triggered resume for resumable transfers.
- Resumed transfers emit truthful progress again instead of appearing frozen until completion.
- Automatic retry remains limited to the current minimal slice:
  - `maxAttempts = 2`
  - one automatic retry for retryable start/pre-transfer failures
  - no automatic retry after partial-transfer progress is observed

Notes / remaining follow-up:

- Current resumable execution is intentionally scoped to the implemented PTY-backed SCP flows and current partial-artifact validation model.
- Future enhancements may still extend resume coverage or add more advanced retry heuristics, but they are no longer required to call the current senior-review item complete.

## Summary

All items from the current Phase 5 senior-review slice are now implemented or aligned for the agreed scope:

1. **Quotas / byte budgets** — done for the current default-tenant slice
2. **Server-side rate limiting / saturation nack** — done
3. **SLO tracking / monitoring** — done for the current lightweight summary/export slice
4. **Chunk/stall timeout contract** — done for the current practical Phase 5 contract
5. **True resume / retry behavior** — done for the implemented PTY-backed resumable transfer slice

Future work, if desired, should be tracked as new enhancements rather than as unresolved senior-review gaps for Phase 5.
