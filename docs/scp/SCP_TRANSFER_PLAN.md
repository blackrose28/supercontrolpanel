# SCP Transfer UX Plan

## Goal

Improve the Servers page SCP workflow so users can choose remote sources and destinations without typing raw paths, and support copying folders in addition to single files.

## Current Issues

- [ ] Upload requires a local file picker plus a manual remote path prompt.
- [ ] Download requires a manual remote path prompt plus a local save dialog.
- [ ] There is no remote folder tree or path browser.
- [ ] There is no folder upload or folder download support.
- [ ] Result feedback is limited to a single status line.
- [ ] Some UI copy is outdated about direct SSH vs Boundary SCP support.

## Delivery Plan

### Phase 1: Remote Browse API

- [x] Add backend command to list a remote directory for a selected server.
- [x] Add backend command to inspect a remote path and tell whether it is a file, directory, or missing.
- [x] Optionally add backend command to resolve a useful starting path such as remote home.
- [x] Reuse the same server connection resolution logic used by SSH and SCP so direct SSH and Boundary both work.
- [x] Return explicit errors for auth failures, missing paths, and command execution failures.
- [x] Document the new command contract in `README.md` and `AGENTS.md` if the API changes.

### Phase 2: Remote Picker Modal

- [x] Replace raw remote path prompts with a modal in the Servers page.
- [x] Show current path, parent navigation, folder-first listing, and click-to-select behavior.
- [x] Keep a manual path input as fallback for edge cases.
- [ ] Support two modes:
  - [x] Pick upload destination directory
  - [x] Pick download source file or directory
- [x] Preserve the current selected server context.

### Phase 3: Wire Existing File Transfers To Picker

- [x] Update Upload flow to use the remote picker for destination selection.
- [x] Update Download flow to use the remote picker for source selection.
- [x] Keep the current single-file SCP behavior working while folder support is still pending.
- [x] Improve validation for empty or invalid path selections before invoking the backend.

### Phase 4: Add Folder SCP Support

- [x] Extend backend SCP commands to accept recursive transfer mode.
- [x] Map recursive transfers to `scp -r`.
- [x] Add frontend actions for:
  - [x] Upload file
  - [x] Upload folder
  - [x] Download file
  - [x] Download folder
- [x] Auto-detect or clearly prompt for destination semantics when the target path already exists.
- [ ] Ensure Boundary-based transfers still work through the tunnel path.

### Phase 5: Improve Transfer Feedback

- [x] Replace the single SCP result line with clearer per-transfer status.
- [x] Show running, success, and error states with source and destination summary.
- [x] Keep a short in-session transfer history for recent actions.
- [ ] Decide whether progress output is worth supporting cross-platform, or defer it.

### Phase 6: Validation, Copy, And Cleanup

- [x] Normalize common path edge cases such as trailing slashes.
- [x] Add overwrite confirmation or clearer overwrite behavior.
- [x] Tighten error messages for tunnel-not-running, auth, and path-not-found cases.
- [x] Update the Servers page copy to match actual SCP support.
- [x] Update `README.md` and `AGENTS.md` to reflect the final UX and command/event contracts.

## Recommended Execution Order

- [x] Phase 1: Remote Browse API
- [x] Phase 2: Remote Picker Modal
- [x] Phase 3: Wire existing file transfers to the picker
- [x] Phase 4: Add folder SCP support
- [x] Phase 5: Improve transfer feedback
- [x] Phase 6: Validation, copy, and cleanup

## Phase 1 Scope For Immediate Work

Status: implemented in backend; runtime verification against live direct SSH and Boundary targets is still pending.

- [x] Identify whether to implement remote browsing via `ssh` shell commands or another approach.
- [x] Define the exact Tauri command payloads and response shapes.
- [x] Implement backend remote listing and path inspection commands.
- [ ] Verify they work for both direct SSH and Boundary servers.
- [x] Expose stable error messages the frontend can surface directly.
