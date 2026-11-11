# Settings backup, standards-based MCP, and standalone CLI plan

Status: Work 1 desktop implementation added; native release qualification remains. Work 2 Slices 1–6 are implemented; release qualification limits are recorded in its dedicated plan. Work 3 Slice 0 is in progress (shared registry and independent CLI discovery); runtime and stdio gates remain open in its dedicated plan.
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

The detailed scope, source baseline, protocol/SDK gate, operation matrix requirements, security and lifetime contracts, delivery slices, and acceptance checklist now live in [Desktop MCP over Streamable HTTP](DESKTOP_STREAMABLE_HTTP_PLAN.md). That file is the source of truth for Work 2 implementation details; this roadmap retains cross-work dependencies and delivery order.

The desktop will expose standard MCP at `/mcp` using a pinned official Rust SDK and a maintained HTTP framework, while retaining the current REST routes for the helper. Target 2026-07-28 plus tested 2025-11-25 compatibility, gated on published SDK and real-client evidence; release only revisions actually verified. Preserve existing operation names/resource URIs, enforce shared policy and safe schemas, and complete explicitly reviewed operation gaps.

Work 2 can ship once the operation registry/policy and desktop adapter seams exist. It does **not** require the full standalone core/CLI extraction. Desktop-owned operations survive HTTP disconnects and listener restarts. The reusable schemas/policy are intended for Work 3's stdio adapter, while CLI ownership and process lifetime remain Work 3 concerns.

## Work 3: standalone CLI and stdio MCP

The detailed scope, shared-core extraction, operation lifetime, command surface, profile/credential isolation, headless capabilities, stdio contract, delivery slices, and acceptance tests now live in [Standalone CLI and stdio MCP](STANDALONE_CLI_STDIO_MCP_PLAN.md). That file is the source of truth for Work 3 implementation details; this roadmap retains cross-work dependencies and delivery order.

The CLI executes a Tauri-independent runtime directly with the desktop closed and HTTP disabled. Foreground commands and the stdio host own their sessions and child processes. Reuse Work 2's schemas/policy and Work 1's backup implementation; daemon-based control and shared desktop/CLI profiles remain deferred.

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
