# Phase 5 — File Operations (SCP + Remote Browser)

## Scope
- CC-driven `scp_upload`/`scp_download`; remote browser session control.

## Commands
- `scp_upload`, `scp_download`, `start_scp_upload`, `start_scp_download`.
- Browser: `open_remote_browser`, `list_remote_browser_children`, `stat_remote_browser_path`, `create_remote_browser_directory`, `delete_remote_browser_path`, `close_remote_browser`.

## Agent
- Reuse existing SCP + SFTP with persistent session per browser modal.
- PTY-backed transfers for progress bars.
- Normalize paths; confirm overwrite on uploads.

## CC / Admin UI
- Two-pane browser; transfer queue with progress + history.
- Boundary-backed SCP requires active tunnel.

## Acceptance Criteria
- Upload/download shows continuous progress and final status.
- Browser ops reflect remote FS changes within one refresh.

## Risks
- Large file memory pressure → stream + chunked progress.
- Stale tunnel detection.
