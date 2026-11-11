# Encrypted settings backup (desktop)

Implemented for Work 1 of [the combined plan](SETTINGS_BACKUP_STREAMABLE_HTTP_STANDALONE_CLI_PLAN.md). Work 2 (MCP transport) and Work 3 (standalone runtime/CLI) are separate changes.

## Using it

Save any edits first. Config → **Export settings** selects a `.scpbak` destination and requests a matching passphrase of at least 12 characters. Export reads the OS credential store directly and fails if an entry is locked, denied, or unreadable. A missing entry is recorded as absent. The result reports section/credential counts, never credential values. Keep the passphrase separately; the app cannot recover it.

Config → **Import settings** authenticates the archive, then shows a read-only preview. Choose shared settings, server records, device-local settings and/or configured passwords. Review set/delete/keep counts, referenced files, source executable configuration and paths. Preserve destination paths by default, or enter explicit destination overrides before previewing. Changing selections or overrides invalidates the preview. Replacement requires a separate confirmation; merge is not supported.

Stop PTYs, functions, authentication, transfers and reconnect attempts, and close remote browsers before committing. Disable MCP and CC. If CC has started in this process, restart after disabling it, so a disconnected reconnect loop cannot activate copied settings. Import does not terminate active work. Previews expire after ten minutes and are bound to a backend-held archive and destination revision; changing the destination requires another preview.

Portable restore generates a new MCP token and clears copied CC identity/token fields for re-enrollment. MCP and CC remain disabled. The optional **same-device recovery** selection preserves the original CC identity and MCP token, still disabled. These policies apply only when device-local settings are selected; unselected sections remain unchanged. Configure missing paths and external files before starting connections; importing settings alone does not make a new device connection-ready.

Export and preview can be cancelled. Native keyring reads run off the UI thread, have a 30-second per-read wait and a two-minute overall read-work budget. A native prompt itself may remain visible until the OS returns; retries refuse to spawn more native reads while one is outstanding. KDF and filesystem calls finish their bounded/current operation before cancellation is observed. Applying a restore is deliberately non-cancellable after mutation begins.

## Inventory and scope

The encrypted payload contains every field of the three JSON files, preserving unknown JSON fields and stable server IDs, with explicit absent-file declarations:

- `config.json`: shared settings, including any pre-migration legacy fields. Restore strips the five legacy plaintext credential fields before writing shared settings.
- `servers.json`: server records and services.
- `sync-local.json`: all device-local paths, MCP settings/token, CC configuration/identity/token and enrollment state.
- OS accounts in service `com.example.supercontrolpanel`: `openvpn_password`, `openvpn_otp_secret`, `boundary_password`, `boundary_oidc_password`, `boundary_oidc_otp_secret`, `boundary_app_token`, `cc-telemetry-queue-key`, and `server_ssh_password:{id}` for each snapshotted server ID. Each account has a value or explicit absence; the archive cannot choose arbitrary services/accounts.
- External references: executable/OVPN/CLI/sync/Caddy paths, `Caddyfile`, certificate/key references in the configured OVPN and extra arguments, and SSH identity references in Boundary SSH options. Referenced OVPN inspection is bounded; missing/unreadable OVPN dependencies are labelled unavailable. References are recorded, not copied. Inline keys and file contents are never bundled.

No additional persisted desktop UI preferences were found: the current desktop source does not use browser local/session storage for preferences. Backend runtime models, logs, queues, connections and scrollback are outside this backup. Boundary CLI credentials, OS/SSH-agent identities and orphaned keyring accounts for deleted servers are also outside scope; the portable keyring API cannot enumerate these.

Source Boundary app tokens and the telemetry queue key are encrypted in the archive but never restored by this importer. A change to the selected Boundary controller/auth identity explicitly deletes the destination app token, is listed in the preview, and requires fresh login; rollback protects that deletion. Boundary token validation/controller compatibility and stopped-telemetry queue-key compatibility have not been established, so their recovery is blocked. The destination telemetry key always remains intact; the destination Boundary app token is kept when its controller/auth identity stays the same. Queue backup/migration is a separate feature.

The exporter offers complete export only. It never silently substitutes cached/missing credentials for OS read failures and does not offer a partially successful archive as a complete backup. File presence and credential presence form the encrypted inventory; the preview adds included/absent/unselected section states and checks referenced paths on the destination.

## Format and boundaries

`src-tauri/src/settings_backup/` owns the feature. `Archive` and `Changes` are backend-only and are not IPC DTOs. Only redacted summaries, paths and executable configuration reach the frontend. No backup tool/resource is registered in MCP or CC. Passphrases cross the narrow Tauri commands, are not logged, and are wrapped in zeroizing buffers; input fields clear after IPC submission. Plaintext serialized payloads, derived keys, decrypted archives, staged settings and journal buffers are cleared where practical. JavaScript/IPC copies and allocator/OS copies cannot be guaranteed erased.

Version 1 consists of `SCPBAK01`, a little-endian 32-bit JSON-header length, the exact header bytes, and AES-256-GCM ciphertext/tag. The magic, length and header are associated data. The header declares format version, `argon2id-19`, `aes-256-gcm`, memory/iterations/parallelism, a fresh random 16-byte salt and 12-byte nonce. The encrypted payload has its own schema version, app version, platform, timestamp, file/credential inventory and external references. There is no compression or machine-bound decryption key.

