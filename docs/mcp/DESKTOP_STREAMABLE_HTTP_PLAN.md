# Desktop MCP over Streamable HTTP

Status: Slices 1–3 implemented: shared safe contracts/policy and SDK HTTP adapter for the existing catalog. Lifecycle/events, reviewed parity additions, Config UX and native application qualification remain open.

[Slice 3 implementation and verification](DESKTOP_STREAMABLE_HTTP_SLICE3.md).

[Slice 1 implementation, compatibility corrections, and TDD evidence](DESKTOP_STREAMABLE_HTTP_SLICE1.md).
Source assessment: 2026-11-11, repository baseline `e120a2bd6`.
Extracted from Work 2 in [the combined roadmap](SETTINGS_BACKUP_STREAMABLE_HTTP_STANDALONE_CLI_PLAN.md).

Slice 0 evidence at baseline `e9e843092`: [operation inventory](DESKTOP_STREAMABLE_HTTP_INVENTORY.md), [SDK/client decision and qualification report](DESKTOP_STREAMABLE_HTTP_SLICE0.md), and [reproducible fixture](../../experiments/mcp-http-slice0/README.md). The tested candidates are published `rmcp 3.9.0` and `axum 0.8.8`; production dependencies have not changed.

## Outcome and release boundary

An MCP client connects to the running desktop app at `http://127.0.0.1:46739/mcp`, authenticates with its device-local bearer token, discovers typed tools/resources, and controls the existing desktop-owned runtime through standard MCP. Use the configured loopback address/port when different; bracket IPv6 literals in URLs.

Work 2 owns the desktop HTTP host, standard protocol adapter, operation schemas/policy, interoperability, and the reviewed operation gaps below. It can ship without Work 3's standalone CLI, Cargo workspace, or complete Tauri-independent runtime. Work 1 backup release qualification is separate; this work must preserve its writer/recovery gate.

“Complete MCP” means interoperable tools/resources for the documented supported operations. Prompts, sampling, elicitation, task extensions, remote hosting/OAuth, raw PTY access, a functioning approval queue, and the deprecated 2024 HTTP+SSE transport are outside this release. Advertise only implemented and verified capabilities. stdio hosting belongs to Work 3; keep reusable schemas and policy independent of the HTTP host.

The desktop remains the runtime owner. Closing a client connection or restarting the MCP listener must not stop an accepted VPN, tunnel, SSH session, or transfer. Normal desktop shutdown continues to use the existing runtime cleanup paths.

## Baseline verified in source

| Surface | Current behavior | Required change |
| --- | --- | --- |
| `src-tauri/src/mcp/server.rs` | Blocking `TcpListener`/`TcpStream`, hand-written parser, six custom REST routes, bearer authentication | Maintained HTTP framework and SDK transport; compatibility routing on the same listener |
| `src-tauri/src/mcp/dto.rs` | Tool descriptors lack argument/result schemas; calls return `ok/message/data/warnings/timestamp` | Dedicated public DTOs and standard protocol results; retain legacy envelope adapter |
| `src-tauri/src/mcp/tools.rs` | 40 descriptors, string dispatch, Tauri action calls; sensitivity maintained separately | One operation registry, typed validation, central execution policy |
| Policy | Category checks occur during calls; only the exact `require-ui-approval` string blocks the sensitive subset | Reject unknown policies/categories; review every operation's sensitivity; prevent remote policy escalation |
| Redaction | App-password fields and `mcpAuthToken` are redacted by selected helpers | Explicit safe output DTOs; cover CC credentials, nested values, errors, and audit records |
| `src-tauri/src/mcp/resources.rs` | Five fixed URIs delegate reads to tools | Standard resource content with the same authorization as the equivalent operation |
| Config UI, `src/main.ts` | Generic HTTP snippet points at listener root | Tested client-specific `/mcp` examples, connection state, accurate approval-policy wording |
| `supercontrolpanelhelper` | HTTP client of custom REST endpoints; discovers local settings | Keep existing endpoint shapes and discovery working throughout migration |

Treat older catalogs/checklists as intent; verify every item against dispatch and tests before marking it covered. No code changes are authorized merely by this plan's proposed defaults.

## Protocol and dependency gate

Use the official Rust `rmcp` SDK with a maintained HTTP framework, preferably its supported integration. Pin published versions and lock dependencies after a bounded spike. Do not extend the custom parser or implement a second protocol engine to fill SDK gaps.

