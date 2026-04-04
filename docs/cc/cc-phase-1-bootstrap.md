# Phase 1 — Bootstrap

## Goals
- Establish secure outbound WS connection from agent to CC.
- Stream  and  telemetry.
- Provide minimal Admin API + in-memory registry to view devices.

## Deliverables
- CC Server (default: NestJS)
  - WebSocket  (subprotocol , )
  - HTTP Admin: , 
  - In-memory stores for , 
  - Simple Admin page (server-rendered) listing devices + lastSeenAt
- Agent (this app)
  - New module  with , , 
  - Config keys in : , , , , 
  - Heartbeat every 15s: 
  - Forward  events to CC
  - Reconnect with jitter and exponential backoff
- Frontend
  - Config page: Control plane section (URL, token, enable toggle)
  - Status pill: CC
