# Control Plane Protocol (v1)

Envelope: `{ type, deviceId, seq, ts, payload }`

## Agent → CC (telemetry)
- `function-status`, `pty-output`, `pty-exit`, `openvpn-log`
- `heartbeat` (uptime, versions, platform)
- `inventory` (servers list hash)

## CC → Agent (control)
- `start_openvpn_function`, `stop_openvpn_function`
- `start_boundary_auth`, `stop_boundary_auth`
- `start_server_tunnel`, `stop_server_tunnel`
- `start_server_ssh`, `stop_server_ssh`
- PTY: `open_pty`, `write_pty`, `resize_pty`, `close_pty`
- SCP: `scp_upload`, `scp_download`
- Remote browser: `open_remote_browser`, `list_remote_browser_children`, `stat_remote_browser_path`, `create_remote_browser_directory`, `delete_remote_browser_path`, `close_remote_browser`

## Replies
- `ack` / `nack` with `reqSeq`, `status`, `error?`.

## WebSocket
- URL: `wss /agent`, subprotocol `scp.cc.v1`.
- Auth: Bearer `<agentToken>` or mTLS.
- Backpressure: small-frame batching (10–50 ms or 8–16 KB).

## Mapping to App Contracts
- Names mirror existing Tauri commands/events 1:1; agent dispatches to `SessionStore`/`FunctionStore` and forwards `function-status`/`pty-output`/`pty-exit`/`openvpn-log`.
