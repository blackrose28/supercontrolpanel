# Phase 5 — Admin UI UX Hardening

## Overview
Phase 5 UX hardening focuses on improving the Admin UI professional polish, information density, and operational clarity. The Admin UI serves as the primary operational dashboard for managing devices, functions, file transfers, and remote browsers.

## Top Bar Redesign

### Current State
- Top bar likely contains multiple scattered icons/links for various functions
- Alerts/notifications appear inline or in multiple locations

### Target State
```
┌─────────────────────────────────────────────────────────────────────────────┐
│ [Logo] SuperControlPanel    [Devices▼] [Status] [Configs]    [🔔 3] [⚙️] [👤] │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Components:**
1. **Logo/Home** - Click returns to Devices page
2. **Navigation dropdown** - Devices (default), Boundary, OpenVPN, Servers
3. **Quick status pill** - Shows aggregate device health (e.g., "12 connected, 2 errors")
4. **Bell icon (alerts)** - Red badge with unread count, toggles alerts panel
5. **Config gear** - Opens Config page
6. **User menu** - Admin session info, logout

### Alerts Panel (Bell Icon)
- **Trigger**: Click bell icon to toggle panel
- **Content**: Scrollable list of recent alerts (max 50)
- **Alert types**:
  - Device disconnected
  - Function error state
  - Transfer failed
  - Tunnel not running
  - Quota warning/exceeded
- **Alert item format**:
  ```
  [🔴/🟡] <timestamp> · <device> · <message>
  ```
- **Actions per alert**:
  - Click → navigate to relevant context
  - Dismiss button (marks as read, removes from badge count)
  - "Clear all" button
- **Persistence**: Read state stored in localStorage, not server-side
- **Polling**: New alerts fetched via SSE or polling every 30s

## Devices Page

### Inventory Compact Display
Current: Likely verbose device cards with full metadata visible.

Target: Compact table view with smart density.

```
┌──────┬──────────────┬─────────┬───────────┬──────────┬────────────────┐
│ Status│ Device Name  │ Type    │ Last Seen │ Functions│ Actions       │
├──────┼──────────────┼─────────┼───────────┼──────────┼────────────────┤
│ 🟢   │ laptop-dev   │ linux   │ 2m ago    │ ●●○○     │ [View] [Logs]  │
│ 🟡   │ workstation  │ windows │ 15m ago   │ ●●○○     │ [View] [Logs]  │
│ 🔴   │ pi-server    │ linux   │ 2h ago    │ ○○○○     │ [View] [Logs]  │
└──────┴──────────────┴─────────┴───────────┴──────────┴────────────────┘
```

**Columns:**
- **Status**: 🟢 connected (green), 🟡 degraded (yellow - some functions errored), 🔴 disconnected (red)
- **Device Name**: Clickable, navigates to device detail
- **Type**: OS icon + name (linux/windows/macos)
- **Last Seen**: Relative time ("2m ago", "1h ago", "2d ago")
- **Functions**: Compact status dots (● running, ○ idle, ● error). Format: `<openvpn> <boundary> <tunnel> <ssh>`
- **Actions**: Dropdown with View Details, Request Snapshot, View Logs

### Device Detail Modal/Page
- **Header**: Device name, status, last seen, uptime
- **Tabs**: Status | Functions | Terminal | Transfers | Browser | Logs | Audit
- **Status tab**: Aggregated function states, quick actions
- **Functions tab**: Per-function cards (same as current but more compact)
- **Terminal tab**: Embedded xterm.js for remote PTY
- **Transfers tab**: Active and recent transfers for this device
- **Browser tab**: Remote browser if active session
- **Logs tab**: Recent logs, filterable
- **Audit tab**: Command history

## Status Cards (Function Status)

### Current State
Likely verbose cards showing full function details.

### Target State: Compact Cards
```
┌─────────────────────────────────────────────────┐
│ ● OpenVPN              [●] Connected             │
│ ─────────────────────────────────────────────── │
│ Server: vpn.corp.com · 2h 34m · 45.2 MB         │
│ [Restart] [Logs] [Disconnect]                   │
└─────────────────────────────────────────────────┘
```

**Card Anatomy:**
- **Header row**: State dot + Function name + Status pill
- **Info row**: Key metrics (server, duration, data transferred)
- **Actions**: Contextual buttons (max 3, grouped)

**Status Pill States:**
- Running: Green pill with duration
- Idle: Gray pill
- Error: Red pill with error message (truncated)
- Starting/Stopping: Animated yellow pill

### Grid Layout
- 3-column grid on desktop
- 2-column on tablet
- 1-column on mobile
- Collapse to accordion on narrow screens

## Command History

### Compact View (Default)
```
┌──────────────────────────────────────────────────────────────┐
│ 📋 Command History                                  [Export] │
├──────────────────────────────────────────────────────────────┤
│ 14:32:15  start_server_tunnel      ● succeeded   server-abc │
│ 14:31:42  request_status_snapshot  ● succeeded   -          │
│ 14:30:18  restart_openvpn          ● succeeded   -          │
│ 14:28:55  scp_upload               ● failed      quota_excd │
│ 14:25:12  open_remote_browser      ● succeeded   server-xyz │
└──────────────────────────────────────────────────────────────┘
```

**Columns:**
- Timestamp (HH:MM:SS or relative)
- Command name (truncated at 30 chars)
- Status dot (● succeeded, ○ failed, ◐ running)
- Status text (succeeded/failed/running)
- Target (serverId or deviceId, clickable)

### Expanded View (Toggle)
Clicking a row expands to show:
- Full command name
- Command ID (UUID)
- Full payload (JSON, syntax highlighted)
- Timing (submitted, started, finished, duration)
- Actor (admin session or API token)
- Audit trail for this command

### Implementation
- **Storage**: Server stores in `commands` table, admin UI fetches via `GET /v1/commands`
- **Pagination**: Infinite scroll, load 50 at a time
- **Filters**: By device, command type, status, date range
- **Export**: CSV/JSON download

## Audit Trail

### Current State
Likely mixed with command history or hidden in logs.

### Target State: Dedicated Tab/Panel

**Access:**
1. Top bar icon (shield icon) - toggles slide-out panel
2. Device detail → Audit tab

**Display:**
```
┌──────────────────────────────────────────────────────────────┐
│ 🛡️ Audit Trail                                    [Filter] [Export] │
├──────────────────────────────────────────────────────────────┤
│ 14:32:15  device-1    command:start_server_tunnel  admin:s1  │
│ 14:32:14  device-1    pty:opened                        system  │
│ 14:31:42  device-2    command:request_status_snapshot admin:s1 │
│ 14:30:18  device-1    command:restart_openvpn        admin:s1 │
└──────────────────────────────────────────────────────────────┘
```

**Columns:**
- Timestamp
- Device ID
- Event type
- Actor (admin session ID, API token ID, or "system")
- Details (expandable)

**Storage:**
- Server stores in `command_audit` and `pty_audit` tables
- UI fetches via `GET /v1/audit` or `GET /v1/devices/:id/audit`
- Retention: Configurable (default 30 days)

## Transfer Queue & History (Phase 5)

### Location
- Dedicated "Transfers" tab in device detail
- Also accessible via top bar dropdown

### Transfer Queue Panel
```
┌─────────────────────────────────────────────────────────────┐
│ 📂 Transfers                                    [+ Upload] │
├─────────────────────────────────────────────────────────────┤
│ ● Uploading...  45% ████████░░░░  2.3 MB/s  ETA 1:23      │
│ ○ Downloading   queued    -         -         ETA -        │
│ ● Uploading...  completed 34.2 MB in 12s    ✓             │
└─────────────────────────────────────────────────────────────┘
```

**Queue Item States:**
- **Queued**: Gray, shows position in queue
- **Starting**: Yellow, brief state
- **Running**: Green, shows progress bar, percent, speed, ETA
- **Succeeded**: Green checkmark, shows final stats
- **Failed**: Red X, shows reason code (clickable for details)
- **Canceled**: Gray strikethrough

**Progress Bar Details:**
- Smooth animation
- Shows bytes transferred / total
- Throughput (KB/s or MB/s)
- ETA (estimated time remaining)
- Phase label (precheck / transfer / checksum / finalize)

### Transfer History
Similar to command history but for file transfers:
- Columns: Time, Direction, Source → Dest, Size, Status, Reason
- Expandable for full details
- Filterable by device, status, date range

### Upload/Download Modal
```
┌─────────────────────────────────────────────────────────────┐
│ 📤 Upload to server-abc                           [X]       │
├─────────────────────────────────────────────────────────────┤
│ Direction:  Upload ○  Download ○                              │
│ Server:     [server-abc ▼]                                   │
│ Local:      [______________] [Browse...]                     │
│ Remote:     [______________] [Browse...]                     │
│                                                          [▼] │
│ Options:                                                      │
│   ☑ Overwrite  (○ prompt  ○ force  ○ skip)                   │
│   ☑ Recursive folder copy                                     │
│   ☐ Verify checksum (SHA256)                                 │
│   ☐ Preserve timestamps                                       │
│   ☐ Preserve permissions                                      │
│   Rate limit: [________] KB/s (optional)                     │
│                                                            ─ │
│                              [Cancel]        [Start Transfer]│
└─────────────────────────────────────────────────────────────┘
```

**Remote Browser Integration:**
- "Browse..." buttons open remote browser modal
- Two-pane file browser (local/remote)
- Path input with autocomplete
- Shows validation errors inline

## Remote Browser (Phase 5)

### Modal Layout
```
┌─────────────────────────────────────────────────────────────┐
│ 🌐 Browse: server-abc                      [X]               │
├─────────────────────────────────────────────────────────────┤
│ Path: [/home/user/]  [../] [↻] [📁+🗑️]                      │
├───────────────────────┬─────────────────────────────────────┤
│ Name          │ Size  │ Modified                           │
│ ├─ documents   │ -     │ 2024-01-15 14:30                  │
│ ├─ downloads   │ -     │ 2024-02-20 09:15                  │
│ ├─ project.zip │ 45 MB │ 2024-03-01 16:45                  │
│ └─ notes.txt   │ 4 KB  │ 2024-03-10 11:20                  │
├───────────────────────┴─────────────────────────────────────┤
│ [Select]  Mode: ○ Upload  ○ Download                         │
└─────────────────────────────────────────────────────────────┘
```

**Features:**
- Lazy-loading folder tree (click to expand)
- Current folder contents list
- Breadcrumb navigation (clickable)
- Parent folder (..) button
- Refresh button
- Create folder button
- Delete selected button (with confirmation)
- File/folder icons with type indicators
- Size column (folders show "-")
- Modified timestamp
- Double-click folder to navigate
- Single-click to select
- Transfer mode toggle (upload/download)

## Interactive PTY Redesign

### Terminal Panel
```
┌─────────────────────────────────────────────────────────────┐
│ ▶ SSH - server-abc (user@host)        [1024x768 ▼] [📋] [×] │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│ user@host:~$ _                                             │
│                                                             │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Header Bar:**
- Session type icon (SSH, Shell, SFTP progress)
- Connection info (user@host or function name)
- Dimensions dropdown (presets: 80x24, 1024x768, custom)
- Copy all button
- Close button

