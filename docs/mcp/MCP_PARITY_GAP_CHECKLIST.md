# SuperControlPanel MCP and Helper Parity Gap Checklist

## Purpose

Track the remaining gaps before:

- all practical app features are exposed through MCP
- the helper CLI reaches broad parity with the MCP surface

---

## MCP parity gaps

### Missing read-only/query tools

- [ ] `detect_openvpn_executable`
- [ ] `boundary_search_targets`

### Missing runtime/action tools

- [ ] `server_rdp_start`

### Missing lower-level or advanced control

- [ ] Decide whether raw PTY tools are needed
- [ ] If yes, add:
  - [ ] `pty_open`
  - [ ] `pty_write`
  - [ ] `pty_resize`
  - [ ] `pty_close`
  - [ ] `pty_read_snapshot`

### Missing transport/polish items

- [ ] Notifications for function status changes
- [ ] Notifications for SCP transfer changes
- [ ] Richer approval flow beyond current hard-block behavior
- [ ] Additional MCP transport/spec polish if desired

### Product decision gap

- [ ] Decide whether `server_rdp_start` should be exposed through MCP at all
  - launching an OS-native RDP client may or may not fit the desired MCP safety model

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

1. [ ] `boundary_search_targets`
2. [ ] `detect_openvpn_executable`
3. [ ] `server_rdp_start`

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
