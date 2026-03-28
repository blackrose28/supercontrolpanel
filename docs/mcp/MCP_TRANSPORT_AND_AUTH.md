# SuperControlPanel MCP Transport and Auth

## Goal

Provide an MCP connection model that works well when the SuperControlPanel desktop app is already running and an AI agent wants to call into that live app instance.

This same MCP endpoint should also be the automation surface used by future command-line helper tooling.

## Recommended transport

Use an embedded MCP server reachable from the local machine.

### Why this fits SuperControlPanel

- The app is already a long-running desktop process
- The backend already holds live runtime state in memory
- AI agents should be able to query and control that live state
- It avoids spawning a separate process just to proxy the desktop app

## Recommended defaults

- Bind address: `127.0.0.1`
- Configurable port
- Disabled by default
- One auth token per app instance or per configured profile

## Lifecycle

- Turning on the Config page toggle starts the MCP server
- Turning it off stops the MCP server cleanly
- App shutdown should also stop the MCP server cleanly

## Helper CLI strategy

Do not add a second custom IPC protocol for one-shot command-line automation.

Instead, a future `supercontrolpanelhelper` command-line tool should act as a thin MCP client.

### Example direction

- `supercontrolpanelhelper boundary login start --hidden`
- `supercontrolpanelhelper openvpn status --json`
- `supercontrolpanelhelper server tunnel start --server-id abc`

### Why this is preferred

- one automation protocol
- one auth model
- one tool surface
- less backend duplication
- easier testing and maintenance

## Auth approach

Use a bearer-style token or equivalent shared secret for initial implementation.

### Requirements

- Token generated locally
- Token stored in device-local config, not synced by default
- Token shown in Config page with regenerate action
- Token required for all MCP calls unless a future read-only anonymous mode is explicitly designed

## Client setup UX

The Config page should show a copyable snippet containing:

- transport type
- localhost URL
- auth token placement

## Future transport options

If a future need appears, additional local transports can be evaluated later, but they should not replace the main design:

- one running app instance
- one local MCP endpoint
- optional helper CLI layered on top of MCP
