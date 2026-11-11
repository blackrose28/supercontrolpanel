# Desktop HTTP MCP Slice 3: existing surface migration

Date: 2026-11-11. The standard `/mcp` adapter now covers the existing 40-operation registry and all five fixed resources. It reuses the Slice 1 safe schemas, policy and desktop dispatch rather than implementing operation families a second time. The maximum discoverable set is 37 tools under `allow-with-token` with all categories enabled. `config_sync_pull`, `config_git_pull` and `config_git_push` retain the deliberate Slice 1 capability denial, remain hidden, and return `CAPABILITY_UNAVAILABLE` when called directly with valid arguments. Protected staging/Git review is still required before enabling them.

## Contracts

Discovery derives descriptors and schemas from the registry and filters the current host-authorized names. Calls and resource reads reenter `call_mcp_tool`, which reloads policy and holds the existing backup/recovery lease. Invalid inputs are rejected before desktop dispatch. Disabling a category or changing to `require-ui-approval` affects subsequent requests on an existing SDK connection; no protocol session gains authority. Resource authorization is identical to the equivalent tool.

The Slice 2 `function_status_list` and `scp://status/functions` result stays `{ "statuses": [...] }`. All other successful tool/resource results are `{ "result": <safe legacy data> }`. This uniform object wrapper supports arrays, null status snapshots and empty mutation results in standard structured content without changing desktop or REST payloads. Tools return matching JSON text and structured content, publish the corresponding object output schema, and validate host output before returning it. Resources use JSON text with `application/json`.

Unknown tools and invalid arguments produce SDK invalid-params errors. Operational errors produce tool `isError: true` with the existing safe application code/message; resource errors use SDK resource errors. Runtime diagnostics, malformed host output and keyring/remote error strings are not returned. Legacy envelopes and their transport warning remain confined to compatibility routes; there are no domain warnings in the current dispatcher to migrate.

Starts retain the current function/session IDs inside `result`; synchronous transfer results remain sanitized acknowledgements, and PTY-backed transfers return session IDs. Operations still execute on blocking workers through the same desktop actions and stores. No new runtime owner, automatic retry, stop-on-disconnect behavior, or change to duplicate-start/already-stopped semantics is introduced. Cancellation ownership, capacity bounds, stream revocation, start-response loss and listener-reconfigure stress tests remain Slice 4.

The listener's existing `transport: "http"`, `implementationStage: "http-streamable-slice2"`, Tauri commands/events, config keys and six REST helper routes remain unchanged. The stage string is retained for compatibility; it is not a per-family capability list. Query tool/resource discovery for currently permitted operations. Config UX changes and clearer client setup remain Slice 6.

## Per-family adapter evidence

Every existing-tool row in the [inventory](DESKTOP_STREAMABLE_HTTP_INVENTORY.md) links to the [official-client test](../../experiments/mcp-http-slice0/tests/desktop.test.mjs). Each named operation has an independent argument/result fixture in [desktop-surface.json](../../experiments/mcp-http-slice0/tests/desktop-surface.json). Fixtures are handwritten examples, not generated from the schema being checked. Both clients validate the published output schemas and compare returned values and text. The trusted test host asserts exact operation-specific arguments before returning the fixture through the real SDK router and shared policy/validator.

| Family | Cases | Checks |
| --- | --- | --- |
| Overview/status/inventory/paths | `app_get_overview`, `function_status_list/get`, `openvpn_get_status`, `boundary_get_status`, `servers_list`, `server_get`, `config_get_paths` | Safe objects, nonempty arrays, null snapshot, preserved status wrapper; five resources match tools |
| Runtime | OpenVPN/Boundary/tunnel/SSH/Caddy start and stop | Existing input names, stable function/session ID result shape, empty stop result, sensitive policy denial |
| Servers/config/local settings | `server_upsert/delete`, `config_save/save_partial`, `sync_settings_get/save_partial` | Nested typed inputs and safe outputs; unknown MCP/CC/launch fields rejected; partial arguments forwarded unchanged |
| Remote browser | Open/close/list/stat/mkdir/delete | Existing IDs/paths, directory listing, missing-path stat, empty mutation results; category enforcement |
| Transfers | Direct upload/download, PTY start upload/download, transfer list/get | Safe acknowledgement, session ID, safe transfer metadata and redacted source/destination fields |
| Sync/Git | `config_sync_push`; `config_sync_pull`, `config_git_pull/push` | Push adaptation; explicit capability denial for the three blocked operations |

All available operations also test unknown fields, missing/null required arguments, host errors containing an unlabelled secret, invalid secret-bearing host output, live category denial and discovery removal. Every resource tests equivalent success and safe operational/category failures. Live `require-ui-approval` keeps reads available while denying a previously visible start. Existing auth/Origin/Host/authority, body-size, version and compiled helper `function list` checks remain in the same test.

These are adapter acceptance tests with a synthetic host, supplemented by the existing policy/config/redaction/backup suites. They do not qualify native elevation, live VPN/Boundary, SSH/SFTP servers, OS keyrings, PTY scrollback or transfer completion. Native/action-specific acceptance targets in the inventory remain open where not already covered by existing tests. No Slice 5 parity additions are exposed.

## Reproduction and results

Dependencies/toolchain remain the Slice 2 pins: rmcp 3.9.0, Axum 0.8.8, Rust/Cargo 1.99.1 on Linux. Clients remain official legacy SDK 1.32.2 (2025-11-25) and modern client 2.4.0 (2026-07-28), using the existing experiment lockfile. No dependencies changed. Native Windows/macOS builds and end-user-client/Inspector requalification were not performed.

1. `npm ci --prefix experiments/mcp-http-slice0` if client dependencies are absent.
2. `cargo build --manifest-path src-tauri/Cargo.toml --bin supercontrolpanelhelper` if the compiled helper is absent.
3. `cargo test --manifest-path src-tauri/Cargo.toml mcp::protocol -- --nocapture` runs both revisions in both JSON-preferred and SSE transport modes. The Rust harness starts the real router and invokes Node from the desktop package directory.

TDD RED: the extended client discovery assertion failed with `1 !== 37` before adapter changes. GREEN: all 40 operation decisions and five resource mappings pass in both modes. After discovery refactoring, the full `cargo test --manifest-path src-tauri/Cargo.toml` suite passed: **114 tests**. `npm run build` passed. `npx playwright test` passed: **36 passed, 15 existing skips**. The final focused official-client rerun passed after fixture cleanup. `git diff --check` passed. Ripwire impact/uses/affected, edit-check and diff review were run; quality delta has **zero blocking regressions**, with no new acknowledgements. Its remaining new-code dead-code reports name the SDK callback/test functions that execute in the integration tests; static trait/macro/iterator reachability is incomplete. Host wiring reaches listener startup, settings saves and enrollment, with public command/event contracts unchanged.
