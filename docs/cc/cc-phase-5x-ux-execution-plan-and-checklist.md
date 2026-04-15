# Phase 5 UX Execution Plan & Checklist

## Overview
This file tracks execution of the Phase 5 Admin UI UX hardening plan. Items are organized by component with checkboxes for tracking progress.

---

## P0 - Must Have

### 1. Top Bar Redesign

- [x] **1.1** Top bar with Logo, Devices button, Config button, Bell icon
- [x] **1.2** Bell icon with unread badge counter
- [x] **1.3** Alerts panel (slide-out dropdown)
  - [x] **1.3.1** Fetch alerts via polling (`GET /admin/v1/alerts`)
  - [x] **1.3.2** Display alert list with type, timestamp, device, message
  - [x] **1.3.3** Implement dismiss per alert
  - [x] **1.3.4** Implement "Clear all" button
- [x] **1.4** Persist read state in localStorage
- [x] **1.5** Status pill showing device health (connected count)
- [ ] **1.6** Navigation dropdown (not implemented - using button nav instead)
- [ ] **1.7** Config gear / User menu (not implemented)

### 2. Devices Page - Compact Table

- [x] **2.1** Create new compact devices table component
- [x] **2.2** Table columns: Status, Device, Last Seen, Functions, Actions
- [ ] **2.2b** Type column (not implemented - shows OS in Device name instead)
- [x] **2.3** Status dots (🟢 connected, 🟡 degraded, 🔴 disconnected)
- [x] **2.4** Function status dots (● running, ○ idle, ● error)
- [x] **2.5** Actions dropdown (View Details, Request Snapshot, View Logs, Config)
- [ ] **2.6** Implement sortable columns
- [x] **2.7** Add search/filter input
- [x] **2.8** Device detail opens on selection (existing inline view, not modal)

### 3. Function Status Cards - Compact

- [x] **3.1** Redesign status cards to compact format
  - [x] **3.1.1** Header: Function name + Status pill
  - [x] **3.1.2** Info row: message, observed timestamp
  - [x] **3.1.3** No actions on card (device-level actions exist)
- [x] **3.2** Status pill states (running/idle/error)
- [x] **3.3** Grid layout (auto-fit)
- [ ] **3.4** Responsive breakpoints (not implemented)
- [ ] **3.5** Click-to-navigate (not implemented)

### 4. Transfer Queue with Progress

- [x] **4.1** Transfer section exists (existing UI)
- [x] **4.2** Transfer states displayed (existing UI)
- [x] **4.3** Progress bar (existing UI)
- [x] **4.4** Throughput display (existing UI)
- [x] **4.5** ETA display (existing UI)
- [x] **4.6** Phase label (existing UI)
- [x] **4.7** SSE for transfers (existing)
- [x] **4.8** Queue position (N/A - single transfer)

### 5. Command History - Compact

- [x] **5.1** Command history table (existing UI)
- [x] **5.2** Columns: Time, Type, Status (existing UI)
- [x] **5.3** Expand shows details (existing UI)
- [x] **5.4** Pagination (existing UI)
- [x] **5.5** Filters (existing UI)
- [x] **5.6** Export (existing UI)
- [x] **5.7** Fetch from `GET /v1/commands` (existing)

---

## P1 - Should Have

### 6. Audit Trail Tab/Panel

- [x] **6.1** Audit trail section exists (existing UI)
- [x] **6.2** Audit trail table (existing UI)
- [x] **6.3** Columns: Time, Device, Type, Actor (existing UI)
- [x] **6.4** Expand on row (existing UI)
- [x] **6.5** Filters (existing UI)
- [x] **6.6** Export (existing UI)
- [x] **6.7** Audit tab on command selection (existing UI)
- [x] **6.8** Fetch from `GET /v1/commands/:id/audit` (existing)

### 7. Remote Browser Modal

- [x] **7.1** Remote browser modal (existing UI)
- [x] **7.2** Folder browsing implemented
- [x] **7.3** Contents with Name, Size, Modified (existing UI)
- [x] **7.4** Breadcrumb navigation (existing UI)
- [x] **7.5** Parent folder navigation (existing UI)
- [x] **7.6** Refresh button (existing UI)
- [x] **7.7** Create folder (existing UI)
- [x] **7.8** Delete selected (existing UI)
- [x] **7.9** Double-click to navigate (existing UI)
- [x] **7.10** Single-click select (existing UI)
- [x] **7.11** Transfer mode toggle (existing UI)
- [x] **7.12** Integrate with transfer (existing UI)

### 8. API Token Display + Copy

- [x] **8.1** API token section in Config (existing UI)
- [ ] **8.2** Display masked token (not implemented - shows full)
- [ ] **8.3** Click-to-reveal (not implemented)
- [x] **8.4** Copy button on token creation
- [x] **8.5** Success message on copy
- [x] **8.6** Regenerate Token button (existing UI)
- [x] **8.7** Revoke button (existing UI)
- [x] **8.8** Token metadata displayed