The Slice 0 fixture passes for both target revisions using official TypeScript clients and Inspector CLI 2.14.0. Keep SDK-managed **legacy sessions enabled**: the stateless legacy configuration failed server-side cancellation, while session mode passed. Modern requests remain sessionless through SDK routing. Legacy session responses use SSE even with JSON preferred; modern requests passed JSON and SSE. Inspector requires explicit `--protocol-era modern` to test the modern path. These results establish feasibility, not full application or native platform conformance.

The target remains revision **2026-07-28**, with **2025-11-25 compatibility** where supported by the selected SDK and required clients. These are different contracts:

| Concern | 2026-07-28 | 2025-11-25 |
| --- | --- | --- |
| Lifecycle | Per-request metadata and `server/discover` | `initialize` followed by `notifications/initialized` |
| HTTP | POST with JSON or request-scoped SSE; routing/version headers checked against body | Initialization, optional sessions, POST plus GET stream or 405; revision-specific DELETE behavior |
| Updates | `subscriptions/listen` when supported | Negotiated notifications/resource subscriptions |
| Cancellation | Request SSE closure cancels that request | Explicit protocol cancellation; disconnect alone is insufficient |

Implement these through the SDK. Modern requests do not use legacy protocol sessions. Unsupported versions, missing/mismatched metadata, invalid media types, and unsupported methods must receive the selected revision's prescribed errors. See [modern HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http), [versioning](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning), and [legacy transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).

Before implementation, record a compatibility table with exact SDK/framework versions and features, Rust minimum/toolchain, lockfile impact, supported revisions, client name/version, transport, OS, test command, and observed result. Use an official SDK client and at least one representative real client per claimed revision. Also demonstrate both a JSON response and an SSE response, cancellation, and authenticated requests.