**Terminal Features:**
- xterm.js with ligatures support
- Search (Ctrl+Shift+F)
- Scrollback (10,000 lines default)
- Clickable URLs
- Copy selection (Ctrl+Shift+C)
- Paste (Ctrl+Shift+V)
- Font size controls
- Theme toggle (dark/light)

**Multiple Terminals:**
- Tab bar for multiple sessions
- Drag to reorder tabs
- Close button per tab
- Notification dot on tab if output while backgrounded

## Config Page

### API Token Display
```
┌─────────────────────────────────────────────────────────────┐
│ ⚙️ Configuration                                             │
├─────────────────────────────────────────────────────────────┤
│ Admin API Token:                                            │
│ ┌─────────────────────────────────────────┐ [📋 Copy]      │
│ │ 7f3a9b2c-... │ Status: Active │ Expires: Never │        │
│ └─────────────────────────────────────────┘                 │
│                                                              │
│ [Regenerate Token]  [Revoke]                                │
│ ⚠️ Regenerating will invalidate the current token           │
└─────────────────────────────────────────────────────────────┘
```

**Features:**
- Token masked by default (show last 4 chars: `****7f3a`)
- Click to reveal full token (session timeout 30s)
- Copy button copies to clipboard with success toast
- Regenerate creates new token, invalidates old
- Shows creation date, expiration (if any), last used

