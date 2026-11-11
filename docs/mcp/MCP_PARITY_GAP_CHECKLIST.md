# SuperControlPanel MCP and Helper Parity Gap Checklist

## Slice 3 status

The existing 40-tool catalog and five resources are adapted to standard `/mcp`, with three protected import/Git tools explicitly unavailable. Official-client tests cover all rows, JSON/SSE, both pinned revisions, schemas, safe errors and policy denial. Existing desktop action payloads/runtime ownership are preserved. Lifecycle/events, reviewed parity additions, Config UX and native/end-user-client qualification remain open. See [Slice 3 evidence](DESKTOP_STREAMABLE_HTTP_SLICE3.md).

## Slice 5 status

Reviewed MCP gaps are implemented: target search, OpenVPN detection, conditional sensitive RDP launch and write-only SSH credential set/clear. The catalog now has 45 operations (42 discoverable with all categories and allow-with-token), with six resources and Slice 4 snapshot invalidations. Config UX, intended-client/native qualification and helper subcommands remain open. See [Slice 5 policy, tests and limitations](DESKTOP_STREAMABLE_HTTP_SLICE5.md).

## Purpose

Track the remaining gaps before:

- all practical app features are exposed through MCP
- the helper CLI reaches broad parity with the MCP surface

---

## MCP parity gaps

### Missing read-only/query tools

- [x] `detect_openvpn_executable`
- [x] `boundary_search_targets`

### Missing runtime/action tools

- [x] `server_rdp_start`

### Reviewed SSH credentials

- [x] `server_ssh_password_set` / `server_ssh_password_clear` (write-only, sensitive config mutations)
- Credential reads remain excluded.

### Missing lower-level or advanced control

- [ ] Decide whether raw PTY tools are needed
- [ ] If yes, add:
  - [ ] `pty_open`
  - [ ] `pty_write`
  - [ ] `pty_resize`
  - [ ] `pty_close`
  - [ ] `pty_read_snapshot`

### Missing transport/polish items

- [x] Notifications for function status changes (snapshot invalidations)
- [x] Notifications for SCP transfer changes (snapshot invalidations)
- [ ] Richer approval flow beyond current hard-block behavior
- [ ] Additional MCP transport/spec polish if desired

### Product decision gap

- [x] Expose `server_rdp_start` as sensitive runtime control with interactive host/destination checks; local client required

---

## Helper CLI parity gaps

### OpenVPN

- [ ] `openvpn detect-executable`

### Boundary

- [ ] `boundary search-targets --filter ...`

### Servers

- [ ] `server rdp start --server-id <id>`
- [ ] `server upsert ...`
- [ ] `server delete --server-id <id>`

### Remote browser

- [ ] `remote-browser open --server-id <id>`
- [ ] `remote-browser close --browser-id <id>`
- [ ] `remote-browser list --browser-id <id> --path <path>`
- [ ] `remote-browser stat --browser-id <id> --path <path>`
- [ ] `remote-browser mkdir --browser-id <id> --path <path>`
- [ ] `remote-browser delete --browser-id <id> --path <path> [--recursive]`

### SCP

- [ ] `scp upload ...`
- [ ] `scp download ...`
- [ ] `scp start-upload ...`
- [ ] `scp start-download ...`
- [ ] `scp transfer list`
- [ ] `scp transfer get --transfer-id <id>`

### Config and sync

- [ ] `config save ...`
- [ ] `config patch ...`
- [ ] `config sync-settings get`
- [ ] `config sync-settings patch ...`
- [ ] `config git pull --repo-root <path>`
- [ ] `config git push --repo-root <path>`

### MCP helper/introspection utilities

- [ ] `mcp tools`
- [ ] `mcp resources`
- [ ] optional debug passthrough:
  - [ ] `mcp call --tool <name> --args <json>`

---

## Recommended implementation order

### Finish MCP parity first

1. [x] `boundary_search_targets`
2. [x] `detect_openvpn_executable`
3. [x] `server_rdp_start`

### Then improve helper CLI parity

4. [ ] helper support for the 3 MCP gaps above
5. [ ] helper support for SCP commands and transfer queries
6. [ ] helper support for remote browser commands
7. [ ] helper support for config mutation and git commands

---

## Current summary

### MCP

Already covered:

- app/status overview
- function status list/get
- OpenVPN start/stop/status
- Boundary auth start/stop/status
- server list/get/upsert/delete
- server tunnel start/stop
- server SSH start/stop
- Caddy start/stop
- remote browser lifecycle and file operations
- SCP direct and PTY-backed transfers
- SCP transfer history lookup
- config full/partial save
- sync settings get/partial save
- config sync pull/push
- config git pull/push

### Helper CLI

Currently covered:

- status
- function list/get
- openvpn status/start/stop
- boundary status/login start/login stop
- server list/get
- server tunnel start/stop
- server ssh start/stop
- config paths
- config sync pull/push
- mcp ping/info
