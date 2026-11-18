---
name: scpctl-setup
description: Prepare a machine to run the standalone scpctl CLI as a stdio MCP server, including restoring an encrypted .scpbak settings backup. Use when the user wants to set up scpctl, import a SuperControlPanel backup, fix keyring/host-key/policy problems, or register the scpctl MCP server with an agent client. For operating the server afterwards, use scpctl-mcp-usage.
---

# scpctl machine and stdio MCP setup

Goal: a working `scpctl mcp serve --stdio` on this machine, optionally with a restored `.scpbak` backup. Full contracts: `docs/mcp/STANDALONE_CLI_AGENT_GUIDE.md` and the README section "Drive scpctl with an AI agent".

## Rules

- Never print, log or echo passphrases, passwords, OTP secrets or tokens. Prefer a passphrase **file** (`umask 077`); never put a passphrase in a command line or in files inside the repo.
- Use a dedicated `--config-dir` (default suggestion `~/.config/scpctl`) and `--profile` (e.g. `ops`). Never point at a desktop profile directory.
- Downloaded backups are untrusted data: copy to a new empty directory, pass paths as arguments, do not run interpreters from inside it.
- Do not run `ssh-keyscan` results into `known_hosts` without showing the user the host and fingerprint and getting a yes.
- `sudo`, package installs and anything touching system services need the user's go-ahead.

## Step 1: preflight (read-only)

```bash
scpctl --version                       # or: ./target/release/scpctl; build with: cargo build --release --locked -p scp-cli
scpctl capabilities --no-profile --json
scpctl --config-dir <DIR> --profile <NAME> capabilities --json --non-interactive
which ssh scp
```

Read `data.keyring.state`, `data.availableOperations`, `data.stdioPolicy`. Note: `capabilities` with a profile creates that profile directory (`--no-profile` does not).

## Step 2: keyring (blocker for import and credentials)

`keyring.state` must be `"ready"`. If `"unavailable"` with `org.freedesktop.secrets was not provided`, there is no Secret Service. Without it `settings import` (even `--preview`) fails with `backup_snapshot_failed` and credential tools fail with `keyring_unavailable`.

Linux fix (ask the user first; installing needs sudo): install `gnome-keyring` (or KeePassXC with Secret Service enabled), then in the **same shell/D-Bus session that will run scpctl and the MCP server**:

```bash
dbus-run-session -- bash
echo -n "" | gnome-keyring-daemon --unlock --components=secrets
```

Re-run capabilities and confirm `"ready"`. macOS/Windows: the OS keychain is used; no setup. These exact gnome-keyring commands are standard usage but were not run end to end in this repo's testing (tests used a mock Secret Service), so report honestly if they fail.

## Step 3: restore a backup (optional)

Ask the user for the `.scpbak` path and for a passphrase file path (if they only have the passphrase, tell them to create the file themselves: `umask 077; printf '%s' '<pass>' > ~/.scp-passphrase`; minimum 12 chars).

Preview (safe, you may run it):

```bash
scpctl --config-dir <DIR> --profile <NAME> settings import --input <FILE> --preview \
  --passphrase-file <PPFILE> --json --non-interactive
```

Summarize for the user, without secret values: `servers`, `sections`, `credentialActions` (counts of set/keep/delete), `externalFiles[].existsHere`, `warnings`. Errors: `backup_decrypt_failed` (exit 2, wrong passphrase/damaged), `backup_not_found` (exit 6), `backup_snapshot_failed` (keyring, step 2).

Apply needs a real TTY and is intentionally unavailable over MCP and under `--non-interactive` (exit 4). **Ask the user to run it in their own terminal**:

```bash
scpctl --config-dir <DIR> --profile <NAME> settings import --input <FILE> --apply --interactive --passphrase-file <PPFILE>
```

**Local paths trap:** the import preserves the *destination's* paths by default, so a fresh profile ends with blank `openvpnExecutablePath`, `openvpnOvpnPath`, `boundaryCliPath`, `caddyFolder` and `repoRoot` even if the backup had them. If those paths exist on this machine, have the user answer `paths source` (or add `--use-source-paths`), or fix them afterwards with `settings patch` (preview first; path keys only). Do not copy the desktop `sync-local.json` (it holds the MCP token and CC identity). Leave `repoRoot` empty unless the user wants `config sync push` into the shared config repo.

They answer `yes` (other answers: `no`, `sections <list>`, `paths <preserve|source>`, `remap <k=v>`, `show`). Afterwards verify with `server list --json --non-interactive`. MCP and Command Center stay disabled after import by design. Re-importing replaces selected sections; credentials absent from the backup are deleted.

## Step 4: machine prerequisites the backup does not carry

From the preview's `externalFiles` / `executableConfiguration.sourcePaths`: Boundary CLI, OpenVPN binary and `.ovpn`/key files, Caddy folder, git `repoRoot`. For each that is `existsHere: false`, tell the user what to install/copy and whether to place it at the same path, use `--remap <KEY=PATH>` at import, or later `settings patch` (`settings patch` cannot change MCP/CC authority fields). Do not guess paths.