### Settings Sections
- **Connection**: CC server URL, timeout settings
- **Notifications**: Alert preferences, thresholds
- **Quota**: Daily budget display (admin only)
- **Security**: Session timeout, IP allowlist
- **About**: Version info, license

## Telemetry Page

### Access
Top bar icon (chart icon) - toggles slide-out or navigates to page.

### Content
```
┌─────────────────────────────────────────────────────────────┐
│ 📊 Telemetry                                        [Export] │
├─────────────────────────────────────────────────────────────┤
│ Time Range: [Last 1h ▼]  Device: [All ▼]  Type: [All ▼]     │
├─────────────────────────────────────────────────────────────┤
│ Timeline (Events over time)                                 │
│ ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄ │
│                                                              │
│ Recent Events                                                │
│ ─────────────────────────────────────────────────────────── │
│ 14:32:15  device-1  function-status   OpenVPN → running      │
│ 14:31:42  device-1  pty-output       ...                    │
│ 14:30:18  device-2  heartbeat        uptime: 2h 34m       │
│ 14:28:55  device-1  transfer_progress 45% 2.3 MB/s         │
└─────────────────────────────────────────────────────────────┘
```

**Event Types:**
- function-status
- pty-output (sampled, not every line)
- pty-exit
- transfer_progress
- transfer_status
- heartbeat
- command_status

