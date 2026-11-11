# Settings backup, standards-based MCP, and standalone CLI plan

Status: Work 1 desktop implementation added; native release qualification remains. Work 2 and Work 3 are proposed.
Assessment and review date: 2026-11-11.

## Requested outcomes and feasibility

| Work | Verdict | Main work / constraint |
| --- | --- | --- |
| Desktop buttons to export/import all settings, including stored passwords | Viable | Versioned encrypted backup; enumerate app-owned credentials; reliable OS-keyring reads and recoverable restore |
| Complete desktop MCP with Streamable HTTP | Viable | Replace custom HTTP routing with standard MCP; preserve tool behavior, authorization, and compatibility |
| Standalone CLI with its own stdio MCP server | Viable, largest effort | Extract a Tauri-independent runtime; define process lifetime and desktop-only capability behavior |

The CLI must work with the desktop app closed. It will execute the shared runtime itself, not proxy requests to the desktop HTTP endpoint. Its MCP mode will serve that runtime over stdio. Ordinary shell commands retain human-readable/JSON output; stdio is specifically the MCP server mode.

These findings come from source inspection and protocol documentation, not a working prototype. Cross-platform keyring permissions, headless packaging, and SDK integration require implementation spikes before estimates can be firm.

## Current implementation and evidence

- `src-tauri/src/commands/config.rs`: config load/save, plaintext migration, local settings patches, enrollment, and sync actions.
- `src-tauri/src/services/config_service.rs`: app-owned keyring account names, direct `keyring_load_from_os`, write-through `keyring_set_or_delete`, local settings, and initialization.
- `src-tauri/src/stores/keyring_cache.rs`: process-local secret cache. `initialize_keyring_cache` in `services/config_service.rs` currently skips missing credentials and read failures; a backup must distinguish these cases.
- Existing config loaders can migrate and rewrite files, and `save_app_config_action` preserves existing passwords for empty inputs. Neither is a suitable read-only preview or exact-replacement import API. Current file writes also do not provide the proposed coordinated transaction.
- `src-tauri/src/cc/telemetry_queue_crypto.rs`: the queue key is cached in a live cipher; replacing the stored key alone does not safely reconfigure telemetry.
- `package.json`, `playwright.config.ts`, and `tests/`: desktop frontend tests use Playwright; the root package has no `npm test` script. `src-tauri/build.rs` unconditionally invokes Tauri build tooling.
- `src-tauri/src/models/config.rs`: secret-bearing runtime models versus secret-free disk models. Copying `config.json` alone cannot export passwords.
- `src-tauri/src/server_mgmt.rs`: server records; SSH secrets use dynamic `server_ssh_password:{serverId}` accounts.
- `src-tauri/src/mcp/server.rs`: hand-written TCP/HTTP server, bearer token, localhost restriction, custom REST routes.
- `src-tauri/src/mcp/tools.rs` and `resources.rs`: substantial existing operation coverage, but descriptors/results and dispatch depend on Tauri and custom envelopes.
- `src-tauri/src/bin/supercontrolpanelhelper.rs`: existing CLI is an HTTP client requiring the desktop app.
- `src-tauri/src/main.rs`, `services/pty_service.rs`, `services/boundary_service.rs`: runtime stores, events, child processes, and automated OIDC are coupled to Tauri. No reusable core library currently exists.

If adopted, this proposal replaces the helper-only direction in `MCP_HELPER_CLI_SPEC.md` and `MCP_TRANSPORT_AND_AUTH.md` for the new CLI. Those documents still describe the existing desktop-client helper. Keep that helper functional during migration; desktop control remains useful even after a standalone runtime exists.

## Shared architecture

Proposed Cargo workspace boundaries:

```text
scp-core          models, config repository, secret store, runtime stores,
                  operations, PTYs/transfers, backup, policy, domain events
scp-mcp           standard tool/resource schemas and protocol adapter
                  depends on core; transport features selected by host
desktop           Tauri commands/events/dialogs/webviews + HTTP MCP host
scp-cli           shell commands + headless host + stdio MCP host
```

Names are provisional. Avoid a large rewrite: extract one tested operation family at a time. Keep existing frontend command/event names stable.