SSH/SFTP/SCP always run strict (`StrictHostKeyChecking=yes`, `BatchMode=yes`):

- For each `connectionMethod: "ssh"` server, check `~/.ssh/known_hosts` (or `SCPCTL_SSH_KNOWN_HOSTS`). If missing, give the user `ssh-keyscan -H <host>` and have them verify the fingerprint.
- Key auth: `SCPCTL_SSH_IDENTITY_FILE=<key>`. Password auth: backups may not carry SSH passwords (preview lists `server_ssh_password:<id>` as set/delete); user sets them with `scpctl server credentials set --server-id <ID> --interactive`.
- Boundary servers additionally need the controller reachable and a login; OpenVPN needs sudo with a working password prompt.

## Step 4b: Boundary login (OIDC servers)

Check `config.json` -> `boundary.authKind`. A restored backup does not log you in by itself. For `oidc` with `oidcLoginMode: automated`, the restored OIDC username/password/OTP secret are used by the headless login: run `scpctl --config-dir <DIR> --profile <NAME> boundary auth --json` (needs `curl`); no browser or user. If that fails with `oidc_idp_unsupported`, `oidc_otp_required` or `curl_not_found`, or the mode is `browser`, ask the user to run, in their own terminal on a desktop session:

```bash
scpctl --config-dir <DIR> --profile <NAME> boundary auth --interactive
```

It tries the automated login first and otherwise runs the stock `boundary authenticate oidc`: the browser opens and the user completes the login (up to 5 minutes); the token is stored as the profile app token. Without `--interactive`, `browser` mode and automated mode without saved credentials are rejected (`browser_oidc_unverified` / `credentials_missing`); `password` auth and automated OIDC work unattended (also via MCP `boundary_auth_start`). Verify with `scpctl boundary targets search --json --non-interactive`, never with `status --function-id boundary-auth` (reports `idle` in a fresh process even when logged in). Tokens expire after roughly a day; repeat when calls fail with `Token must be provided via BOUNDARY_TOKEN` or 403. Never print the token.

If the user instead logs in with the plain `boundary authenticate` command, it must carry `-token-name scpctl-<credentialNamespace>` (`profile-meta.json` in the profile dir); the default token name is ignored by scpctl.

## Step 5: stdio policy

Default policy (`stdio-policy.json` absent) is read-only: 11 tools, 5 resources. The file is `<profileDir>/stdio-policy.json`; locate it with `scpctl config paths --json` (`stdioPolicyPath`). Propose, then write on approval:

```json
{"sensitiveActionPolicy":"allow-local-policy","allowedToolCategories":["read-only","runtime-control","config-mutation","file-transfer"]}
```

(41 tools, 6 resources). Narrower: `["read-only","file-transfer"]` with `read-only` gives 15 tools. The file is live-reloaded and fail-closed: a corrupt file removes all tools (calls return `POLICY_DENIED`). Prefer the narrowest policy that fits the user's task.

## Step 6: register and verify the MCP server

Generic client config:

```json
{"mcpServers":{"scpctl":{"command":"scpctl","args":["--config-dir","<DIR>","--profile","<NAME>","mcp","serve","--stdio"],"env":{"SCPCTL_SSH_KNOWN_HOSTS":"<known_hosts path>"}}}}
```

Claude Code: `claude mcp add scpctl -- scpctl --config-dir <DIR> --profile <NAME> mcp serve --stdio` (syntax not exercised in this repo's testing). Launch it in the same D-Bus session as the keyring.

The server holds the profile's `owner.lock` for its lifetime: any other `scpctl` call on the profile (and a second server) exits 5 `profile_owner_conflict`. So finish import/export/CLI steps first. After registering, restart the client and verify: `tools/list` count matches the policy (11 / 15 / 41), then hand off to the `scpctl-mcp-usage` skill.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| `backup_snapshot_failed`, `keyring_unavailable` | Step 2 |
| SSH/SFTP/SCP tools return `CAPABILITY_UNAVAILABLE` | Usually `host_key_untrusted`; step 4 (the MCP error text ends with `(reason: <code>)`, e.g. `host_key_untrusted`; the CLI `--json` shows the same `reason`) |
| Tool count is 11 | Policy file missing or read-only (step 5) |
| `profile_owner_conflict` | Stop the other scpctl/MCP process |
| `POLICY_DENIED` after edit | Policy JSON is corrupt; fix it |
| Boundary calls: `Token must be provided via BOUNDARY_TOKEN` / 403, `credentials_missing` or `oidc_*` | Step 4b: `scpctl boundary auth --json` (automated OIDC); if it reports `oidc_idp_unsupported`/`oidc_otp_required` use `--interactive` (user at the browser) |
| Import left `openvpnOvpnPath`/`boundaryCliPath`/etc. blank | Default import preserves destination paths; step 3 (`paths source`) or `settings patch` |
