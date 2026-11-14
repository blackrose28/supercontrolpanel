# Standalone operation feasibility ledger

Baseline: `ed2bda686` (Slice 0) updated for **Slice 1** on Linux `x86_64`. In Slice 1, the 11 read-only operations and 6 resources (`app_get_overview`, `function_status_list`, `function_status_get`, `openvpn_get_status`, `boundary_get_status`, `servers_list`, `server_get`, `config_get_paths`, `sync_settings_get`, plus category-gated `scp_transfer_list` and `scp_transfer_get`) and read-only CLI inspection/stdio serving are implemented in `scp-core`, `scp-mcp`, and `scpctl`. All mutating operations and external runtime workers remain `runtime_not_extracted` until Slices 2–4. Existing desktop handlers and policy remain unchanged via shared re-exports.

## Operation dispositions

Core = candidate direct-core operation after its gate; gated = requires platform/interaction qualification; deferred = unavailable in v1. All candidates remain `runtime_not_extracted` today. Shell mappings describe intended commands; a dash means persistent MCP only or no v1 mapping. Names for future acceptance tests identify required work, not tests already passing.

| Operation | Disposition | Shell mapping | Ownership / interaction / observability / preview / retry | Required acceptance gate (planned) |
| --- | --- | --- | --- | --- | --- |
| `boundary_search_targets` | Gated | boundary targets search | Query; preconfigured isolated CLI auth; bounded results; no preview; retry read after auth setup | `boundary_external_state_isolation` |
| `detect_openvpn_executable` | Gated | openvpn detect | Query; explicit platform executable lookup; no prompts; no preview; safe read retry | `platform_executable_discovery` |
| `server_rdp_start` | Gated | server rdp (interactive only) | Host-owned native launch; explicit interaction; client spawn is not connection readiness; no preview; inspect before retry | `native_rdp_capability_and_ownership` |
| `server_ssh_password_set` | Gated | server credentials set / clear | Mutation; namespaced OS store; safe explicit input; completion after keyring write; no preview; inspect destination before retry | `namespaced_keyring_no_desktop_access` |
| `server_ssh_password_clear` | Gated | server credentials set / clear | Mutation; namespaced OS store; safe explicit input; completion after keyring write; no preview; inspect destination before retry | `namespaced_keyring_no_desktop_access` |
| `app_get_overview` | Core | status (new invocation only) | Query; no prompts; process-local snapshot; no preview; safe read retry | `owned_status_snapshot` |
| `function_status_list` | Core | status (new invocation only) | Query; no prompts; process-local snapshot; no preview; safe read retry | `owned_status_snapshot` |
| `function_status_get` | Core | status (new invocation only) | Query; no prompts; process-local snapshot; no preview; safe read retry | `owned_status_snapshot` |
| `openvpn_get_status` | Core | status (new invocation only) | Query; no prompts; process-local snapshot; no preview; safe read retry | `owned_status_snapshot` |
| `boundary_get_status` | Core | status (new invocation only) | Query; no prompts; process-local snapshot; no preview; safe read retry | `owned_status_snapshot` |
| `servers_list` | Core | server list / get with exact ID | Query; no credential unlock; bounded stable results; no preview; safe read retry | `profile_safe_server_queries` |
| `server_get` | Core | server list / get with exact ID | Query; no credential unlock; bounded stable results; no preview; safe read retry | `profile_safe_server_queries` |
| `config_get_paths` | Core | config paths / settings show | Query; redacted selected-profile paths/settings; no preview; safe read retry | `profile_safe_settings_queries` |
| `openvpn_start` | Gated | openvpn start --foreground | Host-owned; explicit elevation capability; route readiness; no preview; inspect before retry; global VPN lease | `vpn_elevated_ownership_cleanup` |
| `openvpn_stop` | Gated | — | Stop only this host VPN; no new interaction; await reaping; no preview; stopped known ID succeeds | `vpn_stop_rejects_foreign_owner` |
| `boundary_auth_start` | Gated | boundary auth --interactive | Host-owned; CLI auth only after native validation; hidden-webview OIDC unavailable; auth completion; no preview; inspect before retry | `boundary_auth_capability_cleanup` |
| `boundary_auth_stop` | Gated | — | Stop this host auth; no new interaction; await reaping; no preview; stopped known ID succeeds | `boundary_auth_owned_stop` |
| `server_tunnel_start` | Core | server tunnel start --server-id ID --foreground | Host-owned; preconfigured credentials; socket plus usable connection readiness; no preview; reserve exact function/port; inspect before retry | `tunnel_duplicate_start_readiness_cleanup` |
| `server_tunnel_stop` | Core | — | Stop this host tunnel after dependent browser/transfers/SSH; no prompts; reap; no preview; stopped known ID succeeds | `tunnel_dependency_shutdown` |
| `server_ssh_start` | Gated | server ssh --server-id ID --interactive | Host-owned terminal; explicit interaction; authenticated readiness; no JSON/raw PTY MCP; no preview; inspect before retry | `ssh_terminal_restore_and_owner_race` |
| `server_ssh_stop` | Core | — | Stop owned SSH; no prompts; reap; no preview; stopped known ID succeeds | `ssh_owned_stop` |
| `caddy_start` | Core | caddy start --foreground | Host-owned; validate executable/listener; socket readiness; no preview; inspect before retry | `caddy_listener_ownership_cleanup` |
| `caddy_stop` | Core | — | Stop owned Caddy; no prompts; reap; no preview; stopped known ID succeeds | `caddy_owned_stop` |
| `server_upsert` | Core | server / config mutation with exact target | Mutation; profile writer/recovery gate; no implicit prompts; atomic completion; advisory preview and state recheck; inspect before retry | `config_preview_policy_namespace_protection` |
| `server_delete` | Core | server / config mutation with exact target | Mutation; profile writer/recovery gate; no implicit prompts; atomic completion; advisory preview and state recheck; inspect before retry | `config_preview_policy_namespace_protection` |
| `config_save` | Core | server / config mutation with exact target | Mutation; profile writer/recovery gate; no implicit prompts; atomic completion; advisory preview and state recheck; inspect before retry | `config_preview_policy_namespace_protection` |
| `config_save_partial` | Core | server / config mutation with exact target | Mutation; profile writer/recovery gate; no implicit prompts; atomic completion; advisory preview and state recheck; inspect before retry | `config_preview_policy_namespace_protection` |
| `sync_settings_get` | Core | config paths / settings show | Query; redacted selected-profile paths/settings; no preview; safe read retry | `profile_safe_settings_queries` |
| `sync_settings_save_partial` | Core | server / config mutation with exact target | Mutation; profile writer/recovery gate; no implicit prompts; atomic completion; advisory preview and state recheck; inspect before retry | `config_preview_policy_namespace_protection` |
| `remote_browser_open` | Core | — | Host-owned SFTP; no hidden trust/password prompt; existing ready tunnel; accepted browser ID; no preview; inspect before retry | `sftp_owned_session_lifecycle` |
| `remote_browser_close` | Core | — | Stop owned browser; no prompts; close persistent SFTP; no preview; stopped known ID succeeds | `sftp_close_on_dependency_stop` |
| `remote_browser_list_children` | Core | — | Query owned browser; inert remote data; bounded listing/stat; no preview; safe read retry while session exists | `sftp_bounded_untrusted_metadata` |
| `remote_browser_stat_path` | Core | — | Query owned browser; inert remote data; bounded listing/stat; no preview; safe read retry while session exists | `sftp_bounded_untrusted_metadata` |
| `remote_browser_create_directory` | Core | — | Request-owned mutation; exact path/recursive choice; no prompts; completion or partial outcome; preview/recheck; inspect remote state before retry | `sftp_mutation_preview_cancel_partial` |
| `remote_browser_delete_path` | Core | — | Request-owned mutation; exact path/recursive choice; no prompts; completion or partial outcome; preview/recheck; inspect remote state before retry | `sftp_mutation_preview_cancel_partial` |
| `scp_upload` | Core | scp upload / download --server-id ID | Request-owned; no trust/password/overwrite prompts; temporary ready Boundary tunnel as needed; await transfer and cleanup; no preview; inspect partial destination before retry | `scp_one_shot_ready_transfer_cleanup` |
| `scp_download` | Core | scp upload / download --server-id ID | Request-owned; no trust/password/overwrite prompts; temporary ready Boundary tunnel as needed; await transfer and cleanup; no preview; inspect partial destination before retry | `scp_one_shot_ready_transfer_cleanup` |
| `scp_start_upload` | Core | — (shell uses completed transfer) | Host-owned async; preconfigured credentials/trust; accepted session-to-transfer IDs then status completion; no preview; inspect before retry | `scp_ownership_registration_cancel_race` |
| `scp_start_download` | Core | — (shell uses completed transfer) | Host-owned async; preconfigured credentials/trust; accepted session-to-transfer IDs then status completion; no preview; inspect before retry | `scp_ownership_registration_cancel_race` |
| `scp_transfer_list` | Core | — (separate invocation has no history) | Query this host transfers; no prompts; bounded status snapshots; no preview; safe read retry | `scp_status_runtime_id_isolation` |
| `scp_transfer_get` | Core | — (separate invocation has no history) | Query this host transfers; no prompts; bounded status snapshots; no preview; safe read retry | `scp_status_runtime_id_isolation` |
| `config_sync_pull` | Deferred | — | Protected import unavailable; no side effect or automatic retry; local backup import is separate | `protected_import_stays_unavailable` |
| `config_sync_push` | Core | config sync push | Mutation; reviewed destination and isolated config; no prompts; completion; preview/recheck; inspect repo before retry | `sync_push_cannot_export_local_policy` |
| `config_git_pull` | Deferred | — | Reviewed Git unavailable; no side effect or automatic retry; policy cannot be escalated indirectly | `reviewed_git_stays_unavailable` |
| `config_git_push` | Deferred | — | Reviewed Git unavailable; no side effect or automatic retry; policy cannot be escalated indirectly | `reviewed_git_stays_unavailable` |