**Features:**
- Timeline visualization
- Event filtering (device, type, time range)
- Event detail expansion
- Export to JSON/CSV

## Responsive Breakpoints

| Breakpoint | Width | Layout Changes |
|------------|-------|----------------|
| Desktop    | ≥1200px | 3-column grids, full sidebars |
| Tablet     | 768-1199px | 2-column grids, collapsible sidebar |
| Mobile     | <768px | 1-column, bottom nav, stacked cards |

## Implementation Priorities

### P0 - Must Have
1. Top bar alerts panel (bell icon)
2. Compact devices table
3. Compact function status cards
4. Transfer queue with progress
5. Command history (compact)

### P1 - Should Have
1. Audit trail tab/panel
2. Remote browser modal
3. API token display + copy
4. Telemetry page

### P2 - Nice to Have
1. PTY redesign (tabs, header bar)
2. Responsive breakpoints
3. Transfer history filtering
4. Event timeline visualization

## API Endpoints Required

### New/Enhanced
- `GET /v1/alerts` - Fetch current alerts
- `POST /v1/alerts/:id/dismiss` - Mark alert as read
- `GET /v1/commands` - List with pagination, filters
- `GET /v1/commands/:id` - Single command detail
- `GET /v1/commands/:id/audit` - Audit trail for command
- `GET /v1/audit` - Global audit trail
- `GET /v1/file-transfers` - Transfer history
- `GET /v1/file-transfers/stream` - Real-time transfer updates
- `POST /v1/file-ops/transfers` - Start transfer
- `GET /v1/telemetry` - Telemetry events
- `GET /v1/config` - Current config (non-sensitive)
- `POST /v1/config` - Update config

## Component Library

Recommended UI components (if using a framework):
- **Table**: TanStack Table (headless, flexible)
- **Dialogs**: Radix UI Dialog
- **Dropdowns**: Radix UI Dropdown
- **Toast**: Sonner or React Hot Toast
- **Icons**: Lucide React
- **Forms**: React Hook Form + Zod
- **Date/Time**: date-fns

If building from scratch:
- Follow existing CSS patterns in the app
- Use CSS custom properties for theming
- Ensure keyboard navigation
- Focus management for modals
