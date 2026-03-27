# SCP Remote Browser Refactor Plan

## Goal

Replace the current SCP picker with a real remote file browser that:

- uses a tree-like folder explorer instead of a flat per-directory table
- does not open a new SSH process on every navigation step
- remains compatible with both direct SSH servers and Boundary-backed servers
- becomes the long-term source picker and destination picker for SCP/SFTP-style workflows

## Why The Current Approach Must Be Replaced

- The current modal is a folder listing, not a tree explorer.
- Each folder navigation triggers a new backend browse command, which currently means another SSH command execution.
- The UX is slow and visually shallow for deep folder navigation.
- The architecture is stateless, so the frontend cannot build a smooth lazy tree without repeatedly reconnecting.
- Further polishing the current picker would be wasted work.

## Target Architecture

### Backend

- Add a persistent remote browse session per server while the browser modal is open.
- Keep the connection alive for repeated `list`, `stat`, and expansion requests.
- Prefer a proper SFTP-backed browse layer over shelling out to `ssh ls/stat` for every interaction.
- Cleanly close the session when:
  - the modal closes
  - the selected server changes
  - the underlying tunnel/session becomes invalid

### Frontend

- Replace the current modal table with a lazy tree explorer.
- Use expandable folders with per-node loading state.
- Show the current path as breadcrumbs.
- Support two selection modes:
  - destination folder mode
  - source file or folder mode
- Keep manual path fallback, but demote it to a secondary option.

## Non-Goals For This Refactor

- Do not add drag-and-drop yet.
- Do not add transfer byte progress yet.
- Do not migrate file transfer from SCP to SFTP in the same change unless the browse implementation makes that trivially safe.
- Do not attempt offline caching or persistent saved trees yet.

## Delivery Plan

### Phase A: Replace The Browse Backend Contract

- [ ] Define new backend browse-session types:
  - [ ] `RemoteBrowseSession`
  - [ ] `RemoteBrowseEntry`
  - [ ] `RemoteBrowseNodeInfo`
- [ ] Add commands to manage a persistent browse session:
  - [ ] `open_remote_browser({ serverId }) -> { browserId, rootPath }`
  - [ ] `close_remote_browser({ browserId }) -> void`
  - [ ] `list_remote_browser_children({ browserId, path }) -> RemoteBrowseEntry[]`
  - [ ] `stat_remote_browser_path({ browserId, path }) -> RemoteBrowseNodeInfo`
- [ ] Ensure the session abstraction works for:
  - [ ] direct SSH servers
  - [ ] Boundary servers through the active tunnel
- [ ] Decide the transport layer:
  - [ ] preferred: persistent SFTP/session-backed listing
  - [ ] fallback only if necessary: persistent shell process with command multiplexing
- [ ] Fail fast with explicit errors when:
  - [ ] Boundary tunnel is not running
  - [ ] auth fails
  - [ ] remote session drops
  - [ ] path lookup fails

### Phase B: Add Session Lifecycle And Safety Rules

- [ ] Tie remote browser session ownership to the currently open modal.
- [ ] Prevent multiple simultaneous browser sessions for the same modal state.
- [ ] Close stale browser sessions when:
  - [ ] modal closes
  - [ ] selected server changes
  - [ ] page navigation invalidates the current context
- [ ] Add defensive cleanup on app-side errors so leaked browser sessions do not accumulate.
- [ ] Confirm the implementation keeps locks short-lived and does not hold broad locks across I/O.

### Phase C: Build A Real Tree UI

- [ ] Replace the current SCP picker table with a two-part browser:
  - [ ] left: folder tree
  - [ ] right or center: contents of selected folder
- [ ] Add lazy-expand folder nodes.
- [ ] Add explicit loading states per folder node.
- [ ] Add breadcrumb navigation for the active folder.
- [ ] Preserve manual path entry as a fallback section, not the primary flow.
- [ ] Make the tree usable for deep navigation without losing state on each click.
- [ ] Ensure switching between folders does not clear already-loaded tree state unnecessarily.

### Phase D: Selection Model And UX Rules

- [ ] Define browser mode behavior precisely:
  - [ ] upload file: select destination folder only
  - [ ] upload folder: select destination folder only
  - [ ] download file: select remote file
  - [ ] download folder: select remote folder
- [ ] Add clear visual distinction between selected node and expanded node.
- [ ] Support double-click/open for folders and single-click selection.
- [ ] Keep a confirm/select action at the bottom of the modal.
- [ ] Prevent invalid selections by mode instead of letting invalid paths flow into transfer commands.
- [ ] Keep local file/folder dialogs separate from remote browser responsibilities.

### Phase E: Rewire Existing Transfer Flows

- [ ] Replace current picker integration with the new browser session API.
- [ ] Remove code paths that rely on repeated `get_remote_home` / `list_remote_path` round-trips for each navigation step.
- [ ] Keep SCP transfer commands unchanged where possible, except for any path normalization adjustments needed by the new browser.
- [ ] Ensure the new browser still feeds:
  - [ ] upload file
  - [ ] upload folder
  - [ ] download file
  - [ ] download folder

### Phase F: Cleanup And Retirement Of Old Picker

- [ ] Remove obsolete frontend state and handlers for the current flat picker.
- [ ] Remove obsolete backend commands if they are no longer needed:
  - [ ] `get_remote_home`
  - [ ] `list_remote_path`
  - [ ] `stat_remote_path`
- [ ] If any old commands are kept temporarily, mark them as transitional in docs.
- [ ] Update all user-facing copy to refer to the remote browser/tree instead of the remote picker.

### Phase G: Verification

- [ ] Test direct SSH browsing through multiple nested folders.
- [ ] Test Boundary browsing with tunnel already running.
- [ ] Test clear failure mode when Boundary tunnel is not running.
- [ ] Test file selection in download-file mode.
- [ ] Test folder selection in download-folder mode.
- [ ] Test destination-folder selection for upload-file and upload-folder flows.
- [ ] Test repeated navigation without connection churn symptoms.
- [ ] Test modal close/reopen to confirm browser sessions are released and recreated cleanly.

## Recommended Execution Order

- [ ] Phase A: Replace the browse backend contract
- [ ] Phase B: Add session lifecycle and safety rules
- [ ] Phase C: Build a real tree UI
- [ ] Phase D: Selection model and UX rules
- [ ] Phase E: Rewire existing transfer flows
- [ ] Phase F: Cleanup and retirement of old picker
- [ ] Phase G: Verification

## Implementation Notes

- Prefer replacing the current picker architecture rather than layering the tree on top of it.
- The key technical requirement is persistence of the remote browse connection while the modal is open.
- If SFTP integration turns out to be significantly cleaner than persistent `ssh` command multiplexing, choose SFTP even if it means modest backend refactoring.
- The frontend should treat remote folders as lazy nodes, not preloaded trees.
- Avoid rebuilding the whole tree on each expansion.

## First Slice To Implement

Start with Phase A only:

- [ ] choose the backend browse transport
- [ ] define the new Tauri command shapes
- [ ] implement persistent browser session open/close
- [ ] implement child listing on an existing browser session
- [ ] verify one browser session can serve repeated folder navigation requests without reconnecting each time
