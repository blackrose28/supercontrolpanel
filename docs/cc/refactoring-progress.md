# Dispatcher Refactoring Progress Report

## Date: 2026-04-16

## Status: PARTIAL COMPLETION (Phase 1 - 50% Complete)

## Completed Tasks

### 1. ✅ Module Structure Created
- Created `/src-tauri/src/cc/dispatcher/` directory
- Created `/src-tauri/src/cc/dispatcher/handlers/` directory  
- Created `/src-tauri/src/cc/dispatcher/transfer/` directory

### 2. ✅ Message Types Extracted (message_types.rs - 280 lines)
Successfully extracted:
- `InboundServerMessage` enum (22 variants)
- `InboundServerMessageError` struct
- All payload structs:
  - `RestartServerPayload`
  - `OpenPtyPayload`, `WritePtyPayload`, `ResizePtyPayload`, `ClosePtyPayload`
  - `OpenRemoteBrowserPayload`, `BrowserSessionPayload`, `BrowserPathPayload`, `DeleteRemoteBrowserPathPayload`
  - `ScpUploadPayload`, `ScpDownloadPayload`

### 3. ✅ Parsers Extracted (parsers.rs - 340 lines)
Successfully extracted:
- `parse_inbound_server_message()` - main parser function
- All 15+ `parse_*_payload()` helper functions
- Generic `parse_payload<T>()` helper
- `parse_restart_server_payload()` with validation

### 4. ✅ Transfer Module Created
Successfully extracted transfer logic into 3 files:

#### transfer/helpers.rs (330 lines)
- `emit_transfer_status()`
- `is_retryable_transfer_reason()`
- `split_file_op_reason()`
- `spawn_transfer_watchdog()` - complex watchdog with timeout/stall detection
- `sample_transfer_progress()`, `sample_upload_progress()`, `sample_download_progress()`
- `sample_remote_file_size()` - SFTP + SSH fallback
- `shell_quote()`
- `SampledTransferProgress` struct
- Constants: `TRANSFER_HEARTBEAT_INTERVAL_SECS`, `TRANSFER_TOTAL_TIMEOUT_SECS`, etc.

#### transfer/blocking.rs (260 lines)
- `run_blocking_command_action()` - generic blocking action executor
- `run_blocking_transfer_command_action()` - blocking transfer with retry logic

#### transfer/pty_backed.rs (320 lines)
- `run_pty_backed_transfer_command_action()` - PTY-based transfer with watchdog
- Handles `StartScpResult::Started` and `StartScpResult::Skipped` cases
- Transfer session binding and watchdog spawning

#### transfer/mod.rs (7 lines)
- Module re-exports

**Total transfer module: ~920 lines extracted from original 1,100+ lines**

## Remaining Tasks

### 5. ⏳ Handlers Not Yet Extracted (~800 lines remaining)
The main `dispatch_inbound_server_message()` function (lines 680-1479) still needs to be split into handler modules:

#### handlers/status.rs (needed)
- RequestStatusSnapshot handler
- RequestInventory handler  
- RequestRecentLogs handler
- `send_status_snapshot()`, `send_inventory()`, `send_recent_logs()`
- `build_inventory_payload()`, `compute_inventory_hash()`

#### handlers/service_restart.rs (needed)
- RequestRestartOpenvpn handler
- RequestRestartBoundaryAuth handler
- RequestRestartServerTunnel handler
- RequestRestartServerSsh handler
- `spawn_restart_task()` function

#### handlers/pty.rs (needed)
- OpenPty handler
- WritePty handler
- ResizePty handler
- ClosePty handler
- `bind_remote_pty_session()`, `resolve_remote_pty_session()`, `unbind_remote_pty_session()`
- `resize_local_pty()`, `pty_error()`