## Resource mappings

All six reuse their operation's policy/redaction, owner lease, bounds, retry and snapshot acceptance gate. Reading a resource never reaches a different process's runtime.

| URI | Operation |
| --- | --- |
| `scp://status/functions` | `function_status_list` |
| `scp://status/openvpn` | `openvpn_get_status` |
| `scp://status/boundary` | `boundary_get_status` |
| `scp://servers/list` | `servers_list` |
| `scp://config/paths` | `config_get_paths` |
| `scp://status/transfers` | `scp_transfer_list` |

## Local CLI-only command workflows

These top-level commands run locally in `scpctl` rather than as desktop MCP tools. `scpctl schema` (`--json`, `--command`), `capabilities`, `status`, `server list/get`, `config paths`, `settings show`, and read-only `mcp serve --stdio` are implemented in Slice 1 (`STANDALONE_CLI_SLICE1.md`). `settings export`/`import` remain planned for Slice 3.

| CLI workflow | Extraction disposition | Ownership rule | Interactive / noninteractive rule | Readiness, preview & retry behavior | Named future acceptance gate |
| --- | --- | --- | --- | --- | --- |
| `scpctl schema` | Implemented Slice 1 (`--json`, `--command <CMD>`) | Profile-free; zero lock, keyring, config, or runtime side effects | Always noninteractive; unknown flags/commands/extra args exit 2 | Reflects Slice 1 11 read-only ops + 6 resources live and 34 runtime/mutation ops unavailable; idempotent | `cli_schema_command` + Slice 1 `schema_command_detail` |
| `scpctl capabilities` | Implemented Slice 1 (local CLI-only) | Acquires selected profile `owner.lock` unless `--no-profile`; rejects desktop profile (`POLICY_DENIED` exit 3) | Noninteractive; probes OS keyring lock state without `gcr-prompter` | Reports OS/keyring state, profile namespace, `stdio-policy.json`, and available/unavailable families cleanly | `cli_capabilities_reports_os_gates` |
| `scpctl settings export` | Slice 3 planned (local CLI-only; reuses desktop encrypted archive format) | Acquires profile `owner.lock` + credentials read; isolated from desktop `config.json` and keyring | Requires explicit passphrase input mechanism (fd/env/prompt), fails fast in `--non-interactive` if absent | Bounded archive export with metadata/namespace manifest; safe retry on write error | `cli_settings_export_cross_compat` |
| `scpctl settings import --preview` | Slice 3 planned (local CLI-only; zero-mutation inspection) | Acquires shared profile `owner.lock`; never mutates files or OS keyring | Requires passphrase input; rejects missing/invalid passphrase noninteractively | Emits exact section/credential diff and target archive digest; idempotent retry | `cli_settings_import_preview_no_mutation` |
| `scpctl settings import --apply --interactive` | Slice 3 planned (local CLI-only; absent from MCP in v1) | Acquires profile `owner.lock`, executes transactional restore & crash recovery journal | Fails closed when noninteractive or without explicit TTY/confirmation after matching preview | Validates preview digest (rejects stale/replaced archive), transactional rollback/journal recovery on crash | `cli_settings_import_apply_recovers_and_rejects_stale_preview` |
| `scpctl mcp serve --stdio` | Implemented Slice 1 (read-only over `rmcp 3.9.0`; Slice 4 extends to workers) | Acquires single-owner `owner.lock` for process lifetime; exits `5` on concurrent owner; reaps cleanly on EOF | Never prompts on stdin/UI or emits shell JSON on stdout; policy read from local `stdio-policy.json` | Exposes 9 default read-only tools (+2 transfer queries when `"file-transfer"` allowed) and 6 resources over NDJSON stdout; diagnostics strictly on stderr | `stdio_host_end_to_end_agent_loop` |

