# Desktop HTTP MCP Slice 5: reviewed parity additions

Date: 2026-11-11. Slice 5 adds five operations to the shared registry, standard `/mcp`, legacy tool routes and local Tauri MCP adapter. The catalog contains 45 entries, at most 42 discoverable with all categories and `allow-with-token`; protected sync import/Git remain unavailable. Six resource URIs, helper commands, settings, SDK pins, command/event names and desktop runtime ownership are unchanged. Config UX and real-client/native qualification remain Slice 6.

## Contracts and policy review

| Operation | Policy / capability / ownership | Behavior |
| --- | --- | --- |
| `boundary_search_targets` | read-only / Desktop / query | Optional filter matches name, ID and description case-insensitively, preserving desktop search and App/CLI authentication. Optional limit 1–100, default 100. Safe metadata has exactly id/name/targetType/description/address, including empty strings for absent metadata. |
| `detect_openvpn_executable` | read-only / Desktop / query | Returns `{path}` from the shared platform detector. Missing yields `NOT_FOUND`; locator/process/filesystem failure yields `OPERATION_FAILED`. No OpenVPN process is launched. |
| `server_rdp_start` | sensitive runtime-control / InteractiveDesktop / desktop-owned launch | Requires serverId, supported Windows/macOS/Linux host, an interactive desktop and a Windows destination. Hostname/IP validation rejects whitespace, URL parameters, userinfo and option syntax. Reuses mstsc/open/xdg-open through the existing desktop action. Returns `{}` after launch; no claim of connected RDP or managed stop/status. |
| `server_ssh_password_set` | sensitive config-mutation / Desktop / mutation | Requires serverId and nonempty password (at most 4096 bytes, schema writeOnly). Existing action verifies the server, writes its existing OS-keyring account and enters the backup gate. Returns `{}`. |
| `server_ssh_password_clear` | sensitive config-mutation / Desktop / mutation | Requires serverId. Reuses existing server lookup/keyring delete action. Returns `{}`; the keyring's established missing-entry behavior is retained. |

Both sensitive-action policy and categories apply on every call. `require-ui-approval` blocks RDP and credential changes before the host executes; it does not implement an approval queue. Discovery filters unavailable/denied operations through the shared registry. RDP's platform capability is checked during authorization; invocation checks destination and desktop availability. On Linux, nonempty DISPLAY or WAYLAND_DISPLAY indicates a candidate interactive desktop; it does not guarantee the display/client is usable. Windows/macOS calls occur in the running GUI app. An installed RDP client/URL handler is still required; launcher failures are safe operational errors with local setup guidance in the tool description/docs.

All input objects reject unknown fields, wrong types, NUL and oversized values. IDs are 1–256 bytes. Standard results use the existing `{result: ...}` wrapper and structured/text output schemas; legacy routes retain their envelopes. Passwords are never returned, credential reads are absent, audits retain names only, and untrusted host/keyring/CLI diagnostics are classified without echoing their text. Boundary auth/CLI failures return a safe `OPERATION_FAILED`, not empty successful results; inspect local Boundary authentication and CLI configuration. No new authentication-error subtype is claimed.

The public target DTO projects only reviewed metadata; it never serializes CLI output wholesale. Result count is bounded after the existing desktop search. The Boundary CLI still collects its full response internally, and target metadata strings are not individually truncated. Request/body/worker limits and query deadlines remain those of Slice 4. A timed-out blocking CLI/keyring/launcher call is not forcibly terminated. Query local state before retrying a lost mutation response.

## Reuse and compatibility

Boundary CLI parsing/filtering is extracted unchanged into a tested helper used by the desktop command and MCP. No alternate authentication or target-search engine is introduced. OpenVPN detection retains Windows common-location-first and Unix PATH-first order and the same candidate locations. The shared checked variant distinguishes locator exit 1 (absent), abnormal exit/spawn failures and invalid output; it tries common locations before reporting a failed locator. The existing Option-returning helper retains its public type. Common-location probes now require a file and surface filesystem errors through the checked path. Missing locator programs still allow common-location fallback. This checks detection, not whether the binary can successfully establish a VPN.

RDP and keyring operations delegate to existing desktop actions rather than introducing new processes/stores with separate ownership. The MCP-only RDP validation does not alter direct desktop controls. Listener cancellation/handoff, capacity, backup/recovery leases and safe result validation remain shared. Existing HTTP routes and helper discovery are retained; no helper subcommands are added.

The [inventory](DESKTOP_STREAMABLE_HTTP_INVENTORY.md) and [parity checklist](MCP_PARITY_GAP_CHECKLIST.md) record the selected additions. Safe `config_get` remains deferred after review because the desktop read contains secrets and was not selected for Slice 5. Raw PTY operations, backup transfer and credential reads remain excluded.

## TDD and qualification evidence

RED observations before implementation:

- The contract test failed because `boundary_search_targets` was absent from the registry.
- Four parity-host tests failed: dispatch returned unavailable and RDP preflight accepted invalid hosts.
- Two Boundary parser tests failed on empty results/malformed-output acceptance before the extracted implementation was connected.
- The detector-output test returned None instead of the expected path before its implementation.
- The official-client test failed the catalog fixture assertion (40 fixtures versus 45 operations) before the five fixtures were added.

GREEN coverage includes category/approval denial before dispatch, closed schemas and invalid bounds, bounded/default search results, exact public projection and filter forwarding, case-insensitive name/ID/description matching, malformed CLI output, Windows/Unix detector output and missing/error distinction, RDP supported/unsupported platforms and desktop/destination rejection, write-only password forwarding, missing-server/keyring/launcher failure sanitization, and absence of credential reads. The existing official SDK clients exercise every catalog entry under both 2025-11-25 and 2026-07-28, JSON-preferred and SSE, schema validation, successful results, safe host failures and policy narrowing. Transport/client hosts are synthetic; none of these tests authenticates to a live Boundary server, writes a native keyring or establishes RDP/VPN sessions.

Reproduce with `cargo test --manifest-path src-tauri/Cargo.toml`, `npm run build`, and `npx playwright test`, using Slice 3's pinned Node-client/helper prerequisites. Native Windows/macOS builds, Windows interactive-service detection, display/URL-handler availability and live keyring/auth workflows remain Slice 6 qualification work. No native interoperability claim is made from the Linux fixtures.

Final Linux verification: **131 Rust tests passed**, including both official-client modes; `npm run build` passed; Playwright **36 passed, 15 existing skips**. `git diff --check` passed. Ripwire quality delta reports **gating=0** without new acknowledgements; remaining nonblocking findings include static dead-code reports on executed tests/trait implementations, small catalog/dispatcher size increases and recent-edit churn. Change review and edit checks confirm unchanged existing dispatcher, Boundary command and Option-returning detector signatures. Static affected-test discovery remains incomplete; the full Rust suite and existing frontend/E2E checks supplied the runtime evidence.
