# Standalone CLI and stdio MCP plan

Status: Slice 0 Linux `x86_64` feasibility gate, **Slice 1** (shared `scp-core` / `scp-mcp` crates, profile owner lease, namespaced keyring store, startup journal recovery, `scpctl` read-only CLI commands, read-only `scpctl mcp serve --stdio`, and `src-tauri` shared re-exports), **Slice 2** (`scp-core` runtime worker families and 9 direct `scpctl` runtime commands), **Slice 3** (shared `scp-core::backup` `.scpbak` engine, safe TTY/secret/passphrase readers, `completion`, server/config/settings/credential/sync-push mutations, `server rdp`, and `settings export`/`import`), and **Slice 4** (`scp-core` full headless runtime parity, `41/45` `Capability::Runtime` stdio MCP workflows with `4` explicit `CAPABILITY_UNAVAILABLE` exclusions, live `stdio-policy.json` reload, concurrency/cancellation/framing bounds, and `@modelcontextprotocol/sdk`, `@modelcontextprotocol/client`, and `@modelcontextprotocol/inspector` CLI qualification) are **complete on Linux `x86_64`**. Windows, macOS, and GUI-free CI runner checks plus Slice 5 remain open gates. See the [operation ledger](STANDALONE_CLI_OPERATION_LEDGER.md), [Slice 0 report](STANDALONE_CLI_SLICE0.md), [Slice 1 report](STANDALONE_CLI_SLICE1.md), [Slice 2 report](STANDALONE_CLI_SLICE2.md), [Slice 3 report](STANDALONE_CLI_SLICE3.md), [Slice 4 report](STANDALONE_CLI_SLICE4.md), and [stdio TAP evidence](evidence/CLI_STDIO_SLICE0.tap).
Extracted on 2026-11-11 from Work 3 in [the combined roadmap](SETTINGS_BACKUP_STREAMABLE_HTTP_STANDALONE_CLI_PLAN.md).
Refined against repository baseline `1fa4669` on 2026-11-11; recheck the source before each implementation slice.

## Outcome and dependencies

Produce a separately installable CLI primarily for AI agents, working with the desktop app closed and HTTP disabled. It executes a shared, Tauri-independent runtime directly; its MCP mode serves that runtime over stdio. Agents using shell tools receive stable structured commands; MCP clients receive typed tools/resources and a persistent process-owned runtime. Human-readable output and interactive administration remain supported. The existing `supercontrolpanelhelper` remains a desktop HTTP client during migration.

This file is the source of truth for Work 3 implementation details. [Desktop MCP over Streamable HTTP](DESKTOP_STREAMABLE_HTTP_PLAN.md) owns Work 2's protocol adapter, operation schemas, policy, and desktop HTTP host. Reuse its implemented seams and [operation inventory](DESKTOP_STREAMABLE_HTTP_INVENTORY.md), verifying their current state before extraction. Work 3 owns the independent runtime, CLI commands, profile/credential isolation, process lifetime, stdio hosting, and packaging. Work 2 does not require this extraction to ship.

Reuse the [settings backup implementation](SETTINGS_BACKUP.md) for local CLI export/import while preserving its writer/recovery gates and portable-restore restrictions. Backup operations remain local CLI commands and are absent from MCP in v1. This plan does not reopen Work 1 or Work 2 release qualification.

## Primary workflow: AI agents

Agent usability is a release requirement, not a consequence of exposing MCP. Both entry points must let an agent discover valid operations, act with explicit inputs, determine the outcome, and recover without parsing terminal prose or waiting for a person unexpectedly.

| Agent access | Intended workflow | Lifetime boundary |
| --- | --- | --- |
| Shell tools | Discover command schemas; run bounded queries, config actions, and completed transfers with `--json --non-interactive` | One invocation owns its work. Boundary-backed transfers acquire and release a temporary tunnel themselves. |
| stdio MCP | Discover tools/capabilities; start a tunnel/function; inspect status/readiness; transfer/browse; stop work | Keep one host alive for the workflow. Use its tools/resources for status and control; separate shell invocations cannot inspect its runtime. |
| Human setup/recovery | Configure credentials and local permissions, complete interactive auth/elevation where required, or confirm a restore | Explicit setup boundary. Agent operations report what is missing and exit promptly rather than starting an unseen prompt. |

The reference agent loop is **discover → check capability/policy → resolve exact IDs → execute → inspect outcome → clean up**. Acceptance tests must complete this loop with preconfigured credentials/policy, the desktop closed, no controlling terminal, and no manual intervention. A shell agent without a persistent MCP connection can use one-shot workflows; arbitrary cross-invocation start/status/stop remains outside v1. Interactive SSH is a human workflow, not evidence of an agent remote-command execution API.