Replace `AppHandle` dependencies in shared behavior with explicit config paths, runtime context, secret-store access, event sinks, cancellation, and platform capabilities. Desktop converts domain events into existing Tauri events; CLI converts them into terminal output or MCP notifications. Keep telemetry/CC hooks behind host adapters rather than making the core import Tauri or connect automatically to CC.

One operation registry defines names, argument/result schemas, categories, sensitivity, capability requirements, and handlers. Both MCP transports and CLI commands use these operations, but not every local operation must be exposed through MCP. Pass a trusted host-created execution context for policy and runtime ownership; never derive authority from caller-supplied tool arguments. Transport-specific tokens and session metadata belong to adapters, not business logic.

Put Tauri and its build script in the desktop package only. A separate CLI binary inside the current package is insufficient isolation. The first extraction gate is a core/CLI dependency graph and build that require no Tauri, GTK/WebKit, frontend build, or Tauri build script. Platform keyring and SSH libraries may still require native dependencies; document these separately.

## Work 1: complete settings export/import

Implementation: see [desktop backup design, usage, recovery and validation](SETTINGS_BACKUP.md). The v1 importer deliberately blocks Boundary token and telemetry key replacement until the recovery constraints below can be verified. Windows/macOS native credential-store, filesystem behavior and cross-platform release KDF benchmarks remain release qualification work.

### Backup contents and scope

Inventory every persisted setting and reachable app-owned credential before implementation, including any UI preferences found outside the three JSON files. Include a manifest describing included, absent, unavailable, and deliberately unselected sections; never report a partial export as complete.

| Section | Contents and restore behavior |
| --- | --- |
| Shared settings | All `config.json` fields and `servers.json`, preserving stable server IDs |
| Device-local settings | All `sync-local.json` fields, including paths, MCP settings/token, CC settings/identity/token, enrollment state |
| Configured keyring secrets | OpenVPN password/OTP, Boundary password, OIDC password/OTP, per-server SSH passwords |
| Other app-owned credentials | Boundary app token and telemetry queue encryption key; retain in the encrypted archive, but exclude from portable restore and apply the recovery constraints below |
| External dependencies | Manifest of referenced OVPN, certificate/key, SSH identity, and Caddy files; identify missing paths on restore |

The full encrypted export includes all app-managed settings and app-owned credentials above. Default portable import restores user configuration and configured passwords, but does not automatically activate copied CC identity or MCP access. Offer explicit same-device recovery options for identity/token restoration; generate a new MCP token on portable import and require CC re-enrollment by default. Preserve original values inside the backup so full recovery remains possible.

“All settings” does not mean copying the user's entire OS credential store. Read only the app's known service/accounts. Enumerate dynamic SSH accounts from the snapshotted server IDs; the current API has no portable enumeration of orphaned keyring entries. State this limit rather than promising to recover credentials for deleted servers. Boundary CLI-managed credentials, SSH-agent keys, and external files are separate assets; list their scope explicitly. A settings backup initially records external file references rather than silently bundling arbitrary directories or private keys. Do not claim that importing settings alone makes a new machine connection-ready.

Do not restore live PTYs, connection state, telemetry queues, or cached logs. Preserve the destination telemetry key by default. Replacing it can strand an existing destination queue, and the running queue cipher may still use the old key. Same-device key recovery must stop telemetry and verify compatibility with retained queue data; if that cannot be established, block key replacement. Queue backup/migration is a separate feature. Boundary tokens can expire or be controller-specific; validate them before reuse and fall back to login.

Portable import must also preview executable paths, command arguments, sync roots, and privilege-related settings as executable configuration. Preserve target-local paths by default and let the user remap foreign paths. When their sections are selected, restore MCP and CC disabled until explicitly activated after import; retain the source enable flags in the archive rather than silently acting on them. Unselected destination sections remain unchanged.

### Format and export flow