## Profile and platform decisions and Linux Slice 0 spike evidence

Use the plan's canonical private profile directory and one OS-backed exclusive owner lease for all profile-backed commands, including queries. Acquire profile lease before metadata/recovery, then the process maintenance gate; never acquire these in reverse. Keep a stable random credential namespace outside portable settings and recover before any loader/cache/runtime side effect. Reject the desktop profile (including canonicalized symlink/`..` aliases); do not infer desktop paths or copy its identity. Root workspace contains only independent packages; desktop remains an explicitly excluded package with its existing lockfile, build script, and target paths. It consumes the shared registry through a path dependency. `experiments/cli-stdio-slice0` holds the disposable Slice 0 platform and `stdio` verification binary, excluded from the root workspace.

| Platform spike | Evidence required before capability is enabled | Linux x86_64 Slice 0 status & open OS matrix gates |
| --- | --- | --- |
| Profile lease & desktop alias rejection | Native `flock` (`LOCK_EX \| LOCK_NB`, Unix) / `LockFileEx` (Windows) on `owner.lock` kept un-unlinked, canonical symlink/`..` rejection of desktop config dir, immediate kernel release on `SIGKILL` | **Verified on Linux x86_64** in `experiments/cli-stdio-slice0` (`profile_lock_rejects_desktop_alias_and_enforces_exclusive_lease`: exit 5 `profile_owner_conflict`, diagnostics PID preserved, instant unlock after `SIGKILL`); Windows & macOS untested |
| OS keyring | Native namespaced get/set/delete without UI prompter hangs; `keyring_locked` / `keyring_unavailable` fast preflight; desktop + cross-profile isolation | **Verified on Linux x86_64** in isolated `dbus-run-session` + `gnome-keyring-daemon` (`dbus-secret-service` `Collection::is_locked()` check prevents `gcr-prompter` hang; `profile:{ns}:{account}` preserves desktop `com.example.supercontrolpanel` secrets & recovery keys); GUI-free CI headless runner without Secret Service, Windows & macOS untested |
| Process supervisor | Child pipes/PTYs (`portable-pty 0.8.1`), cancel-before-accept race cleanup, bounded 64 KiB output ring, EOF/`SIGTERM` reaping | **Verified on Linux x86_64** (`supervisor_enforces_cancel_race_noise_isolation_and_eof_sigterm_reaping`: cancel race reaps child, >256 KiB pipe noise bounded without deadlock, zero stdout leak & both PTY+pipe children reaped on `stdin` EOF and `SIGTERM`); Windows ConPTY & macOS untested |
| Unix VPN | `sudo -n true` noninteractive preflight before spawn, `terminateOpenVpnWhenHelperExits` rejection, host-wide `vpn.lock` contention | **Preflight & lock verified on Linux x86_64** using injected fake `sudo`/`openvpn` scripts (`unix_vpn_preflight_and_host_wide_lease`: fails cleanly before spawn with `elevation_interaction_required` / `elevation_unavailable` / `incompatible_vpn_ownership`, exit 5 `vpn_owner_conflict`); live privileged `openvpn` route tunnel remains a Slice 2/5 gate |
| Windows VPN | UAC helper participates in ownership; elevated child cleanup after parent crash | **Open**: No Windows runner; no CLI elevation adapter yet |
| macOS runtime | Native keyring (`SecKeychain`/Data Protection), PTY, and elevation smoke | **Open**: No macOS runner in this environment |
| Boundary external state | Isolated `BOUNDARY_SKIP_CACHE_DAEMON=true` + `BOUNDARY_TOKEN_NAME=scpctl-{ns}` / injected env `BOUNDARY_KEYRING_TYPE=none` + capability gates | **Verified on Linux x86_64** against real `/usr/bin/boundary 0.19.1` (`boundary_cli_isolation_and_capabilities`: `AutomatedOidc` -> `webview_oidc_desktop_only`, `BrowserOidc` -> `browser_oidc_unverified` gated, D-Bus Secret Service `profile=scpctl-{ns}` isolated); Windows/macOS CLI keyring stores untested |
| stdio | `rmcp 3.9.0` (`stdio_pipes` + `AsyncRwTransport`) tested with `@modelcontextprotocol/sdk 1.32.2` (`2025-11-25`), `@modelcontextprotocol/client 2.4.0` (`2026-07-28`), and `@modelcontextprotocol/inspector 2.14.0`; EOF/writer-stall/noise/error recovery | **Verified on Linux x86_64** (`npm test --prefix experiments/cli-stdio-slice0`: 10/10 passed, TAP saved in `docs/mcp/evidence/CLI_STDIO_SLICE0.tap`; characterized `rmcp` 5s `QuitReason::Closed` drain and bounded via `EofWatchReader` + 150ms grace and `TimedAsyncWriter`); Windows/macOS untested |

A family stays unavailable in production `scpctl` until its production Slice 1–4 implementation, named tests, and target-OS native prerequisites pass. Useful unattended network workflow qualification, GUI-free Linux runner verification, Windows/macOS runners/packaging, interactive Browser OIDC validation, and Slices 1–5 remain open.