Required agent-facing contracts:

- **Machine-readable discovery.** Add proposed `scpctl schema --json` (optionally narrowed by `--command`) for versioned command/input/output metadata without opening a profile. Add `scpctl capabilities --json --non-interactive` for the selected profile's available operations, policy decisions, prerequisites, and stable reason codes. Use the same core/registry metadata for shell and MCP where operations overlap. Provide bounded read-only capability diagnostics through a reviewed MCP tool/resource; add its name/schema to both hosts' inventory and parity tests before exposure.
- **No surprise interaction.** Ordinary automation commands default to noninteractive execution even when launched under a PTY. Require explicit `--interactive` for terminal/password/confirmation flows; `--non-interactive` conflicts with it. `--json` implies noninteractive behavior and disables colors, spinners, pagers, and prose on stdout. Missing credentials, host-key trust, overwrite decisions, or elevation readiness produce a structured error before the dependent side effect. Do not auto-accept SSH host keys.
- **Precise, bounded results.** Mutations accept exact IDs/paths rather than fuzzy name selection. Lists have stable ordering and documented bounds; report truncation and a supported continuation/filter strategy rather than silently hiding records. Return safe summaries by default. Optional filtering/limits belong in reviewed CLI/MCP schemas; do not silently change existing desktop results.
- **Actionable outcomes.** The CLI error contract includes stable `reason`, `nextAction` (for example `configure_credentials`, `inspect_status`, or `use_owner_connection`), and `outcome` (`not_started`, `partial`, or `unknown`) in addition to `code/message/retryable`; successful operations distinguish acceptance from completion. Include available runtime/session/transfer IDs for correlation. Keep these fields typed and redacted; never require an agent to scrape stderr to choose its next step. Coordinate any equivalent MCP additions across both hosts.
- **Deliberate mutations and retries.** Document each operation's retry/idempotency behavior and exact target selection. Provide a non-mutating preview for supported config changes and destructive file actions, plus explicit overwrite/recursive choices; label operations without preview support. A preview is advisory and grants no authority: execution rechecks policy and relevant target state, failing on a changed target where correctness depends on it. After a timeout or lost response, query state before retrying a mutation. No generic `--yes` bypasses policy, host-key verification, or recovery gates.
- **Finite waits.** Publish defaults and bounded timeout controls for request-owned operations and readiness checks. A foreground service or the stdio host intentionally lives until shutdown; a request timeout must not silently become its service lifetime. Missing interaction, stalled output, and authentication failure must not leave the agent waiting indefinitely.
- **Treat remote content as data.** Server names/descriptions, paths, and remote errors are untrusted output, never instructions or authorization. Keep them in typed result fields; executable remediation guidance comes from local error codes/templates. Do not interpolate returned text into shell commands or expose raw terminal history by default.

Ship a short agent usage guide and executable examples for shell-only and MCP workflows, including capability failure, owner conflict, async completion, lost-response recovery, and cleanup. Provision profile credentials and stdio permissions before the workflow; calls cannot grant themselves more authority. Interactive restore apply remains a documented v1 exception, not an unattended-agent feature.

## Verified extraction baseline

The following focused source checks supersede the original extraction assumptions. Older baseline sections in the Work 2 inventory are historical; use its completed slice reports and current code when reconciling contracts.