1. Add **Export settings** and **Import settings** buttons to Config.
2. Export asks for a destination and backup passphrase, with confirmation. Credentials are included; the UI shows section counts, never secret values.
3. Collect a consistent snapshot through a configuration transaction gate used by **all** app writers, including desktop commands, MCP, CC, sync, migrations, and credential updates. Cross-process coordination is required before any shared-profile mode ships. Do not hold runtime-store mutexes while an OS-keyring unlock prompt waits for the user; use a dedicated maintenance gate and bounded/cancellable work. Read the OS store directly, bypassing cache. Missing entries are valid; locked, denied, and other read failures are explicit failures for a full backup, with retry or clearly labeled partial export.
4. Encrypt a versioned structured payload using authenticated encryption and a password KDF. Proposed choice: Argon2id plus AES-256-GCM, using reviewed libraries, random salt/nonce, authenticated version/KDF metadata, and bounded KDF parameters. Benchmark parameters across supported platforms. The existing AES-GCM dependency alone is not a passphrase scheme.
5. Write encrypted bytes through a restricted-permission temporary file in the destination directory, flush durable data, and use a platform-appropriate atomic replace. Confirm overwrite, reject unsafe symlink targets, and preserve an existing backup on failure. Cover Unix permissions and Windows ACLs. No plaintext intermediate files, exported credential values sent to the frontend, secret logs, or passphrase in command arguments. A passphrase entered in the desktop UI necessarily crosses Tauri IPC: use a narrowly scoped command, suppress argument logging, and clear UI/backend buffers promptly where practical.

The header records format version, KDF parameters, salt, and nonce. The encrypted manifest records source app/schema/platform versions, timestamp, selected sections, settings, and credential presence/value mapping. Before running the KDF, bound header length, ciphertext size, KDF memory/time/parallelism, and supported algorithm identifiers; authenticate the same header bytes as associated data. Use fresh cryptographic salt/nonce per export. Validate supported payload schemas after authentication, migrate supported older schemas explicitly, and reject unsupported newer schemas without dropping fields. Avoid compression in v1. Backup encryption is portable; it must not rely on the source machine's keyring for decryption.

### Import and failure recovery

1. Decrypt and authenticate fully before mutation; wrong password, corruption, or unsupported schema must leave the machine untouched.
2. Validate schemas, unique server IDs, referential integrity, bounded sizes/KDF cost, and credential-account mappings. Never allow the archive to specify arbitrary keyring service/account names or extraction paths.
3. Show a redacted preview: sections, server counts, changes, foreign-platform paths, identity choices, and unresolved external files. Initial implementation uses replace for selected sections, with explicit confirmation; merge is deferred to avoid ambiguous secret/server-ID conflicts.
4. Compile the preview into explicit `keep` / `set` / `delete` credential actions. For a selected complete section, source absence means deletion to match the source; unavailable or unselected entries mean keep, never delete. Server records and their credentials form a dependent restore group: require matching destination IDs if importing credentials alone. In server-list replacement, include removal of credentials for removed destination servers in the preview and rollback snapshot. Do not route this through the normal save API's empty-means-preserve semantics. Require affected runtime operations, reconnect loops, and browser sessions to stop before replacing their settings; do not silently terminate active work. Recheck the destination revision and archive identity at commit so a stale preview cannot overwrite intervening changes.
5. Snapshot current affected settings and secrets for rollback, then stage validated files. Failure to read any required rollback credential blocks mutation. Apply credentials through OS persistence; publish the new cache/config revision only when the transaction commits, or reload the recovered state after rollback. Commit config files atomically per file while the maintenance gate prevents readers from observing mixed revisions.
6. There is no transaction spanning files and OS keyrings. Durably write a protected encrypted recovery journal **before** the first mutation, with explicit transaction stages and idempotent recovery. Its random encryption key must be durably stored in a dedicated local OS-keyring account outside the restore set and excluded from backups; verify retrieval before mutation. Do not depend on an in-memory key or the backup passphrase remaining available after a crash. On startup, resolve the journal before ordinary config loading/migrations or listeners/connections. If the journal key is locked/unavailable, stop in a recoverable maintenance state. Retain the journal/key until commit or rollback is durably complete, then clean them up. A failed rollback must be reported with actionable recovery information, never as success.
7. Refresh UI/cache and deliberately reconfigure runtimes after a successful commit. Imported settings must not silently enable network listeners, reconnect CC, or launch commands during preview/restore.

Preview must not write config, migrate secrets, enroll devices, or start runtime services. Gate recovery only when a pending local transaction exists.

Keep secret-bearing backup operations out of MCP by default. If later exposed, require a distinct explicit policy and local file destination; never return decrypted backups or stored passwords through ordinary config tools/resources.

## Work 2: standards-based desktop MCP over Streamable HTTP

### Protocol and SDK decision