#### handlers/remote_browser.rs (needed)
- OpenRemoteBrowser handler
- CloseRemoteBrowser handler
- ListRemoteBrowserChildren handler
- StatRemoteBrowserPath handler
- CreateRemoteBrowserDirectory handler
- DeleteRemoteBrowserPath handler

#### handlers/transfer.rs (needed)
- ScpUpload handler
- ScpDownload handler
- StartScpUpload handler
- StartScpDownload handler

### 6. ⏳ Protocol Functions Not Yet Extracted (~100 lines)
These are currently at the end of dispatcher.rs (lines 2398-2649):
- `send_ack()`
- `send_nack()`
- `send_ws_event()`
- `send_command_status()`
- `send_status_snapshot()`
- `send_inventory()`
- `send_recent_logs()`

**Note:** These might belong in `protocol.rs` instead of dispatcher module.

### 7. ⏳ Main mod.rs Not Created
Need to create `dispatcher/mod.rs` with:
- Module declarations
- Re-exports of public types and functions
- `CcDispatcher` struct
- High-level dispatch entry point

### 8. ⏳ Import Updates Not Done
Need to update imports in:
- `connection.rs` - uses `dispatch_inbound_server_message`, `parse_inbound_server_message`, etc.
- Any other files that import from `dispatcher`

### 9. ⏳ Testing Not Done
- Compilation test
- Functional testing of CC connection

### 10. ⏳ Original File Not Deleted
- `dispatcher.rs` still exists and is the active file

## File Size Comparison

### Before Refactoring
- `dispatcher.rs`: 2,696 lines (monolithic)

### After Refactoring (Projected)
```
dispatcher/
├── mod.rs                      ~100 lines (not created yet)
├── message_types.rs            280 lines ✅
├── parsers.rs                  340 lines ✅
├── handlers/
│   ├── mod.rs                  ~50 lines (not created)
│   ├── status.rs               ~200 lines (not created)
│   ├── service_restart.rs      ~250 lines (not created)
│   ├── pty.rs                  ~300 lines (not created)
│   ├── remote_browser.rs       ~250 lines (not created)
│   └── transfer.rs             ~200 lines (not created)
└── transfer/
    ├── mod.rs                  7 lines ✅
    ├── helpers.rs              330 lines ✅
    ├── blocking.rs             260 lines ✅
    └── pty_backed.rs           320 lines ✅
```

**Total: ~2,887 lines across 13 files (avg ~222 lines/file)**

## Blockers

1. **Large dispatch function**: The 800-line `dispatch_inbound_server_message()` function is complex and requires careful extraction to maintain all the async/await logic and error handling.

2. **Protocol functions location**: Need to decide if `send_ack()`, `send_nack()`, etc. should stay in dispatcher or move to `protocol.rs`.

3. **Testing complexity**: No automated tests exist for CC functionality, so testing requires manual connection to CC server.

## Next Steps

To complete Phase 1, we need to:

1. **Extract handlers** (highest priority)
   - Create handler modules
   - Move dispatch match arms into handler functions
   - Maintain all async/await and error handling

2. **Create dispatcher/mod.rs**
   - Re-export all public types
   - Create clean public API

3. **Update imports**
   - Fix connection.rs imports
   - Fix any other files using dispatcher

4. **Test compilation**
   - Run `cargo check`
   - Fix any compilation errors

5. **Delete original dispatcher.rs**
   - Only after successful compilation

## Estimated Remaining Effort

- Handlers extraction: 2-3 hours
- mod.rs creation: 30 minutes
- Import updates: 30 minutes
- Testing & debugging: 1-2 hours

**Total: 4-6 hours to complete Phase 1**

## Recommendation

Given the complexity, I recommend:

1. **Option A (Complete)**: Continue with full handler extraction to finish Phase 1 properly
2. **Option B (Pause)**: Document current progress, commit what we have as a WIP branch, and continue later
3. **Option C (Hybrid)**: Create a simplified mod.rs that re-exports from the original dispatcher.rs, allowing incremental migration

Which approach would you prefer?