| Current source | Verified behavior / extraction consequence |
| --- | --- |
| [`Cargo.toml`](../../src-tauri/Cargo.toml), [`build.rs`](../../src-tauri/build.rs), [`supercontrolpanelhelper.rs`](../../src-tauri/src/bin/supercontrolpanelhelper.rs) | One desktop package includes Tauri/build tooling and the HTTP helper. Production pins `rmcp =3.9.0` with Streamable HTTP support and `axum =0.8.8`; stdio is not a qualified host. Add independent packages rather than another binary in this package. |
| [`contracts.rs`](../../src-tauri/src/mcp/contracts.rs), [`validation.rs`](../../src-tauri/src/mcp/validation.rs), [`public_dto.rs`](../../src-tauri/src/mcp/public_dto.rs) | Registry, schemas, safe DTOs, and stable errors exist. Capability metadata still uses `Desktop`/`InteractiveDesktop`; lifetime metadata uses `DesktopOwned`. Generalize these internal concepts without changing public names, schemas, or desktop policy. |
| [`protocol.rs`](../../src-tauri/src/mcp/protocol.rs), [`server.rs`](../../src-tauri/src/mcp/server.rs), [`tools.rs`](../../src-tauri/src/mcp/tools.rs) | `ProtocolHost` separates discovery/execution callbacks, but also carries a legacy HTTP callback; its module imports Axum/Streamable HTTP. Desktop callbacks capture `AppHandle`. Split the SDK handler from HTTP routing and supply a direct core host. |
| [`lifecycle.rs`](../../src-tauri/src/mcp/lifecycle.rs) | Admission/cancellation bounds queued blocking work; a started closure retains its permit and can outlive a timeout. Dropping a future does not terminate its child process. Cooperative domain cancellation and CLI-owned shutdown need separate implementation. |
| [`config_service.rs`](../../src-tauri/src/services/config_service.rs), [`keyring_cache.rs`](../../src-tauri/src/stores/keyring_cache.rs) | Paths depend on `AppHandle`; keyring service/accounts are fixed and the global cache is keyed only by account. Both OS account resolution and cache keys need profile scope, including restore recovery credentials. |
| [`settings_backup/`](../../src-tauri/src/settings_backup/) | Crypto, archive validation, transaction/recovery, and `SecretStore` already exist. Desktop owns in-memory previews, capability checks, and cache refresh; maintenance exclusion is process-local. Extract those host responsibilities while retaining the archive format and transaction engine. |

Runtime extraction still requires focused characterization of `main.rs`, `commands/`, `services/pty_service.rs`, and `services/boundary_service.rs`: PTY readers, exit monitors, CC callbacks, elevation, and hidden-webview authentication remain desktop integration points. This document is not a completed platform or dependency audit.

Consult [the helper specification](MCP_HELPER_CLI_SPEC.md) and [transport/auth documentation](MCP_TRANSPORT_AND_AUTH.md) for existing helper behavior. The standalone runtime replaces the helper-only direction for the new CLI, while preserving the helper's desktop-control use cases during migration.

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

The allowed dependency direction is `desktop/scp-cli → scp-mcp → scp-core`, with both hosts also allowed to call core directly. Core imports neither MCP nor either host. SDK and JSON-RPC types stay in `scp-mcp`; HTTP routing, legacy REST envelopes, bearer/session handling, and Axum stay in the desktop HTTP adapter or an optional HTTP feature. The CLI selects only stdio support. Update workspace membership, lockfile location, Tauri paths, and build scripts together; verify existing desktop build commands still work.

Replace `AppHandle` dependencies in shared behavior with explicit config paths, runtime context, secret-store access, event sinks, cancellation, and platform capabilities. Desktop converts domain events into existing Tauri events; CLI converts them into terminal output or MCP notifications. Keep telemetry/CC hooks behind host adapters rather than making the core import Tauri or connect automatically to CC.

Retain one public MCP registry defining names, argument/result schemas, categories, sensitivity, capability requirements, and core handler mappings. Both MCP transports and CLI commands call the same core operations; local backup and terminal workflows do not need MCP registry entries. Shared domain operations must not return REST envelopes or SDK values. Pass a trusted host-created execution context for policy, capabilities, runtime ownership, cancellation, and audit correlation; never derive authority from caller-supplied tool arguments. Transport-specific tokens and session metadata belong to adapters, not business logic. Update error guidance that currently tells all callers to inspect the desktop, preserving existing error codes and redaction.

Put Tauri and its build script in the desktop package only. A separate CLI binary inside the current package is insufficient isolation. The first extraction gate is a core/CLI dependency graph and build that require no Tauri, GTK/WebKit, frontend build, or Tauri build script. Platform keyring and SSH libraries may still require native dependencies; document these separately.

### Operation lifetime contract

Separate accepted/running/completed results internally. An accepted start means the host has registered ownership and a stable session/transfer identifier; it does not prove authentication or network readiness. Status queries report eventual outcomes. Keep existing MCP/Tauri result shapes; expose runtime identity through CLI output/host diagnostics and review any additive MCP schema change across both hosts before publishing it. Correlate async SCP session IDs with transfer IDs explicitly.

| Operation phase | Cancellation / failure contract |
| --- | --- |
| Queued or preparing | Cancellation prevents launch when possible; roll back any partially created request-owned children and reservations. |
| Accepted host-owned VPN/tunnel/SSH/async transfer | Commit ownership before replying. Request cancellation or HTTP disconnect does not stop it; explicit stop operations or host shutdown do. |
| Request-owned synchronous transfer or temporary tunnel | Propagate cancellation into the operation, await bounded cleanup, and report possible partial remote effects. A timeout is not proof of rollback. |
| Config/credential mutation or restore commit | Check cancellation before the mutation boundary; complete commit/rollback once consistency requires it. Do not report a completed write as undone merely because its caller disconnected. |