Use the official Rust `rmcp` SDK with a maintained HTTP framework rather than extending the hand-written HTTP parser. Pin a published SDK version only after verifying its supported protocol revisions, Rust toolchain, dependency compatibility, and cross-platform builds. Do not depend on an unpinned main branch.

The official specification checked for this review is revision **2026-07-28**, which differs materially from 2025-era MCP. Target that revision plus **2025-11-25 compatibility**, subject to the SDK/client spike. Do not mix their lifecycle rules:

- Modern requests use per-request metadata and `server/discover`; legacy clients use `initialize` / `notifications/initialized`. Both HTTP and stdio need tests for each supported lifecycle.
- Modern HTTP uses POST with JSON or request-scoped SSE, required metadata/routing headers validated against the body, and no protocol sessions or GET/DELETE stream lifecycle. Legacy HTTP uses its own initialization, optional sessions, GET streaming (or 405), and DELETE behavior.
- Cancellation differs: modern HTTP closes the request SSE stream, legacy HTTP uses explicit cancellation rather than treating disconnect as cancellation, and stdio uses `notifications/cancelled`. Long-lived modern updates use `subscriptions/listen`. [Transport rules](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http), [legacy rules](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).
- Implement required version-specific semantics through the SDK, and advertise only verified capabilities. SSE alone does not constitute MCP compliance.

If the published SDK or required clients cannot pass the chosen revision tests, release only the verified revision with an explicit compatibility matrix and defer the other; do not hand-build a second protocol engine. The app's custom REST routes are separate from the deprecated 2024 HTTP+SSE transport, which is outside v1 scope.

Scope “complete MCP” as an interoperable tools/resources server with the app's supported operations, standard errors, lifecycle, and progress/events. Optional prompts, sampling, elicitation, and task extensions are only added when needed; do not advertise unsupported features.

### Implementation requirements

- Mount standard JSON-RPC MCP at `/mcp`; implement tools/list, tools/call, resources/list, resources/read and supported resource templates/subscriptions where useful.
- Convert custom envelopes into standard MCP results; distinguish protocol errors from operational tool failures. Preserve tool names and resource URIs where practical.
- Generate public input/output schemas from dedicated DTOs, not secret-bearing runtime models; use shared types only where safe. Validate structured results against output schemas and mark operational failures with standard tool error semantics. Tool annotations are client hints, not enforcement. Enforce arguments and authorization at execution time. Discovery must reflect allowed operations and host capabilities without leaking secrets.
- Deliver status/transfer updates using the selected revision's notification/subscription mechanisms. Provide bounded status snapshots/polling when subscriptions are unavailable; keep progress scoped to an in-flight request, not a completed start call. Bound queues, apply backpressure, and avoid one slow subscriber blocking PTY reads.
- Reuse localhost binding and bearer authorization. Validate Origin and Host/authority, handle IPv4/IPv6 correctly, and avoid permissive CORS. Test requests with and without Origin. Transport sessions are not authorization credentials.
- Document the local bearer-token setup honestly. Any broader OAuth/discovery interoperability is a separately tested capability; a static token is not a general remote OAuth implementation.
- Bound request/body sizes, timeouts, concurrency, connection queues, and event buffers. Move blocking PTY/SFTP/filesystem work off async executor threads. Support cancellation and orderly shutdown without automatically retrying side-effecting calls.
- Preserve category allowlists and audit redaction across HTTP, stdio, resources, errors, and logs, including CC agent tokens as well as MCP tokens. Do not assume transport-level field-name redaction protects the core. Reject unknown policies/categories rather than accidentally treating them as permission to execute. MCP config mutation must not grant itself additional categories, change its approval policy, rotate its own access credentials, or activate CC identity without separate local authorization. Existing `require-ui-approval` currently blocks; initially keep that fail-closed behavior and label it accurately. A real desktop approval queue can follow with timeout, cancellation, and request binding.
- Retain custom REST routes under a documented deprecation period for the existing helper, sharing operation handlers. Remove only after migration and compatibility tests; keep health/status diagnostics separate from the MCP protocol.
- Update Config with endpoint/token handling and tested client setup examples.

