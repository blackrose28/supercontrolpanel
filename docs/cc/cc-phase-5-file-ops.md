# Phase 5 — File Operations (SCP + Remote Browser)

## Scope
- CC-driven `scp_upload`/`scp_download`; remote browser session control.
- Reliability, integrity, and safety guarantees for large and sensitive transfers.

## Commands
- `scp_upload`, `scp_download`, `start_scp_upload`, `start_scp_download`.
- Browser: `open_remote_browser`, `list_remote_browser_children`, `stat_remote_browser_path`, `create_remote_browser_directory`, `delete_remote_browser_path`, `close_remote_browser`.

## Agent
- Reuse existing SCP + SFTP with persistent session per browser modal.
- PTY-backed transfers for progress bars and live percentages.
- Normalize paths; confirm overwrite on uploads.

## Transfer reliability (resume/retry)
- Retries: exponential backoff with jitter, max attempts configurable (default 3), surfaces final error with `reason`.
- Resume: when both endpoints support it, implement chunked resume for uploads/downloads using SFTP `append` or restart from last verified chunk; otherwise restart whole file.
- Timeouts: per-chunk and total transfer timeouts; stalled transfers emit heartbeat progress events.

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
- Per-transfer rate limit (KB/s) optional; global per-device cap enforced by agent.
- Per-tenant quotas: daily transfer byte budget with soft warning at 80%, hard stop at 100% (server-enforced `nack rate_limited`).

## Boundary tunnel precheck
- For Boundary-backed servers, ensure the tunnel is running before starting `scp_*`; else return `failed: tunnel_not_running` with an action hint to start the tunnel.

## CC / Admin UI
- Two-pane browser; transfer queue with progress + history.
- Boundary-backed SCP requires active tunnel (precheck above).
- Transfer history shows running, success, and error entries with stable `reason` codes.

## Acceptance Criteria
- Upload/download shows continuous progress and final status; resume/retry works as designed.
- Atomic rename prevents partial files from appearing at destination.
- Checksum verification (when enabled) detects corruption deterministically.
- Browser ops reflect remote FS changes within one refresh.

## Risks
- Large file memory pressure → stream + chunked progress; avoid buffering whole files.
- Stale tunnel detection → explicit precheck and clear retry guidance.

## SLOs (Phase 5 File Ops)

## Test plan (Phase 5 File Ops)

## Test plan (Phase 5 File Ops)

- Resume/retry
  - Simulate network interruption mid-transfer (50% complete); verify resume from last verified chunk and final success.
  - Exhaust retry attempts (3); verify final `failed` status with stable `reason` code.
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
  - Exceed per-tenant daily quota; verify `nack rate_limited` at 100% threshold and soft warning at 80%.
  - Apply per-transfer rate limit; verify throughput capped and progress updates reflect throttling.
- Boundary tunnel precheck
  - Attempt SCP with tunnel stopped; verify `failed: tunnel_not_running` with action hint.
  - Start tunnel, retry; verify success.
- Remote browser session
  - Open browser, list/stat/create/delete; verify SFTP session persists across ops and closes on modal close.
- Progress and history
  - Transfer 50 MB file; verify progress updates ≥ 1 Hz and final history entry with success/error status.


## SLOs (Phase 5 File Ops)

- Transfer throughput: P50 ≥ 1 MB/s, P95 ≥ 500 KB/s (WAN, typical file sizes 1–100 MB)
- Progress update frequency: ≥ 1 Hz for transfers > 10 MB
- Resume/retry: successful resume within 3 attempts for transient network failures
- Checksum verification: 100% detection of corrupted transfers when enabled
- Atomic rename: 0% partial files visible at destination on success path
- Boundary tunnel precheck: 100% rejection with actionable error when tunnel not running

