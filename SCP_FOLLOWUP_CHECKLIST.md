# SCP Browser Follow-up Checklist

## Goal

Finish the post-refactor cleanup and polish work after the SCP remote browser rollout.

## Phase H: Retire Remaining Legacy Browse Helper

- [ ] Decide whether `stat_remote_path` should be removed or kept as a narrow utility.
- [ ] If removed, route overwrite checks through the active remote browser session flow instead.
- [ ] Remove transitional docs if `stat_remote_path` is retired.

## Phase I: Browser UX Polish

- [ ] Review current tree/content behavior after real-world use.
- [x] Start the visible tree at the remote user home directory instead of `/`.
- [x] Keep tree expand/collapse as local UI state only.
- [x] Expand a tree folder when the user clicks it in the left tree.
- [ ] Improve keyboard navigation and focus behavior.
- [ ] Refine empty/loading/error states after testing feedback.

## Phase J: SCP Transfer Progress

- [x] Add a progress bar area to SCP transfer history items.
- [x] Display transfer percentage when available.
- [x] Move SCP upload/download actions onto PTY-backed execution so progress output can be observed.
- [ ] Verify progress parsing for:
  - [ ] upload file
  - [ ] upload folder
  - [ ] download file
  - [ ] download folder
- [ ] Confirm recursive folder transfers behave reasonably when progress resets per file.
- [ ] Decide whether to keep the old synchronous `scp_upload` / `scp_download` commands as transitional helpers.

## Notes

- On Windows, the remote browser now reuses one persistent OpenSSH-backed shell session for navigation while the browser modal is open.
- SCP transfers still use separate transfer processes from the browser session, but they now run through PTY-backed execution so the UI can observe progress text.