Build an operation matrix with existing desktop command, current HTTP tool, CLI command, capability, sensitivity, ownership, and acceptance test. Record deliberate exclusions instead of equating parity with exposing everything. Finish the known operation gaps from that matrix: Boundary target search, OpenVPN executable detection, RDP launch where supported, SSH credential management, and any newly discovered config/runtime gaps. Raw PTY control requires explicit policy and session ownership; decide its scope rather than exposing arbitrary terminal input by accident. Treat the existing single bearer token as one shared local principal: caller-provided client names and protocol session IDs cannot isolate mutually untrusted agents. Multi-principal access would require distinct credentials and ownership checks.

## Work 3: standalone CLI and stdio MCP

### Operation lifetime contract

Separate accepted/running/completed results. Start tools return a runtime identifier plus session/transfer identifier; status queries report eventual outcomes. Request cancellation stops only work still owned by that request, not an already-started VPN or tunnel; those use explicit stop operations. Desktop HTTP disconnect must not tear down app-owned work after the start response. In contrast, shutdown of the CLI stdio host cleans up everything it owns. A lost start response is an unknown outcome: inspect state before retrying and never use JSON-RPC request IDs as durable deduplication keys. Define duplicate-start and stop-already-stopped behavior per operation.

### Proposed commands

```sh
scpctl status --json
scpctl server list
scpctl openvpn start --foreground
scpctl server tunnel start --server-id ID --foreground
scpctl server ssh --server-id ID
scpctl scp upload --server-id ID --local FILE --remote PATH
scpctl settings export --output settings.scpbackup
scpctl settings import --input settings.scpbackup --preview
scpctl mcp serve --stdio
```

`scpctl` is provisional (check naming conflicts before release). Offer version/help, stable exit codes, JSON output, completion, explicit config/profile selection, and safe stdin/file/terminal secret input. Never accept passphrases/passwords as routine command-line values.

### Runtime ownership and process lifetime

- One-shot queries and completed transfers exit after producing results. A fresh `scpctl status` process has no view of another foreground CLI process; report configured capabilities and this process's runtime ID/state explicitly. Use the existing helper to inspect desktop-owned state. Cross-invocation CLI status/control requires the deferred daemon.
- Long-running VPN/tunnel/Caddy commands stay in the foreground in v1, own their child processes, and handle termination cleanly. Starting a tunnel and then exiting cannot honestly return a usable persistent runtime.
- Boundary-backed one-shot SSH/SCP must create, await, and clean up a temporary tunnel within the same invocation, or use a tunnel explicitly supplied as an external dependency. Do not require a second same-profile CLI owner to start the tunnel while the first holds the owner lock. The stdio host can reuse tunnels it already owns. Tests cover readiness timeout and cleanup when either connection or transfer fails.
- The stdio server is a long-running headless runtime owner; sessions survive between MCP calls while that process lives. Status reports this runtime's state, not a fabricated view of the desktop's memory.
- On stdin EOF, signal, or client shutdown, cancel operations and clean up owned sessions, transfers, browser handles, and children within a bounded timeout. Test platform-specific elevated OpenVPN ownership carefully: the existing `terminateOpenVpnWhenHelperExits` setting can conflict with foreground lifetime. Require compatible child ownership for CLI-managed VPN or reject the start. Crash/forced-termination cleanup requires platform process supervision; document limits rather than guaranteeing cleanup after every kill.
- If independent `start`, `status`, and `stop` invocations must manage persistent background work, add an explicit CLI-owned daemon in a later phase. That daemon must not depend on the desktop. Do not accidentally invent persistence through orphan processes.
- Use separate runtime/profile identifiers and an exclusive runtime-owner lock per profile in v1; multiple MCP clients commonly spawn separate stdio processes. Fail with an actionable conflict when a second process tries to own that profile. Protect config writes with cross-process locking and detect tunnel/listener port conflicts. Do not share PTY ownership across desktop and CLI or let a CLI shutdown stop desktop children.
- Default to a deterministic separate CLI config root/profile, with explicit profile selection and settings import for migration. Each CLI profile gets a stable, locally generated credential namespace that is not copied from the archive. A different config directory alone does not isolate the current process-global cache or fixed keyring accounts. Key caches by namespace/account and keep existing desktop identifiers unchanged. Defer sharing desktop files until cross-process coordination and cache invalidation are implemented.
- Separate profiles do not isolate machine-wide VPN routes/DNS, elevation, or external Boundary CLI credential stores. Define a host-wide conflict policy for VPN ownership and choose isolated external CLI state where supported; otherwise surface the sharing and require explicit use. Do not promise concurrent VPN isolation merely because locks/ports differ.

