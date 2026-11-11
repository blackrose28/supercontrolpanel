# Desktop HTTP MCP Slice 6: Config and qualification

Date: 2026-11-11. Config UX is implemented. Linux native qualification and the executed checks below define the tested release surface; Windows/macOS and untested product clients remain explicit release gates. This is a local static-bearer deployment, without OAuth discovery or general remote-client interoperability claims.

## Config behavior

The Config page displays and copies the complete standard endpoint, including `/mcp` and bracketed IPv6 addresses. The runtime status displays the running listener's address independently of the editable settings. Enable/disable, bind, port, categories, policy and token still save through the existing partial-patch command. Failed saves are visible; the local settings cache updates only after successful persistence. Reload Config to recover saved values after a rejected edit.

Tokens use a password input and a separate copy action. Examples contain shell variable references, never the token. Regeneration saves immediately; listener replacement revokes old connections. Clipboard success/failure messages contain no copied values. Runtime/save/retry failures show local corrective guidance without raw diagnostics. Refresh status reads the existing runtime command; Retry listener reapplies saved settings through `mcp_sync_runtime` (it does not force replacement of an already healthy, unchanged listener).

The stored `require-ui-approval` value is unchanged, but its label now says **Block sensitive actions** and explains that no approval queue exists. `allow-with-token` permits launches/mutations only within the enabled categories/capabilities. Protocol sessions and client names do not establish separate principals.

Inspector examples select modern `2026-07-28` or legacy `2025-11-25`. They target **Inspector CLI 2.14.0**, the real client tested here. No Claude Desktop, ChatGPT, Cursor, VS Code or other product-specific configuration format is advertised. Browser Inspector UI is not qualified: the default listener rejects browser Origin, and tokens must be supplied in request headers, never URLs. Windows native shells need their own environment-variable/line-continuation syntax; the displayed example is explicitly POSIX.

## Tested setup

1. Run the desktop app. In Config, enable MCP and retain the default read-only category and blocked sensitive policy for a status query.
2. Copy the full endpoint. Copy the token separately and set `SCP_MCP_URL` and `SCP_MCP_TOKEN` in the calling shell. Keep token values out of shell history/config shared with others. The UI's example deliberately omits their assignments.
3. With Node/npm available, execute the copied example:

```sh
npx --yes @modelcontextprotocol/inspector@2.14.0 --cli \
  --server-url "$SCP_MCP_URL" --transport http \
  --protocol-era modern --header "Authorization: Bearer $SCP_MCP_TOKEN" \
  --method tools/call --tool-name function_status_list --tool-args-json '{}'
```

Change only `--protocol-era legacy` for the tested initialization-based revision. `--method tools/list` performs discovery. These exact CLI options are exercised using the pinned package's launcher; `npx` resolves that same version. For a header-capable native client, provide `Authorization: Bearer <token>` on all requests and use the full `/mcp` URL. A client that cannot set bearer headers is unsupported by this deployment; no URL-token or OAuth workaround is provided.

The helper continues using the **base URL without `/mcp`**, its current local settings discovery and the six REST envelopes. Keep compatibility for at least one stable release containing the standard endpoint and a documented migration path; removal needs separate review, consumer tests and release notes. No automatic removal deadline or helper rewrite is introduced.

## Evidence and reproduction

UI RED: the new production-page tests failed on the missing endpoint, missing policy explanation and missing runtime refresh control before implementation. GREEN: `tests/mcpConfig.spec.ts` loads the actual page/controller and substitutes only the Tauri/clipboard boundaries. It covers endpoint/example/token copies, masked credentials, both example eras, IPv6, enable/disable, partial policy/category/token patches, runtime inactive/error/retry states failed-save redaction, invalid port/empty-token rejection before IPC and clipboard failures. These are Chromium browser tests, not native WebKit automation.

```sh
npm ci --ignore-scripts --prefix experiments/mcp-http-slice0
npm run build
cargo build --manifest-path src-tauri/Cargo.toml --bin super-control-panel --bin supercontrolpanelhelper
cargo test --manifest-path src-tauri/Cargo.toml
npx playwright test
npx playwright test --config playwright.mcp.config.ts
npx playwright test --config playwright.backup.config.ts
xvfb-run -a dbus-run-session -- node experiments/mcp-http-slice0/tests/native-smoke.mjs
```