If a revision cannot pass this gate, explicitly defer it and release only a verified revision. “Latest SDK” and support visible on upstream main are insufficient evidence. Recheck [official SDK releases](https://github.com/modelcontextprotocol/rust-sdk) at spike time. Do not publish a dual-revision claim or fixed delivery estimate before this gate passes.

## Implementation boundaries

Proposed flow:

```text
standard /mcp -> rmcp adapter -----------+
                                      +-> registry -> policy -> desktop operation host
legacy REST -> legacy envelope adapter -+                         -> existing services/stores
```

Keep the registry, safe DTOs, validation, policy types, and error classification free of HTTP and Tauri imports. The desktop host may initially retain `AppHandle` and call existing actions. A small internal module boundary is sufficient; separate crates can follow in Work 3. Do not duplicate PTY/function stores, spawn another runtime owner, or rewrite working operation families to introduce HTTP.

Each registry entry defines the stable operation name, title/description, category, input/output schema, sensitivity, required capability, lifetime, and host handler. Tools and resource mappings derive from this metadata; do not keep independent sensitivity/name lists. Not every desktop action must become an MCP operation.

Create an execution context in the trusted host containing the local principal, transport, policy snapshot/generation, cancellation handle, and audit correlation. Client names, JSON arguments, JSON-RPC IDs, and protocol-session IDs confer no authority. The single bearer token represents one shared principal; independent clients do not gain isolation by selecting different names or sessions.

Before editing existing symbols, run repository-required Ripwire impact/uses/affected checks and report callers, affected paths, and risk. Important integration points include `call_mcp_tool`, `enforce_tool_policy`, `read_mcp_resource`, `sync_mcp_runtime_from_settings`, the settings-save actions, Tauri MCP commands, and the helper client. Their exact blast radius remains to be measured before implementation.

## Operation inventory and parity decisions

The [Slice 0 inventory](DESKTOP_STREAMABLE_HTTP_INVENTORY.md) now records one row per current tool and desktop operation candidate, including desktop command/action, existing HTTP tool, resource mapping, proposed CLI mapping (or deferred), category, sensitivity, capability, runtime ownership, current argument/result shape, required acceptance test, and decision. Final public schemas and application acceptance tests remain Slice 1 and later work. Proposed CLI mappings are planning metadata, not Work 2 implementation tasks.

All 40 current tool descriptors must have a migration decision. Grouping below is a starting inventory, not permission to omit individual rows:

| Family | Coverage to preserve | Acceptance focus |
| --- | --- | --- |
| Overview/status | App overview, function list/get, VPN/Boundary status | Initial snapshot agrees with subsequent state; no secret-bearing terminal output |
| Servers | List/get/upsert/delete, tunnel and SSH start/stop | Stable server/function IDs; deleted-server filtering and per-server sessions |
| VPN/Boundary/Caddy | Current start/stop/auth behavior | Native elevation/auth capability, explicit unsupported errors, duplicate starts |
| Remote browser | Open/close/list/stat/mkdir/delete | Persistent SFTP session; lifecycle invalidation and deletion policy |
| SCP | Direct and PTY-backed upload/download, transfer list/get | Correct picker/path semantics, overwrite policy, terminal history, eventual outcome |
| Config/sync/git | Paths, config get/save/partial save, local settings get/patch, sync/git pull/push | Partial-patch semantics, writer gate, no self-escalation or credential disclosure |

Preserve these resource URIs: `scp://status/functions`, `scp://status/openvpn`, `scp://status/boundary`, `scp://servers/list`, `scp://config/paths`. Return standard text resource content with a documented JSON media type/schema. A resource read must not bypass a disabled tool category. Add templates only when an actual parameterized resource and tests justify them.

| Candidate gap | Proposed disposition | Required behavior before exposure |
| --- | --- | --- |
| `boundary_search_targets` | Include after schema/policy foundation | Match name, ID, description; bounded results and explicit authentication failures |
| `detect_openvpn_executable` | Include | Preserve platform detection; distinguish missing executable from other failures |
| `server_rdp_start` | Include only on supported interactive hosts after policy review | Explicit capability and sensitive launch classification; actionable unavailable response |
| SSH credential set/clear | Include through reviewed write-only operations | No password-read tool; no values echoed in results/logs; keyring failures surfaced |
| Safe config read | Review the newly recorded `config_get` candidate | Existing desktop `get_app_config` returns secrets; expose only a dedicated safe DTO |
| Raw PTY open/write/resize/close/snapshot | Defer | Requires a separate terminal-access and ownership design |
| Backup export/import | Keep desktop-local in this release | Secret archive/passphrase handling and restore authority need a separate MCP review |

Record deliberate exclusions and client-visible capability reasons. Reconcile this matrix with `MCP_PARITY_GAP_CHECKLIST.md` before release, including any newly found config/runtime gaps.

## Schemas, results, and failure contract

- Public DTOs describe only permitted inputs/outputs. Do not serialize secret-bearing runtime models and rely solely on a recursive field-name filter. Preserve current names and safe field semantics; record any necessary breaking changes explicitly.
- Reject malformed/missing/wrong-type arguments before dispatch; bound strings, arrays, pagination, and paths as appropriate. Mutations reject unknown fields to prevent silent misspellings and privilege-bearing additions. Model partial updates explicitly so omitted fields retain values.
- Publish input schemas and output schemas for structured results. Validate representative success and failure fixtures against schemas. Standard results carry `structuredContent` where supported and a useful text representation; keep legacy envelopes confined to REST.
- Separate transport failures, JSON-RPC errors, and operational failures. Malformed JSON/unknown methods/invalid parameters use SDK protocol errors. A valid tool invocation that fails policy, capability, keyring, or runtime checks returns a safe tool error (`isError: true`) with a stable application error code. Resources use the revision's standard resource errors rather than a tool envelope.
- Proposed application codes include `POLICY_DENIED`, `CAPABILITY_UNAVAILABLE`, `NOT_FOUND`, `CONFLICT`, `CANCELLED`, `TIMEOUT`, and `OPERATION_FAILED`. Final schemas must define retry guidance without echoing commands, passwords, OTPs, tokens, or remote output by default. Never automatically retry a side-effecting call.
- Preserve warnings intentionally; classify and redact them. Test redaction on successes, failures, resources, HTTP diagnostics, and audit records. Annotations such as read-only/destructive are client hints; execution policy remains authoritative.

## Authentication, authorization, and listener lifecycle

1. Keep HTTP disabled by default, loopback-only, and bearer-protected on every route, including health/status and compatibility routes. Never place the token in a URL. Retain device-local token storage and frontend command/event contracts.
2. Validate Host/authority against the configured listener authority, including valid localhost/IPv4/IPv6 forms and port. Reject untrusted origins with 403; absent Origin is valid for native clients. Default to no cross-origin browser access. If browser clients are later allowed, use an explicit origin allowlist and authenticated actual requests; no wildcard CORS.
3. Parse policy/categories as validated values. Unknown policy/category, missing operation metadata, or unavailable capability fails closed. Discovery filters disabled operations; blocked sensitive operations may remain discoverable only with clear policy information. Recheck authorization on every call/resource read, including existing connections.
4. Apply the same rules to standard HTTP, legacy REST, and resource-to-operation dispatch. A category allowlist alone does not make arbitrary config writes safe. Review mutation sensitivity explicitly instead of preserving the incomplete existing subset.
5. MCP callers cannot alter MCP enable/bind/port/token/policy/categories, activate or replace CC identity, or indirectly obtain these powers through config sync/git operations. Reject the entire forbidden mutation atomically; do not silently strip fields. Define protected-field comparison and stage/validate imported config before apply. Local desktop commands retain their existing authority and backup safeguards.
6. Token rotation revokes old access and closes associated streams/sessions; policy narrowing cancels subscriptions that are no longer allowed and prevents queued unauthorized work from starting. Accepted app-owned operations keep running. Reauthorize immediately before side effects without holding settings locks through long work.
7. Listener stop/reconfigure cancels transport tasks and drains them within a bounded deadline. Never hold runtime/store mutexes while awaiting I/O or thread completion. Preserve the backup settings gate and avoid deadlock between restore, settings save, listener shutdown, and operation completion.

Describe this as a local static-bearer deployment. Do not claim general remote OAuth/discovery interoperability. A real desktop approval queue remains deferred; `require-ui-approval` continues to reject sensitive calls with actionable local instructions.

## Operation lifetime, updates, and bounded execution

Start results distinguish accepted/running/completed and return the existing stable function/session/transfer identifier where available. A returned start result means the desktop owns the work. A lost start response means unknown outcome: query state before retrying. Request IDs are not durable deduplication keys.

Define duplicate-start and stop-already-stopped behavior per registry entry. Preserve established operation behavior where safe; add characterization tests before changing it. Request cancellation affects work still owned by the request. Once ownership transfers to the desktop, stopping requires the corresponding explicit stop operation. Document the precise handoff and test cancellation on both sides of it.

Use existing domain status/transfer updates through a bounded event bridge. Never block PTY readers on a slow HTTP consumer. Keep request progress attached to an in-flight request; completed start calls use subscriptions or status queries for later changes. Snapshot/polling is the required fallback when subscriptions are unsupported. On dropped/coalesced events, instruct clients to refresh a snapshot; do not imply durable replay without implementing it.

Proposed initial configurable internal limits, to validate in the spike: 1 MiB request body, 16 concurrent operation executions globally, 8 update streams, 128 queued updates per stream, 10-second header/body receive timeout, and 5-second transport shutdown drain. Separate ordinary-call deadlines from long transfer/auth operations and subscription streams; define explicit bounds for each operation before release. Reject excess work promptly; prioritize bounded memory over unbounded queues. These values are tuning proposals, not claims about current behavior.

Run blocking PTY, SSH/SFTP, filesystem, keyring, and existing blocking HTTP work on an appropriate blocking worker pool. Cancellation must be cooperative; dropping an async wrapper does not stop a blocking task. Bound workers and ensure a cancelled pending mutation cannot execute later unnoticed.

## Compatibility and migration

- Add standard MCP at `/mcp`. Retain `GET /health`, `/mcp/status`, `/mcp/tools`, `/mcp/resources` and `POST /mcp/tools/call`, `/mcp/resources/read` with their existing response shapes for the helper. Do not redirect legacy callers to JSON-RPC.
- Use one listener/router and shared operation execution. Keep legacy descriptors/envelopes as conversion types, not a second business-logic implementation. Security corrections apply to both routes and are documented as intentional behavior changes.
- Preserve device-local settings keys and existing runtime-status DTO fields consumed by desktop/helper/CC. Add fields only after checking consumers; record implementation-stage changes. The MCP spec revision is not a replacement for the app's existing transport/status field values.
- Update Config to display/copy the full endpoint and manage the token without logging it. Publish setup examples only for clients actually tested, with exact client versions and any bearer-header limitations. Generic JSON is not proof that a client accepts that configuration format.
- Keep REST through at least one stable release containing the standard endpoint and a documented helper migration path. Remove only in a separately reviewed change after helper/consumer tests and release notes; no automatic removal deadline. The HTTP helper remains useful for controlling the desktop even after Work 3 exists.

## Delivery slices and RED/GREEN gates

Every code increment follows repository strict TDD: write a behavioral test, run it and confirm the intended failure, implement minimally, rerun it, then refactor and run relevant existing tests. Documentation-only planning does not require runtime tests. These slices are independently reviewable; do not combine them into one extraction.

| Slice | Deliverable | Gate before moving on |
| --- | --- | --- |
| 0. Inventory and SDK spike | Per-operation matrix, protocol/client table, dependency decision, native build feasibility | Actual client traces and compile results; unsupported revisions explicitly deferred |
| 1. Policy and safe contracts | Registry foundation, DTOs, error classification, protected mutations, redaction | Failing-then-passing policy/escalation/redaction tests, including legacy paths |
| 2. First vertical protocol slice | SDK `/mcp`, read-only function status tool and matching resource | Official client discovery, schemas, success/errors, auth/Origin/Host, legacy helper still works |
| 3. Existing surface migration | All current tools/resources classified and adapted in small operation-family changes | Matrix rows link to passing tests; desktop payloads and runtime ownership unchanged |
| 4. Lifecycle and events | Cancellation, status/transfer event bridge, bounded workers/queues, listener reconfiguration | Disconnect, slow-client, rotation, concurrent calls, backup/recovery gate tests |
| 5. Reviewed parity additions | Target search, executable detection, approved RDP/credential operations | Capability, policy, platform/error tests for each addition |
| 6. Config UX and qualification | Tested setup examples, docs, real-client and native smoke evidence | Release checklist below; no unsupported interoperability claims |

Slice 3 implementation and verification: [DESKTOP_STREAMABLE_HTTP_SLICE3.md](DESKTOP_STREAMABLE_HTTP_SLICE3.md).

Slice 2 implementation and verification: [DESKTOP_STREAMABLE_HTTP_SLICE2.md](DESKTOP_STREAMABLE_HTTP_SLICE2.md).

Complete the policy foundation before enabling mutations through the standard endpoint. Slice 2 may land behind the existing disabled-by-default setting while later families are migrated; clearly expose the implemented subset.

## Acceptance tests and release checklist

- [ ] Every advertised revision passes its lifecycle/metadata, JSON/SSE, notifications, malformed input, unsupported-version, and cancellation fixtures using the pinned SDK client. Test revisions independently; no blended lifecycle assumptions.
- [ ] Every current tool/resource has a preserved or deliberately changed contract, schema, policy decision, and acceptance test. Forbidden config changes fail through both REST and MCP, including indirect sync/git imports.
- [ ] Missing/invalid bearer, untrusted Origin, Host/authority variants, IPv6 URL formatting, oversized/slow input, capacity limits, unknown policies/categories, and resource policy bypass are covered.
- [ ] Nested secrets, CC/MCP credentials, passwords, OTPs, keyring failures, audit arguments, warning/error text, and status/transfer output do not leak. Test both successful and failed requests.
- [ ] Accepted starts survive disconnect/cancellation and listener restart. Cover cancellation before acceptance, duplicate starts/stops, lost start responses, concurrent operations, token rotation, and policy narrowing.
- [ ] Slow subscribers cannot block PTY reads; overflow and snapshot recovery are tested. Transport shutdown terminates streams/workers within bounds without stopping desktop-owned sessions.
- [ ] Existing helper endpoint/token discovery and representative commands work against the compatibility router. Tauri command/event consumers, including CC/admin integration boundaries, retain their contracts.
- [ ] Playwright covers Config enable/disable, full endpoint, token handling, accurate blocked-policy presentation, runtime error/restart state, and client-example controls. Mocked IPC tests are supplemented by native desktop smoke tests.
- [ ] Backup restore/recovery and local settings patches still coordinate correctly with listener reconfiguration and concurrent calls. A restore cannot leave stale authorized streams active.
- [ ] Real-client smoke evidence records client/SDK versions, protocol, OS, bearer configuration, discovery, one safe query, one permitted mutation, one denial, and reconnect behavior. Windows/macOS/Linux desktop builds and native elevation/keyring limitations are recorded honestly.
- [ ] README and current transport/security/schema/catalog/parity/helper docs describe the shipped behavior and retained compatibility routes. Remove stale claims only when corresponding behavior lands.

Use `cargo test --manifest-path src-tauri/Cargo.toml`, `npx playwright test`, and `npm run build` as the current baseline, selecting focused tests during RED/GREEN. Add SDK client integration tests to the appropriate Rust test boundary. The root has no `npm test` script. Before completion/commit inspect the diff and run required Ripwire quality/change checks for code; record environment-dependent failures instead of claiming native coverage from mocks.

## Decisions still requiring spike evidence

The preferred defaults are one SDK protocol adapter, one loopback listener, shared operation policy, retained REST compatibility, desktop-owned long-running work, and a bounded snapshot/event model. Slice 0 establishes the candidate dependency versions and both revisions with the tested clients. Remaining gates are native Windows/macOS desktop builds, intended end-user client support beyond Inspector, async Tauri integration, per-operation deadlines and queue tuning, protected config-sync staging, and RDP/credential policy details. See the qualification report for precise limits; none requires the full CLI refactor.