### Host capability differences

| Capability | Headless CLI approach |
| --- | --- |
| Config, servers, SCP/SFTP, tunnels, Caddy | Shared core operations |
| Interactive SSH/password Boundary auth | CLI terminal/PTYS; MCP cannot borrow protocol stdin for password prompts |
| Browser-based Boundary login | System browser only where available; otherwise explicit unsupported/manual flow |
| Automated OIDC using hidden Tauri webview | Desktop-only initially; headless replacement is a separate validated project |
| OpenVPN elevation | Preserve Windows helper/UAC and Unix sudo behavior through platform adapters; clearly fail when interaction is unavailable |
| RDP launch | Optional native-client capability on supported interactive hosts |
| OS keyring | Same supported OS stores, with explicit locked/unavailable errors; Linux headless environments may lack a secret service |
| CC agent/telemetry | Remains desktop-hosted initially; CLI must not duplicate enrolled device identity or emit duplicate telemetry |

Expose capability availability/reasons rather than promising identical behavior on a machine without a browser, desktop, keyring, or elevation mechanism. Document this matrix as the deliberate limit of parity.

### stdio MCP contract

- Use the same SDK adapter, operation schemas, policy, and redaction as desktop MCP, hosted directly on the headless runtime.
- Standard newline-delimited JSON-RPC on stdin/stdout; diagnostics only on stderr. Capture child output and convert it into explicit results/events so it cannot corrupt the protocol stream.
- No listening HTTP port or desktop token discovery in stdio mode. Authorization comes from the launching user's process context plus explicit local operation policy; tokens remain relevant to HTTP.
- Never prompt on protocol stdin or block indefinitely on an invisible approval UI. For sensitive actions require an explicit configured allow policy, otherwise fail closed with an actionable error. This applies even though a client can launch the process.
- Support concurrent requests, cancellation, progress, and bounded output; keep raw terminal/secret output out of ordinary status resources.
- Produce a separately installable binary that runs without Tauri, a webview, frontend assets, or the desktop app. External programs such as OpenVPN, Boundary, SSH/SCP, and Caddy remain prerequisites for their respective operations.

## Delivery order and TDD gates

Every code increment must follow repository RED → GREEN → REFACTOR: write a behavioral test, run and verify the expected failure, implement minimally, then rerun relevant tests and the existing suite. Before editing existing symbols, run `ripwire . --impact="<symbol>" --legend=compact` and `ripwire . --uses="<symbol>" --legend=compact`; report direct callers, affected paths, and risk, and warn before high-risk edits. No symbol impact analysis or runtime refactoring was performed during this documentation-only assessment.

1. **Inventory and spikes.** Capture settings/credential/operation/capability matrices; baseline current tests. Specify the backup schema, secret-action semantics, transaction/recovery-key design, and local-only policy controls before implementation. Verify SDK published-version support using one real client for each target protocol era and both transports. Verify direct keyring access and a minimal headless build on supported OSes. Resolve dependency/toolchain issues before committing to extraction boundaries.
2. **Backup foundation and desktop UI.** Introduce injectable secret/config repositories, coordinated writers, and backup format; implement encrypted export, preview, recoverable import, and Config buttons. Complete this first user-visible feature without waiting for the full CLI extraction.
3. **Core extraction.** Move config, policy/registry, stores/events, PTY lifecycle, and each operation family behind host adapters. Characterization tests preserve desktop command/event contracts throughout. Build the core with no Tauri dependency.
4. **Desktop Streamable HTTP.** Add SDK-based MCP adapter and HTTP hosting with compatibility routes; validate protocol behavior and permissions before finishing operation gaps. This needs the policy/registry seam, not completion of every PTY/CLI extraction; it may ship earlier once that seam is ready.
5. **Standalone CLI.** Add direct core commands, foreground lifetime, profile handling, and OS adapters. Verify it works with the desktop closed and HTTP disabled.
6. **stdio MCP.** Host the same adapter on the CLI runtime; validate process lifecycle, clean stdout, long-running operations, and cross-transport schema/policy parity.
7. **Release and migration.** Package and smoke-test Windows/macOS/Linux CLI artifacts independently of the desktop, update README/AGENTS and older MCP docs, publish client examples and the capability matrix, then deprecate the helper only when replacements cover its documented usage.

