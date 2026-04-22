# CC Folder Refactoring Plan

## Summary
Found 3 files that are too large and need refactoring:

| File | Lines | Functions | Issue |
|------|-------|-----------|-------|
| `dispatcher.rs` | 2,696 | 36 | Massive dispatch logic + transfer handling + PTY management |
| `telemetry_queue_disk.rs` | 1,294 | 40 | Complex disk queue operations mixed together |
| `connection.rs` | 1,022 | 14 | WebSocket connection + multiple background tasks |

## 1. dispatcher.rs (2,696 lines) - CRITICAL

### Current Structure
- **Lines 1-75**: Imports, constants, CcDispatcher struct
- **Lines 77-236**: InboundServerMessage enum (30+ variants)
- **Lines 237-648**: Payload parsing functions (15+ parsers)
- **Lines 649-1479**: Main dispatch_inbound_server_message (800 lines!)
- **Lines 1480-2603**: Transfer execution helpers (1,100+ lines)
- **Lines 2604-2696**: Inventory building

### Problems
1. **Monolithic dispatch function** (680-1479): 800 lines of nested match arms
2. **Transfer logic sprawl** (1480-2603): Multiple transfer strategies mixed
3. **Too many responsibilities**: parsing, dispatching, PTY, transfers, inventory

### Recommended Split

```
src-tauri/src/cc/dispatcher/
├── mod.rs                      (100 lines) - Re-exports, CcDispatcher, dispatch entry
├── message_types.rs            (200 lines) - InboundServerMessage enum, error types, payload structs
├── parsers.rs                  (400 lines) - All parse_*_payload functions
├── handlers/
│   ├── mod.rs                  (50 lines)  - Handler re-exports
│   ├── status.rs               (150 lines) - Status/inventory/logs handlers
│   ├── service_restart.rs      (200 lines) - Service restart handlers
│   ├── pty.rs                  (300 lines) - PTY handlers
│   ├── remote_browser.rs       (250 lines) - Remote browser handlers
│   └── transfer.rs             (150 lines) - Transfer coordination handlers
└── transfer/
    ├── mod.rs                  (100 lines) - Transfer re-exports, common types
    ├── blocking.rs             (300 lines) - Blocking transfer strategy
    ├── pty_backed.rs           (500 lines) - PTY-based transfer strategy
    └── helpers.rs              (200 lines) - Transfer utilities
```

**Total: ~2,700 lines across 13 files (avg ~200 lines/file)**

## 2. telemetry_queue_disk.rs (1,294 lines) - HIGH PRIORITY

### Current Structure
- **Lines 1-83**: Imports, constants, types
- **Lines 84-526**: Queue layout and manifest operations
- **Lines 527-1294**: Record I/O operations (append, read, ack, compact)

### Problems
1. **Mixed concerns**: Layout management + I/O + compaction
2. **Long functions**: Some functions exceed 100 lines
3. **Crypto integration scattered throughout**

### Recommended Split

```
src-tauri/src/cc/telemetry_queue_disk/
├── mod.rs                      (100 lines) - Re-exports, public API
├── types.rs                    (100 lines) - All queue-related types
├── layout.rs                   (250 lines) - Layout, manifest, cursor management
├── writer.rs                   (400 lines) - Append, rotation, pressure handling
├── reader.rs                   (350 lines) - Read, ack, cursor management
└── compaction.rs               (200 lines) - Compaction, cleanup, recovery
```

**Total: ~1,400 lines across 6 files (avg ~230 lines/file)**

## 3. connection.rs (1,022 lines) - MEDIUM PRIORITY

### Current Structure
- **Lines 1-46**: Imports, constants, CcConnection struct
- **Lines 47-897**: Connection lifecycle + message handling
- **Lines 898-1022**: Background tasks (inventory, PTY GC)

### Problems
1. **Multiple background tasks** in one file
2. **Long connect() method** (probably 200+ lines)
3. **Mixed concerns**: connection + inventory + PTY cleanup

### Recommended Split

```
src-tauri/src/cc/connection/
├── mod.rs                      (150 lines) - CcConnection, connect_with_retry
├── types.rs                    (100 lines) - Connection-specific types
├── message_handler.rs          (400 lines) - Message receive loop, dispatching
└── background_tasks.rs         (300 lines) - Inventory publisher, PTY GC
```

**Total: ~950 lines across 4 files (avg ~240 lines/file)**

## Implementation Priority

### Phase 1: dispatcher.rs (CRITICAL)
This file is the biggest problem. Start here.

**Order:**
1. Extract types → `message_types.rs`
2. Extract parsers → `parsers.rs`
3. Extract transfer helpers → `transfer/helpers.rs`
4. Extract transfer strategies → `transfer/blocking.rs`, `transfer/pty_backed.rs`
5. Extract handlers → `handlers/*.rs`
6. Create clean `mod.rs`

**Estimated effort**: 1-2 days

### Phase 2: telemetry_queue_disk.rs (HIGH)
Second biggest file with clear separation points.

**Order:**
1. Extract types → `types.rs`
2. Extract layout → `layout.rs`
3. Extract writer → `writer.rs`
4. Extract reader → `reader.rs`
5. Extract compaction → `compaction.rs`
6. Create clean `mod.rs`

**Estimated effort**: 1 day

### Phase 3: connection.rs (MEDIUM)
Smallest of the three, but still needs work.

**Order:**
1. Extract types → `types.rs`
2. Extract background tasks → `background_tasks.rs`
3. Extract message handler → `message_handler.rs`
4. Clean up `mod.rs`

**Estimated effort**: 0.5 day

## Benefits After Refactoring

1. **Easier to navigate**: Each file has a single, clear purpose
2. **Easier to test**: Smaller units can be tested independently
3. **Easier to modify**: Changes are localized to specific modules
4. **Better compile times**: Smaller files = faster incremental compilation
5. **Clearer ownership**: Each module has clear responsibilities
6. **Reduced merge conflicts**: Changes less likely to overlap

## Risks

1. **Breaking changes**: Need to update all imports
2. **Testing burden**: Must verify all functionality still works
3. **Time investment**: ~2-3 days of careful refactoring
4. **Potential bugs**: Moving code can introduce subtle issues

## Recommendation

**Start with dispatcher.rs** - it's the most problematic and will give the biggest benefit. Use the following approach:

1. Create new module structure
2. Copy code to new files (don't delete original yet)
3. Fix imports and visibility
4. Test thoroughly
5. Delete original monolithic file
6. Commit with detailed message

Each phase should be a separate PR for easier review.

## Testing Strategy

After each refactoring phase:

1. **Compile check**: Ensure the code compiles without errors
2. **Unit tests**: Run existing unit tests (if any)
3. **Integration tests**: Test the full CC connection flow
4. **Manual testing**: 
   - Connect to CC server
   - Open PTY sessions
   - Transfer files (upload/download)
   - Restart services
   - Browse remote files
5. **Performance check**: Ensure no performance regressions

## Migration Checklist

For each file being refactored:

- [ ] Create new module directory structure
- [ ] Copy code to new files with proper visibility (pub/pub(crate))
- [ ] Update imports in new files
- [ ] Create mod.rs with re-exports
- [ ] Update imports in files that use the refactored module
- [ ] Compile and fix any errors
- [ ] Run tests
- [ ] Manual testing
- [ ] Delete original monolithic file
- [ ] Final compile and test
- [ ] Commit with detailed message
- [ ] Create PR for review

---

**Document created**: 2026-04-16
**Status**: Planning phase
**Next action**: Begin Phase 1 (dispatcher.rs refactoring)