The opt-in native smoke creates temporary XDG config/data/cache paths, a separate display and D-Bus session, and an ephemeral loopback port. It starts the actual compiled Tauri application, checks its visible window, and runs Inspector subprocesses against the desktop-owned listener. It persists inert `example.invalid` server metadata through the real desktop host, checks the file/resource, restarts with a new token and narrower policy, verifies the old token fails and a denied delete leaves the file unchanged, then reconnects for a safe query. It removes the temporary profile and stops only its own process. No live VPN, Boundary, SSH, SFTP, RDP, transfer, enrollment or keyring mutation is exercised.

The existing 131-test Rust suite supplies protocol/router, schema/policy, lifecycle/worker/event, settings/backup/recovery, parity-host and CC contract checks. The official-client integration uses real protocol routing with a synthetic operation host; it must not be mistaken for native session/elevation smoke. Both revisions and JSON/SSE modes are exercised separately there. See [Slice 0](DESKTOP_STREAMABLE_HTTP_SLICE0.md), [Slice 3](DESKTOP_STREAMABLE_HTTP_SLICE3.md), [Slice 4](DESKTOP_STREAMABLE_HTTP_SLICE4.md) and [Slice 5](DESKTOP_STREAMABLE_HTTP_SLICE5.md) for their detailed coverage and limits.

| Check | Observed result |
| --- | --- |
| Linux desktop + helper debug build | Pass, real Tauri executable with embedded built frontend; 18 existing Rust warnings, no build failure. No installer/release-package claim. |
| `cargo test` | **131 passed**, including both official-client JSON/SSE modes and compiled helper compatibility checks. |
| Frontend production build | Pass. |
| Default Playwright configuration | **36 passed, 27 skipped**. The 27 are the five backup and four MCP fixture-dependent tests in each of three browser projects; focused configurations below run them. |
| Config Playwright fixture | **4 passed** on Chromium, including the invalid-input/clipboard checks. |
| Backup Playwright fixture | **5 passed** on Chromium. |
| Linux native app + Inspector | **Both eras pass**: visible Tauri window, authenticated discovery/query, actual server upsert persisted to disk, resource read, fresh-client reconnect, application restart with rotated token, old-token 401, hidden mutation refusal and server `POLICY_DENIED` without file deletion. |
| Ripwire / diff review | Quality **gating=0**, unchanged edited signatures with no incompatible callers, `git diff --check` passes. Two nonblocking new-symbol error-masking findings are the intentional bounded readiness retry and sanitized Inspector error wrapper; the latter prevents execFile from printing bearer arguments. Static test discovery does not model the browser/IPC boundary; executed tests are the evidence. |

Tested versions: app **1.3.3**, Linux x86_64, Rust **1.99.1**, Node **22.23.4**, Playwright **1.59.1**, GTK **3.24.41**, WebKitGTK **2.52.6**; rmcp **3.9.0**, Axum **0.8.8**, Inspector **2.14.0**, legacy official SDK **1.32.2** and modern official client **2.4.0**. The legacy native SDK denial uses `2025-11-25`; modern is pinned to `2026-07-28`. Full wire negotiation and both response modes remain independently asserted by the Rust/Node integration tests.

Inspector's CLI validates discovered names before invoking a tool: under the blocked policy, it returns its own `tool_not_found` for `server_delete`. The first native smoke run exposed this limitation. The final smoke asserts this client behavior, then uses the official SDK to send the hidden operation and verifies the real host's safe `POLICY_DENIED`. It does not misreport the CLI's local refusal as server authorization evidence. A later qualification-harness cleanup failure from the isolated document-portal mount was corrected by unmounting that temporary mount before profile deletion; the complete rerun exits successfully.

Native output without tokens or temporary-path diagnostics is retained in [Slice 6 evidence](evidence/SLICE6_NATIVE.txt).

## Release gates still open

- Native Windows and macOS desktop builds/smoke. The isolated Slice 0 Windows transport cross-check is not a desktop build; no macOS SDK is installed here.
- Native interaction with all Config controls and system clipboard on each OS. Chromium IPC mocks plus the Linux native listener/window test supplement one another but do not establish that coverage.
- Native elevation, unlocked/locked/missing OS credential stores, real Boundary authentication, installed RDP handlers, network sessions and transfer ownership across real listener restart. Existing synthetic/error tests do not prove external-system behavior.
- Product clients beyond Inspector and full protocol conformance beyond the recorded fixtures. Published revisions are tested subsets, not a universal client compatibility promise.
- Protected sync/Git imports, safe `config_get`, raw PTYs, backup transport and credential reads remain deliberately excluded as recorded in the inventory/parity checklist. Operation deadlines and queue sizes retain Slice 4's documented limitations.

These gates prevent an unconditional cross-platform release sign-off. The final implementation slice can be reviewed independently of obtaining those additional hosts and credentials.