### Required acceptance tests

- Backup round-trip includes every configured secret, dynamic server credentials, and local settings; source can be a different platform. Configured keyring secrets are written to the target OS store, never plaintext JSON. Device-local MCP/CC tokens currently live in `sync-local.json`; preserve their documented storage contract and portable-restore rules rather than claiming this work migrates all tokens into the keyring.
- Locked keyring versus missing credential, stale cache, wrong passphrase, tampering, malformed/oversized archive, unknown schema, unsafe account mapping, and restrictive file permissions are covered.
- Inject failure after every import stage, including journal/key creation, keyring write/delete, file commit, rollback failure, and process interruption; verify recovery and cache consistency. Cover a locked journal key on restart, stale preview, concurrent writers, source-absent versus unavailable secrets, removed-server credential cleanup, and preservation of unselected sections.
- Portable restore leaves imported CC/MCP sections inactive, preserves target queue encryption and unselected settings, cannot trigger imported commands, and keeps preview free of config/keyring mutations.
- UI export/import cancellation, passphrase confirmation, preview, selected-section behavior, external-path warnings, and success/failure presentation are tested with Playwright and mocked Tauri IPC/native dialogs where appropriate; supplement with real desktop smoke tests. Use `npx playwright test`, `npm run build`, and `cargo test --manifest-path src-tauri/Cargo.toml` as the current baseline, recording environment-dependent failures. Do not invent a root `npm test` command or assume React/Vitest.
- MCP tests use an official SDK client plus representative real clients: discovery/initialization by revision, schemas/results/errors, malformed input, authorization/Origin, request streaming, cancellation, event backpressure, concurrent calls, and shutdown.
- CLI integration tests close/disable the desktop, exercise direct SSH/tunnel/transfer fixtures, assert exit codes and JSON output, test profile locking/conflicts and credential isolation, and verify owned child cleanup. Also run desktop and CLI concurrently to verify CLI shutdown cannot stop desktop children or change desktop secrets.
- MCP tests reject remote policy escalation and unknown policy values, check token rotation invalidates subscriptions/access as designed, and distinguish request cancellation from lifetime of an accepted background operation. Include duplicate starts and loss of the start response.
- stdio subprocess tests assert every stdout line is valid protocol JSON, PTY/subprocess output never leaks into stdout, EOF cleanup works, and sensitive calls cannot bypass policy.
- Cross-platform CI checks compile and exercise the extracted core/CLI independently of GUI libraries. Real OS-keyring/elevation/browser smoke tests supplement mocks; unavailable platform checks must be documented rather than claimed as passed.

### Explicitly deferred from the first release

No daemon, background cross-invocation CLI control, shared desktop/CLI profile, full remote OAuth deployment, 2024 HTTP+SSE transport, raw PTY API, headless webview OIDC replacement, external-file bundling, queue migration, or restore merge mode. A functioning desktop approval queue remains separate from the initial fail-closed policy. These can be added after the three requested outcomes work within the stated capability limits.

## Risks and decisions to validate

The main risk is hidden Tauri coupling, especially webview OIDC, elevated Windows OpenVPN, PTY telemetry, and lifecycle hooks. Next are non-transactional keyring restore, portable paths/device identity, and conflicts between independent runtime owners. Staged extraction and explicit host capabilities contain these risks.

Proposed defaults are: encrypted complete export; portable import with optional same-device identity recovery; referenced external files listed rather than bundled; foreground CLI runtime first; stdio MCP owns its runtime; desktop-only webview OIDC initially; modern MCP plus tested 2025 compatibility. These are planning assumptions, not implemented restrictions, and can be revised before their respective implementation stages.

No fixed delivery estimate is justified until the extraction/SDK/platform spikes finish. Relative effort: backup is medium, standard HTTP MCP is medium-to-large, and a truly independent CLI with runtime parity is large. This should be several reviewable changes, not one combined rewrite.

## External references checked

- [Official MCP transport overview](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports)
- [Streamable HTTP binding](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http)
- [stdio binding](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio)
- [Legacy 2025-11-25 transport contract](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
- [Protocol versioning and compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)
- [Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)

Recheck pinned SDK releases and actual client support at implementation time. Current specification support in upstream source is not proof that a particular published SDK release or user client implements every revision.