Treat acceptance and cancellation as a race with one ownership transition, covered by a deterministic test. A lost start response is an unknown outcome: inspect state before retrying and never use JSON-RPC request IDs as durable deduplication keys. Reserve function IDs and listener ports before launch; concurrent duplicate starts return the existing compatible owned session or `CONFLICT`, never create two children. Stop of an already-stopped known function succeeds; unknown/stale session IDs cannot address a new runtime's children.

Define readiness per family before implementing its start command: process spawn, listening socket, completed authentication, and usable VPN routes are distinct observations. A dependent SSH/SCP operation waits for its tunnel's verified readiness with a deadline, and checks for early child exit. Preserve existing function IDs and desktop events; keep readiness detail in the core/CLI contract until a coordinated public schema change is approved.

### Proposed commands

```sh
scpctl schema --json
scpctl --profile agent-a capabilities --json --non-interactive
scpctl status --json --non-interactive
scpctl server list --json --non-interactive
scpctl openvpn start --foreground
scpctl server tunnel start --server-id ID --foreground
scpctl server ssh --server-id ID --interactive
scpctl scp upload --server-id ID --local FILE --remote PATH --json --non-interactive
scpctl settings export --output settings.scpbak
scpctl settings import --input settings.scpbak --preview
scpctl settings import --input settings.scpbak --apply --interactive
scpctl --profile agent-a mcp serve --stdio
```

`scpctl` is provisional (check naming conflicts before release). Offer version/help, stable exit codes, JSON output, completion, explicit config/profile selection, and safe stdin/file/terminal secret input. Never accept passphrases/passwords as routine command-line values.

### Shell command contract

- Resolve profiles independently of the working directory. Proposed precedence: explicit `--config-dir` root, CLI-specific environment override, then the platform's per-user CLI config root; `--profile` selects a validated child profile, defaulting to `default`. Print the resolved location in diagnostics. Never implicitly discover desktop settings or fall back to them on failure.
- `--help`, `--version`, `schema`, and completion generation do not open a profile, unlock the keyring, or perform recovery. All profile-backed commands use the startup sequence below. Foreground operation is the only v1 start mode; `--foreground` documents that intent, and no detached option is accepted.
- Noninteractive shell commands with `--json` emit exactly one versioned result object on stdout: `schemaVersion`, `ok`, `runtimeId` (nullable before startup), `data`, and `error` (null or the typed error defined above). Human progress and sanitized diagnostics go to stderr. Foreground JSON commands emit their final result at exit; JSON streaming is deferred. Interactive SSH and raw terminal sessions reject `--json` before launch. `mcp serve --stdio` also rejects `--json`; its stdout contains only MCP messages, including during startup failure/shutdown (diagnostics use stderr).
- Proposed process exits: `0` success, `1` operational failure, `2` usage/invalid arguments, `3` policy denied, `4` capability unavailable, `5` conflict/recovery blocked, `6` not found, `7` timeout, and `130` handled user interruption. Publish and test the mapping, including cleanup failures; OS-forced termination cannot promise a final JSON object or these exit values. MCP tool errors stay protocol results and do not terminate the stdio host.
- Explicit `--interactive` commands use a controlling terminal and restore terminal modes on every handled exit. Noninteractive execution is the default and never waits for approval/password input. Prompting failures are explicit capability errors. Reuse dedicated file/stdin/terminal secret readers with bounded input and no echo; reject ambiguous stdin uses before starting work. stdio MCP cannot use these shell secret-input flags. Headless capability/keyring checks must also obey these interaction and deadline rules.
- No standalone `stop`, transfer-history, or browser-ID command may imply access to a different invocation's runtime. Map stateful controls to calls within the stdio host; shell SSH/SCP complete within one invocation. If a command is unavailable without a persistent owner, its help must say so.

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

### Profile startup, locking, and recovery

Use one exclusive OS-backed owner lease for **every profile-backed invocation**, including queries, to avoid reads racing migration/recovery. Hold it until work and cleanup finish. This deliberately means `status` against a profile owned by a stdio server fails with `CONFLICT`; query that host through MCP instead. Diagnostics identify the profile and owner without secrets. PID files alone are not locks; never delete a lease or kill a process merely because its recorded PID looks stale.

Startup order is part of the contract:

