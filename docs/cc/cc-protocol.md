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

## Versioning and Capabilities
- Protocol version: `scp.cc.v1` (WebSocket subprotocol).
- Feature negotiation: agent sends `hello{ agentVersion, supported: ["gzip", "chunking"] }`; server replies `welcome{ protocol: "scp.cc.v1", capabilities: { gzip: true, chunking: true } }`.
- Backward/forward compatibility: additive payload fields are tolerated; breaking changes bump protocol or `schemaVersion` per type.

## Idempotency and Delivery Semantics
- Idempotency: control commands carry `commandId` (UUID); duplicates must return the same result; the agent caches recent `commandId`s (TTL 10m).
- Telemetry ordering: guaranteed per `(deviceId, sessionId, type)` keyed by `seq`; cross-stream interleaving allowed.
- Acknowledgements: batching allowed (10–50 ms or 8–16 KB); server `ack{ reqSeq }` / `nack{ reqSeq, retryAfterMs }`.
- Compression: gzip permitted when frames > 8 KB if `welcome.capabilities.gzip=true`.
- Chunking: oversize events include `{ chunkIndex, totalChunks }` if `welcome.capabilities.chunking=true`.
