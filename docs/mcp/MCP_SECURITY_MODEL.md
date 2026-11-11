# SuperControlPanel MCP Security Model

## Slice 3 standard endpoint

All existing standard tools/resources use Slice 1 authorization, validation, backup gate, safe DTOs and output checks through the shared desktop host. Discovery and execution check current policy; sessions confer no authority. Sensitive operations require local `allow-with-token` and their category. Protected config imports and Git remain capability-blocked, including direct calls to hidden names. MCP/CC authority and local launch configuration remain protected. All HTTP routes require bearer auth and reject browser Origin and invalid Host/port. Operational errors omit runtime diagnostics. See [Slice 3 verification](DESKTOP_STREAMABLE_HTTP_SLICE3.md). Stream revocation, worker bounds and cancellation qualification remain Slice 4.

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

## Current Slice 1 boundary

The [Slice 1 contract and evidence](DESKTOP_STREAMABLE_HTTP_SLICE1.md) is authoritative for the shipped legacy MCP paths. The registry and schemas enforce policy before host dispatch. MCP/CC authority and local launch settings are protected; unsafe sync imports and git actions are unavailable remotely. Discovery is filtered by the same policy as calls and resources. Unknown policy/category values fail closed. Explicit empty categories grant no access.

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

All launches, stops, config/server writes, remote-browser open/close/mkdir/delete, transfers, and shared-config push are sensitive. Browser list/stat and transfer/status reads are read-only within their own categories. MCP callers cannot change this classification or their policy.

## Auditability

Log MCP activity with:

- timestamp
- tool name
- caller/session id if available
- no argument values (including nested values or apparently harmless fields)
- result status

Current implementation status:

- MCP HTTP tool/resource calls log only a fixed call kind and a registry-resolved operation name to stderr; unknown names are replaced with `unknown`

## Safe defaults for responses

- Return concise summaries, not raw secrets
- Include warnings when an operation is risky
- Prefer explicit errors over silent failures

Current implementation status:

- dedicated public config/settings DTOs omit keyring secrets and MCP/CC tokens
- status messages, raw transfer diagnostics/paths, and command result text are suppressed remotely
- schema checks and recursive secret-field redaction guard public results; runtime/parser errors become safe classified messages
- HTTP transport errors return a standardized JSON envelope with `ok`, `message`, `data`, `warnings`, and `timestamp`

## Future hardening ideas

- per-tool allowlists
- expiring tokens
- separate read-only token mode
- rate limiting
- explicit client identity labeling
