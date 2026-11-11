# Standalone operation feasibility ledger

Baseline: `ed2bda686`, Linux x86_64, 2026-11-11. Slice 0 is **in progress**. The shared registry is extracted; no standalone runtime handler, profile, or stdio transport is enabled. `schema --json` is metadata discovery only. The dispositions below are extraction targets, not capability promises. Existing desktop handlers and policy are unchanged.

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

## Profile and platform decisions still requiring evidence

Use the plan's canonical private profile directory and one OS-backed exclusive owner lease for all profile-backed commands, including queries. Acquire profile lease before metadata/recovery, then the process maintenance gate; never acquire these in reverse. Keep a stable random credential namespace outside portable settings and recover before any loader/cache/runtime side effect. Reject the desktop profile; do not infer desktop paths or copy its identity. Root workspace contains only independent packages; desktop remains an explicitly excluded package with its existing lockfile, build script and target paths. It consumes the shared registry through a path dependency. This avoids changing Tauri/CI/artifact paths during feasibility; combining workspaces is optional later, not required for sharing code.

Before implementing profile-backed commands, choose and validate native lock primitives and directory-alias handling on Windows/macOS/Linux with real concurrent processes. No profile path, keyring, lock, recovery or runtime implementation has been added in this increment. The proposal above is an acquisition contract, not proof of crash behavior.

| Platform spike | Evidence required before capability is enabled | Current blocker |
| --- | --- | --- |
| OS keyring | Native namespaced get/set/delete; locked/missing-store errors; destination recovery key isolation | Not run on any OS; headless Linux may lack Secret Service |
| Process supervisor | Child pipes/PTYS, cancellation registration race, bounded EOF/signal cleanup and reaping | Not extracted; desktop still owns its runtime |
| Unix VPN | Sudo interaction unavailable fails before spawn; routes readiness; crash cleanup; desktop plus CLI host-wide exclusion | No CLI VPN adapter or shared machine lease |
| Windows VPN | UAC helper participates in ownership; elevated child cleanup after parent crash | No Windows runner; no CLI elevation adapter |
| macOS runtime | Native keyring, PTY and elevation smoke | No macOS runner |
| Boundary external state | Isolated CLI token/config state or explicit sharing policy; browser/password capability checks | Existing CLI state sharing not characterized |
| stdio | Published SDK pin and independent client revision/framing qualification; saturation/EOF tests | Transport not added; HTTP SDK evidence alone is insufficient |

A family stays unavailable until its named tests and native prerequisites pass. Useful unattended network workflow qualification, GUI-free runner, Windows/macOS packaging and all Slice 1 gates remain open. The discovery preview is not a completed Slice 0 or a standalone runtime release.