Exports use Argon2id v19, 64 MiB, three iterations, one lane, deriving 32 bytes. Before running the KDF, imports enforce a 1 KiB header limit, a 32 MiB file limit, supported algorithms/version, fixed salt/nonce lengths, 8–64 MiB memory, 1–3 iterations and one lane. Decryption/authentication and schema, unique server-ID (ASCII letters/digits plus `-_.:`), account-map and size checks precede all mutations. Newer schema versions are rejected; v1 is the only currently supported schema. Future versions require explicit migrations.

Encrypted output is staged in the destination directory with private permissions, flushed, and atomically published, with explicit overwrite confirmation and final-symlink rejection. Unix files use mode 0600 and directory fsync. Windows uses a protected owner-rights DACL and write-through `MoveFileExW` replacement after file flush. Export refuses to overwrite the profile's settings/recovery files. Filesystem/device durability still depends on OS and storage behavior.

Enrollment/revocation requests retain a lightweight in-flight marker across network awaits, and reconnect workers acquire maintenance access before rechecking their generation and publishing a process. The process-wide maintenance gate permits concurrent ordinary operations and reentrant reads; export/preview/restore exclusively exclude those operations. It covers config loaders/migrations, config/server/credential writers, desktop/MCP/CC entry points, internal config sync, enrollment persistence and runtime start paths. Ordinary operations fail promptly while exclusive maintenance is active, instead of waiting while a runtime lock could be held. Maintenance workers wait at most five seconds for existing operations to finish before reporting contention. Runtime-store mutexes are not held across backup keyring prompts. Existing command/event names and payloads are unchanged. Revision counters plus a digest of destination settings/known credentials reject stale previews. The selected source archive stays in backend memory, so replacing the source file cannot substitute a different archive at commit.

This is a single-desktop-profile feature. Do not run another desktop instance or an external editor/sync script against the same profile during backup/import. Cross-process profile locking and shared-profile/headless ownership remain gates for Work 3; this change does not introduce shared-profile mode.

## Commit and crash recovery

Restore compiles omitted accounts into keep actions, selected present accounts into exact set actions, and selected absent accounts into delete actions. Replacing the server list also deletes credentials belonging to removed destination server IDs. Credentials-only server restore requires matching server ID sets. Normal save APIs and their empty-means-preserve behavior are not used.

Before the first settings mutation, the importer reads every required rollback credential from the OS and captures affected files as exact bytes. A read failure aborts mutation. It stores and re-reads a random recovery key in the dedicated OS account `settings-restore-recovery-v1`, outside the restore set and excluded from backups. It durably encrypts the undo snapshot into `settings-restore-journal.enc`. New file bytes are staged in memory. Credential persistence precedes file publication under the maintenance gate; the secret cache is refreshed only after commit or successful rollback.

The journal starts in an undo-required stage. After all mutations succeed, a durable committed marker switches recovery to retaining the new revision. Cleanup durably replaces the journal with a non-secret completion tombstone before unlinking it and deleting the recovery key; a tombstone requires no key if a filesystem later exposes it again. Cleanup failures are explicit.

On startup, recovery runs before normal config loading, plaintext migration, cache seeding or MCP/CC startup. An incomplete journal restores original files and credentials idempotently; a committed journal retains the imported revision. A locked/missing key, corrupt journal or failed rollback stops ordinary startup in a recoverable maintenance state. **Keep the journal and recovery key, restore/unlock OS-keyring access, then restart. Do not delete either to bypass recovery.** Failed in-process recovery blocks ordinary settings/runtime commands until restart. No affected runtime is silently restarted after import.

## Verification and platform qualification

Automated Rust tests cover authenticated/randomized round trips, wrong passphrases/tampering, KDF bounds, inventory validation, missing versus unavailable secrets, portable/same-device policy, remapping, server credential dependencies/deletions, redaction, stale-state fingerprints, gate exclusion, cancellation, owner-only Unix output/symlink handling, rollback-read failures, failed rollback/retry, interrupted and committed-stage recovery, and completion tombstones. Browser tests use real Config markup and a simulated Tauri boundary for passphrase confirmation, preview/confirmation, wrong-passphrase handling, invalidation, remapping, cancellation and UI refresh.

Run:

```sh
cargo test --manifest-path src-tauri/Cargo.toml
npx playwright test --config playwright.backup.config.ts
npm run build
```

The Windows file-I/O/ACL adapter was also cross-compiled for `x86_64-pc-windows-gnu` in an isolated type-check harness. Development verification was performed on Linux with fake credential stores for mutation/fault-injection tests, not a user's live OS credentials. Real OS unlock/denial scenarios, Windows ACL/replacement runtime behavior, macOS Keychain permissions and release-mode KDF timing on all three platforms still need native release qualification. The configured KDF suite takes about 13 seconds in this Linux debug build across repeated derivations; that is not a cross-platform release benchmark.

References: [RustCrypto Argon2 API](https://docs.rs/argon2/0.5.3/argon2/struct.Argon2.html), [temporary-file persistence](https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html), [Windows write-through moves](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw), [Windows owner-rights SID](https://learn.microsoft.com/en-us/windows/win32/secauthz/sid-strings).