1. Resolve and validate the profile path, establish private directory permissions, and acquire its owner lease. Canonicalize aliases so two paths cannot acquire independent leases for one profile. Reject the known desktop profile location in v1; relocation is an explicit import workflow.
2. Read or atomically create the profile's stable credential namespace in separate local metadata, outside the three portable settings files. A missing namespace with existing recovery state is an error, not permission to create a new identity.
3. Recover a pending restore journal using that namespace **before** config loading, legacy secret migration, cache initialization, or runtime startup. Failed recovery blocks ordinary profile work with actionable, redacted diagnostics.
4. Load and validate configuration, initialize the namespaced secret store/cache, then resolve capabilities and local MCP policy. Do not unlock/read unrelated credentials merely to list servers. Acquire machine-wide VPN coordination only when starting VPN work.
5. Start requested work or begin serving stdio. On shutdown, stop admission, cancel request work, close browsers/transfers/SSH before their dependent tunnels, stop remaining owned functions, reap children, and release leases last. Never hold runtime-store mutexes while waiting on a child, keyring, or shutdown task.

The owner lease is the cross-process boundary; the existing maintenance gate still excludes concurrent operations within a stdio host during backup/config maintenance. Define their acquisition order in the core and test contention. Namespace all credential access, including server passwords, Boundary tokens, backup rollback reads, and `settings-restore-recovery-v1`; archive account names remain logical names resolved by the destination `SecretStore`. Desktop service/account names remain unchanged. Copying raw profile directories is not a supported clone operation because it copies credential identity; use portable import.

VPN coordination must include desktop and CLI launch paths before simultaneous use is claimed. A CLI-only lock cannot detect an older desktop or an externally launched VPN; document that limit and reject detectable conflicts without stopping foreign sessions. The feasibility slice must specify how elevated Windows children participate in ownership and how each supported platform handles crash cleanup. Unsupported combinations return a capability error before launch.

### Local backup workflow

Keep the existing `.scpbak` format, KDF/file bounds, exact credential actions, encrypted journal, and portable-restore rules. The CLI `SecretStore` maps logical archive accounts into the destination namespace; importing must not replace that namespace or access desktop credentials. Recovery keys are never exported. CLI import does not offer same-device desktop identity recovery or enable CC/MCP HTTP hosting.

- `settings export` reads selected-profile credentials directly from the OS store, preserves complete-export failure semantics, and publishes encrypted output with the existing permission/overwrite protections.
- `settings import --preview` authenticates and validates the archive, prints a redacted plan, then exits without mutating settings/credentials. Normal profile startup may still perform mandatory recovery. Its process-local preview ID is not a reusable authorization token.
- `settings import --apply --interactive` is the v1 apply workflow: load once, select sections/path overrides, show a fresh preview, confirm, revalidate the destination, and commit the same in-memory archive in that process. Preserve preview expiry and selection invalidation. Reject noninteractive apply before mutation; a durable reviewed-plan format for unattended restore is deferred.
- Import requires a quiescent owned runtime and the writer/recovery gate. Once mutation starts, finish commit/rollback despite cancellation; report recovery requirements if cleanup cannot finish. Never write decrypted archives or passphrases to a preview file, stdout, or diagnostics.

### Host capability differences

| Capability | Headless CLI approach |
| --- | --- |
| Config, servers, SCP/SFTP, tunnels, Caddy | Shared core operations as each family passes extraction tests; existing protected-import/reviewed-git restrictions remain |
| Interactive SSH/password Boundary auth | CLI terminal/PTYS; MCP cannot borrow protocol stdin for password prompts |
| Browser-based Boundary login | Only after validating the existing Boundary CLI flow on the host; do not assume a system browser replaces the hidden-webview OIDC implementation |
| Automated OIDC using hidden Tauri webview | Desktop-only initially; headless replacement is a separate validated project |
| OpenVPN elevation | Preserve Windows helper/UAC and Unix sudo behavior through platform adapters; clearly fail when interaction is unavailable |
| RDP launch | Optional native-client capability on supported interactive hosts |
| OS keyring | Same supported OS stores, with explicit locked/unavailable errors; Linux headless environments may lack a secret service |
| CC agent/telemetry | Remains desktop-hosted initially; CLI must not duplicate enrolled device identity or emit duplicate telemetry |

Expose capability availability/reasons rather than promising identical behavior on a machine without a browser, desktop, keyring, or elevation mechanism. Document this matrix as the deliberate limit of parity.

### stdio MCP contract

