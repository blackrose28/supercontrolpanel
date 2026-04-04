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
