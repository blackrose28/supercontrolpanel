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

- when policy is `require-ui-approval`, sensitive MCP tools are rejected by the shared dispatcher with an explicit error; no approval queue exists
- switching the policy to `allow-with-token` allows those tools to run with bearer-token auth alone

Explicit sensitive operations (independent of category and discovery `readOnly`):

- `openvpn_start`
- `openvpn_stop`
- `boundary_auth_start`
- `server_delete`
- `remote_browser_delete_path`
- `scp_upload`
- `scp_download`
- `scp_start_upload`
- `scp_start_download`
- `config_sync_push`
- `config_git_push`

## Local authorization controls

`sync_settings_save_partial` cannot change MCP/CC controls or enrollment state.
It accepts only the five path/folder fields documented in `MCP_TOOL_SCHEMAS.md`.
Unknown and disallowed fields (including null) reject the whole patch before any
save action. This guard applies to the shared desktop MCP dispatcher, including
HTTP and direct Tauri MCP calls, under either configured sensitive-action policy.
Desktop-local settings controls retain their full partial-update schema.

The guard prevents self-escalation through this patch tool; it does not make
arbitrary runtime/configuration tools safe for mutually untrusted callers. Shared
execution policy does not provide a sandbox or multi-principal isolation.

## Shared execution policy

`scp_core::mcp_policy::validate_mcp_tool_policy` is a pure fail-closed check.
The host supplies trusted local categories and policy; caller arguments never
supply this authority. The desktop loads settings once at the dispatcher gate,
before any action. HTTP calls, direct Tauri MCP calls and all five resource reads
that delegate to tools use this same gate. Settings-load errors still propagate.
Discovery endpoints retain their existing output and authentication behavior.

Only the exact 40 existing operation names are accepted, via an explicit execution
mapping independent of discovery. Policy must be exactly `require-ui-approval` or
`allow-with-token`. Categories must all be known exact strings: `read-only`,
`runtime-control`, `config-mutation`, `file-transfer`. One unknown category rejects
the whole list, even alongside a valid category for a read-only call. Duplicate
known categories are harmless. The operation's category must be enabled, including
for sensitive operations under `allow-with-token`. The explicit sensitive list
above is preserved; other mutating operations are not implicitly reclassified.

The pure validator denies empty category lists and rejects blank policies; it
performs no trimming/defaulting. Persistence models still preserve unknown and
empty values. The desktop loader continues its existing normalization of empty
category lists and blank policies to defaults before this gate. Nonempty unknown
values fail closed at execution. Policy errors have fixed text, with no supplied
operation/policy/category values or HTTP-specific context. HTTP error envelopes
and command/event payload shapes remain unchanged.

Validation alone does not implement handlers, enable CLI operations, authenticate
callers, enforce runtime ownership/capabilities or isolate callers sharing a token.
Hosts must supply those boundaries separately. The operation registry with schemas,
capabilities and handlers remains incomplete. Future discovery additions cannot
implicitly become authorized executions.

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