- Reuse the SDK handler, operation schemas, policy, and redaction after separating them from HTTP routing; host them directly on the headless runtime. Do not bridge stdio to the desktop HTTP endpoint.
- Use newline-delimited UTF-8 JSON-RPC on stdin/stdout; diagnostics only on stderr. Child processes receive explicit pipes/PTYS, never inherited protocol stdin/stdout. Drain child output into bounded private buffers; publish only approved sanitized results/progress. Do not forward arbitrary terminal output as MCP notifications.
- No MCP HTTP listener or desktop token discovery in stdio mode. Authorized domain operations may still open their own tunnel/Caddy listeners. Authorization comes from the launching user's process context plus explicit local operation policy; bearer tokens remain relevant to HTTP MCP.
- Default stdio policy to read-only. Sensitive actions require a locally configured allow policy and enabled category; imported desktop allow settings cannot silently grant stdio authority. Keep policy metadata outside portable settings and reject all MCP paths that could mutate it, including indirect config/sync/git paths. Unknown policy/category values fail closed. Local shell intent is a separate authority source, not permission to bypass core validation or recovery gates.
- Never prompt on protocol stdin or block on an invisible approval UI. Operations requiring a terminal, webview, keyring unlock, or elevation interaction fail with a capability reason when unavailable. The host must not acquire a controlling terminal behind the client's back.
- Support concurrent requests, request cancellation, negotiated progress/subscriptions, and bounded output. Retain status snapshots as authoritative; coalesced invalidations are hints, not an event log. Reuse all six current resource mappings, including `scp://status/transfers`, with identical policy and redaction. Report unsupported capabilities and recheck them at execution.
- Pin and test limits for input line size, argument/result size, admitted workers, retained history, subscriptions, output queue, writes to a stalled client, and shutdown. Prefer existing verified bounds where applicable. Dropping an update may trigger a snapshot refresh; never silently drop a response. On a broken pipe or sustained write timeout, shut down the owned runtime. Verify EOF remains observable when workers/output are saturated.
- Produce a separately installable binary that runs without Tauri, a webview, frontend assets, or the desktop app. External programs such as OpenVPN, Boundary, SSH/SCP, and Caddy remain prerequisites for their respective operations.

Qualify stdio independently for each claimed revision. The [2026-07-28 stdio specification](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio) uses inline request metadata and explicit cancellation notifications; closing stdin ends the entire host, unlike closing a request-scoped HTTP stream. [2025-11-25 transport rules](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports) also reserve stdout for protocol messages. Let the pinned SDK implement revision-specific discovery/initialization and framing; do not assume passing the HTTP fixture proves stdio compatibility. Document client restart as a new empty runtime, with no automatic replay of side-effecting calls.

## Delivery slices and TDD gates

Every code increment follows repository RED → GREEN → REFACTOR: write a behavioral test, run and verify the expected failure, implement minimally, then rerun relevant tests and the existing suite. Before editing existing symbols, run repository-required Ripwire impact/uses/affected checks and report callers, affected paths, and risk; warn before high-risk edits. This refinement inspected selected source but made no runtime changes; implementation impact analysis remains required.

| Slice | Deliverable | Exit gate |
| --- | --- | --- |
| 0. Feasibility and scope | A current operation ledger and minimal independent CLI/core build; platform spikes for keyring, process supervision, elevation, and external Boundary state | Every current registry operation/resource has a direct-core, capability-gated, local-only, or deferred disposition, shell mapping if meaningful, ownership rule, and named acceptance test. Include agent interaction requirements, readiness/completion observability, preview support, and retry behavior. Record resolved profile/lock designs and explicit platform blockers before expanding extraction. |
| 1. Profile/config and agent discovery | Workspace packages, namespaced secret/config repositories, owner lease, startup recovery, schema/capability discovery, safe server-list/status CLI, and a minimal read-only stdio host | An SDK client and shell harness discover and query the same extracted core without a desktop, HTTP, or controlling terminal. Real concurrent-process tests prove exclusion/recovery ordering. Dependency/build evidence excludes Tauri, GTK/WebKit, frontend assets/builds, and `tauri-build` from core/CLI. Existing desktop paths and credentials still work. |
| 2. Runtime families | Extract stores/events and PTY lifecycle, then tunnels/SSH/SCP/SFTP, Boundary/OpenVPN, and Caddy in independently reviewable increments | Each family has characterization coverage for desktop/CC contracts and direct CLI tests for readiness, failure, cancellation, ownership, and cleanup. Temporary Boundary tunnels support one-shot SSH/SCP. A family stays unavailable until its gate passes. |
| 3. Complete shell surface and backup | Stable output/exits, help/completion, safe secret input, foreground terminal behavior, mutation previews where supported, and local export/preview/apply | Test the agent command boundary, bounded responses, structured recovery guidance, explicit mutation choices, encrypted archive compatibility, profile namespace mapping, stale-preview rejection, and crash recovery. No duplicated backup engine or credential-format fork. |
| 4. Full stdio workflows and qualification | Extend the minimal host to extracted runtime families with local policy and bounded transport work | Official SDK plus representative real clients pass the complete agent loop and a stdio-specific matrix: versions/revisions, schemas, concurrency, cancellation, clean stdout, slow readers, broken pipe, and EOF/signal cleanup. Preserve desktop HTTP/REST behavior. |
| 5. Release/migration | Independently packaged artifacts and versioned docs/client examples for each supported target | Install in an environment without the desktop/GUI stack; native keyring/elevation/session smoke tests support each claimed capability. Publish checksums, prerequisite/minimum-platform requirements, qualification gaps, and helper migration guidance. |