### 9. Telemetry Page

- [ ] **9.1** Create telemetry page accessible from top bar
- [ ] **9.2** Add time range selector
- [ ] **9.3** Add device filter dropdown
- [ ] **9.4** Add event type filter
- [ ] **9.5** Implement timeline visualization (chart)
- [ ] **9.6** Display recent events list
- [ ] **9.7** Add event row expansion for details
- [ ] **9.8** Add Export button (JSON/CSV)
- [ ] **9.9** Fetch from `GET /v1/telemetry` endpoint

---

## P2 - Nice to Have

### 10. PTY Terminal Redesign

- [ ] **10.1** Redesign terminal header bar
  - [ ] **10.1.1** Session type icon
  - [ ] **10.1.2** Connection info (user@host)
  - [ ] **10.1.3** Dimensions dropdown
  - [ ] **10.1.4** Copy all button
  - [ ] **10.1.5** Close button
- [ ] **10.2** Implement tab bar for multiple sessions
  - [ ] **10.2.1** Add tab for each session
  - [ ] **10.2.2** Drag to reorder
  - [ ] **10.2.3** Close button per tab
  - [ ] **10.2.4** Notification dot if backgrounded with output
- [ ] **10.3** Enhance xterm.js features
  - [ ] **10.3.1** Search (Ctrl+Shift+F)
  - [ ] **10.3.2** Font size controls
  - [ ] **10.3.3** Theme toggle (dark/light)

### 11. Transfer History Enhancement

- [ ] **11.1** Add transfer history table
- [ ] **11.2** Add filters (device, status, date range)
- [ ] **11.3** Add expand for full transfer details
- [ ] **11.4** Add transfer retry button for failed transfers

### 12. Event Timeline Visualization

- [ ] **12.1** Add timeline chart to telemetry page
- [ ] **12.2** Implement time range selection
- [ ] **12.3** Add event type filtering on chart
- [ ] **12.4** Add tooltips on hover

---

## API Endpoint Checklist

### Required Endpoints

- [x] `GET /admin/v1/alerts` - List current alerts (admin session only)
- [ ] `POST /admin/v1/alerts/dismiss` - Mark alert as read (not implemented)
- [x] `GET /v1/commands` - List commands (with pagination, filters)
- [x] `GET /v1/commands/:id` - Single command detail
- [x] `GET /v1/commands/:id/audit` - Audit trail for command
- [x] `GET /v1/audit` - Global audit trail
- [x] `GET /v1/file-transfers` - Transfer history
- [x] `GET /v1/file-transfers/stream` - Real-time transfer updates (SSE)
- [x] `POST /v1/file-ops/scp-upload` - Start upload transfer
- [x] `POST /v1/file-ops/scp-download` - Start download transfer
- [x] `GET /v1/device-summary` - Aggregated device summary
- [ ] `GET /v1/telemetry` - Telemetry events (not implemented)
- [ ] `GET /v1/config` - Get config (not implemented)
- [ ] `POST /v1/config` - Update config (not implemented)
- [x] `POST /admin/api-tokens` - Create API token (admin session only)
- [x] `POST /admin/api-tokens/:tokenId/revoke` - Revoke API token (admin session only)

---

## Testing Checklist

### Manual Testing

- [ ] Test on Chrome (latest)
- [ ] Test on Firefox (latest)
- [ ] Test on Safari (latest)
- [ ] Test on Edge (latest)
- [ ] Test responsive: Desktop (1920x1080)
- [ ] Test responsive: Desktop (1366x768)
- [ ] Test responsive: Tablet (768x1024)
- [ ] Test responsive: Mobile (375x667)
- [ ] Test keyboard navigation
- [ ] Test screen reader compatibility

### Automated Tests

- [ ] Unit tests for new components
- [ ] Integration tests for API endpoints
- [ ] E2E tests for critical flows

---

## Milestone Tracking

| Milestone | Target Date | Items | Status |
|-----------|------------|-------|--------|
| M1: Top Bar + Alerts | Done | 1.1-1.5 | [x] |
| M2: Devices Table | Done | 2.1-2.8 | [x] |
| M3: Status Cards | Done | 3.1-3.5 | [x] |
| M4: Transfer Queue | Existing | 4.1-4.8 | [x] |
| M5: Command History | Existing | 5.1-5.7 | [x] |
| M6: Audit Trail | Existing | 6.1-6.8 | [x] |
| M7: Remote Browser | Existing | 7.1-7.12 | [x] |
| M8: API Token | Done | 8.1-8.8 | [x] |
| M9: Telemetry | Not planned | 9.1-9.9 | [ ] |
| M10: PTY Redesign | Not planned | 10.1-10.3 | [ ] |

---

## Notes

- Each checkbox `[ ]` represents a discrete task
- Prefix PR titles with "UX:" for Phase 5 UX changes
- Update this checklist as tasks are completed
- Add PR links to items when merged