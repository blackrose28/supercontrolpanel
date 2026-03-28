# SuperControlPanel MCP Security Model

## Security goals

- Keep MCP access local by default
- Prevent accidental secret exposure
- Reduce risk of destructive remote actions
- Make AI-triggered operations observable and auditable

## Default posture

- MCP server is disabled by default
- Bind only to `127.0.0.1` by default
- Require an auth token
- Do not expose secrets from keyring
- Do not expose sensitive tools unless MCP is explicitly enabled
- Reuse the same MCP auth model for any future helper CLI that calls the app

## Secret handling

The MCP layer should never return these values by default:

- OpenVPN password
- OpenVPN OTP secret
- Boundary password
- Boundary OIDC password
- Boundary OIDC OTP secret
- Any keyring-backed secret

Config-reading tools should return redacted summaries.

## Tool sensitivity levels

Recommended levels:

- `read_only`
- `runtime_control`
- `write_config`
- `destructive`
- `secret_access`

## Confirmation policy

Support a policy that can require local UI approval for sensitive or destructive tools.

Current implementation status:

- when policy is `require-ui-approval`, sensitive MCP tools are rejected over HTTP with an explicit error
- switching the policy to `allow-with-token` allows those tools to run with bearer-token auth alone

Recommended candidates:

- `openvpn_start`
- `openvpn_stop`
- `boundary_auth_start`
- `server_delete`
- `remote_browser_delete_path`
- `scp_upload`
- `config_sync_push`
- `config_git_push`

## Auditability

Log MCP activity with:

- timestamp
- tool name
- caller/session id if available
- arguments summary with redaction
- result status

Current implementation status:

- MCP HTTP tool/resource calls are logged to stderr with redacted argument summaries

## Safe defaults for responses

- Return concise summaries, not raw secrets
- Include warnings when an operation is risky
- Prefer explicit errors over silent failures

Current implementation status:

- config-related MCP outputs redact keyring-backed secrets
- sync-settings MCP outputs redact the MCP auth token
- HTTP transport errors return a standardized JSON envelope with `ok`, `message`, `data`, `warnings`, and `timestamp`

## Future hardening ideas

- per-tool allowlists
- expiring tokens
- separate read-only token mode
- rate limiting
- explicit client identity labeling