> **Slice 0 Linux `x86_64` verification note:** The Slice 0 feasibility gate is verified on Linux `x86_64` in [STANDALONE_CLI_SLICE0.md](./STANDALONE_CLI_SLICE0.md) and [CLI_STDIO_SLICE0.tap](./evidence/CLI_STDIO_SLICE0.tap); Windows, macOS, and GUI-free Linux CI runner checks remain tracked for target-OS matrix gates.
>
> **Slice 1 Linux `x86_64` verification note:** Slice 1 (`scp-core`, `scp-mcp` standalone read-only/stdio adapter, `scpctl` read-only CLI & `mcp serve --stdio`, and shared `src-tauri` re-exports) is verified on Linux `x86_64` in [STANDALONE_CLI_SLICE1.md](./STANDALONE_CLI_SLICE1.md).
>
> **Slice 2 Linux `x86_64` verification note:** Slice 2 (`scp-core` `ProcessSupervisor`, `tty`, `EventSink`, `transfer_store`, `file_paths`, Boundary/SSH/SCP/SFTP, OpenVPN, and Caddy runtime families, `src-tauri` pure helper delegations, and the 9 direct `scpctl` runtime commands while keeping stdio MCP read-only until Slice 4) is verified on Linux `x86_64` in [STANDALONE_CLI_SLICE2.md](./STANDALONE_CLI_SLICE2.md).
>
> **Slice 3 Linux `x86_64` verification note:** Slice 3 (`scp-core::backup` `.scpbak` engine and `config_ops` mutations, `src-tauri` `settings_backup`/`validate_rdp_target` delegations, safe TTY/secret/passphrase readers, and the 14 direct `scpctl` Slice 3 commands across `completion`, server/config/settings/credential/sync-push mutations, `server rdp`, and `settings export`/`import`, keeping stdio MCP read-only until Slice 4) is verified on Linux `x86_64` in [STANDALONE_CLI_SLICE3.md](./STANDALONE_CLI_SLICE3.md).
>
> **Slice 4 Linux `x86_64` verification note:** Slice 4 (`scp-core` full headless runtime parity, `41/45` `Capability::Runtime` stdio MCP operations with `4` explicit `CAPABILITY_UNAVAILABLE` exclusions, live `stdio-policy.json` reload + fail-closed corruption handling, bounded concurrency/cancellation/framing/write-timeout transport controls, userspace `russh` `sshd_fixture_daemon` testing, and `@modelcontextprotocol/sdk`, `@modelcontextprotocol/client`, and `@modelcontextprotocol/inspector` CLI qualification) is verified on Linux `x86_64` in [STANDALONE_CLI_SLICE4.md](./STANDALONE_CLI_SLICE4.md).

Do not call a slice complete based only on mocked host tests. Record each gate's commands, exact commit/toolchain/SDK/client versions, OS/architecture, observed results, and unrun checks. A platform-specific capability may ship as explicitly unavailable; silently assuming parity is not an acceptable gate result. At least one supported target must pass a useful unattended workflow (discover server → establish a ready connection → transfer → inspect result → clean up); a catalog whose useful operations all require human interaction does not satisfy the primary outcome.

For the proposed package names, independent CI should run `cargo test -p scp-core -p scp-mcp -p scp-cli` and `cargo build --locked --release -p scp-cli` with only the CLI/stdio transport feature set selected. Record `cargo tree -p scp-cli -e normal,build` and repeat the build on a GUI-free runner; a desktop-enabled workspace build cannot prove isolation. Final commands depend on the selected workspace layout/features. Keep desktop regression builds as a separate job. Update README installation/usage, MCP/design docs, tested client launch examples, and the platform capability matrix before release.

The helper continues to control desktop-owned sessions; the standalone CLI deliberately cannot replace that use case. Do not deprecate it merely because similarly named shell commands exist. Any future desktop-control replacement requires its own documented migration and parity tests.

## Required acceptance tests

- Agent workflow tests cover both shell and persistent MCP clients with no controlling terminal. Execute discovery, capability checks, exact server selection, connection readiness, transfer completion, and cleanup using preconfigured credentials/policy. Assert no prompts, prose parsing, or desktop dependence; test also when the launcher allocates a PTY so noninteractive behavior cannot depend only on `isatty`.
- Machine-contract tests validate command schemas and JSON envelopes, bounded/stably ordered lists, truncation signals, exact ID selection, structured next actions/outcomes, finite deadlines, and preview/apply target changes. Fixtures include malicious-looking server descriptions/filenames as inert data, ambiguous names, unknown host keys, missing credentials, denied policy, owner conflicts, and a lost mutation response. Verify recovery queries rather than automatic mutation replay.
- CLI integration tests close/disable the desktop, exercise direct SSH/tunnel/transfer fixtures, assert exit codes and JSON output, and verify foreground commands retain usable owned runtimes until shutdown.
- Profile tests cover simultaneous initialization, path aliases, owner-lock conflicts including queries, cross-process config writer coordination, stable credential/recovery namespaces, cache isolation, port conflicts, and host-wide VPN conflicts. Verify recovery precedes any loader/cache/runtime side effect. Run desktop and CLI concurrently to verify CLI shutdown cannot stop desktop children or change desktop secrets.
- Boundary one-shot SSH/SCP tests cover tunnel readiness, timeout, transfer/connection failure, and cleanup. stdio calls may reuse tunnels owned by that host without borrowing another process's PTYs.
- Lifecycle tests distinguish request cancellation from accepted-operation lifetime, including cancellation at ownership registration; cover concurrent duplicate starts, already-stopped operations, stale-runtime identifiers and lost start responses. Verify bounded EOF/signal cleanup of owned sessions, transfers, browser handles, and children, including terminal restoration and output saturation. Record forced-termination limits and platform-specific elevated OpenVPN behavior.
- stdio subprocess tests assert every stdout line is valid protocol JSON, PTY/subprocess output never leaks into stdout, diagnostics use stderr, and protocol stdin is never used for secret prompts.
- Official SDK and representative real-client tests cover discovery/initialization for each claimed revision, schemas/results/errors, malformed input, concurrent calls, progress, cancellation, output backpressure, and shutdown. Verify schema/policy parity with desktop MCP while reporting host-specific capability differences.
- Sensitive calls fail closed without explicit local policy; unknown policies, imported policy escalation, indirect config/sync/git escalation, and caller-supplied authority are rejected. Resources/results/errors redact secrets and raw terminal output. stdio starts no MCP HTTP listener and performs no desktop-token discovery; operation-owned local tunnel listeners remain permitted by the relevant policy.
- CLI settings export/import tests preserve encrypted backup, preview, exact selected-section/credential actions, recovery and writer gates, portable identity rules, and safe passphrase input. Include desktop-export → CLI-import and CLI-export → desktop-preview fixtures, namespace preservation, rejection of noninteractive apply, and archive-file replacement after preview. Reuse existing backup coverage and add CLI-boundary tests rather than duplicating the backup engine.
- Cross-platform CI builds and exercises core/CLI independently of GUI libraries. Real OS-keyring/elevation/browser smoke tests supplement mocks, including locked/unavailable keyrings and noninteractive capability errors. Document unavailable checks rather than claiming they passed.
- Run the relevant new CLI/core suites plus existing desktop regressions. The current desktop baseline is `npx playwright test`, `npm run build`, and `cargo test --manifest-path src-tauri/Cargo.toml`; record environment-dependent failures. Do not assume a root `npm test` script or React/Vitest.

## Deferred scope and risks

No daemon, background cross-invocation CLI control, shared desktop/CLI profile, raw PTY MCP API, headless webview OIDC replacement, or duplicated CC identity/telemetry in v1. A functioning desktop approval queue remains separate; stdio uses explicit local policy and fails closed when interaction is unavailable. Backup merge, unattended restore apply, external-file bundling, telemetry-queue migration, and JSON event streaming remain outside this work.

The largest risks are hidden Tauri coupling, elevated Windows OpenVPN ownership, PTY telemetry/lifecycle hooks, credential isolation, and machine-wide VPN or external Boundary state. Staged extraction, explicit capabilities, and ownership tests must resolve these before parity claims. No fixed delivery estimate is justified until headless/platform spikes pass.

Use the protocol and SDK evidence maintained in [the Work 2 plan](DESKTOP_STREAMABLE_HTTP_PLAN.md), then validate stdio independently. Recheck pinned published SDK releases and actual client support at implementation time; upstream source support alone is insufficient.
