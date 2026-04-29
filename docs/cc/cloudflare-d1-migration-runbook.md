# cc-server SQLite to Cloudflare D1 Migration Runbook

## Purpose

This runbook describes how to move `cc-server` data from the existing normal-server SQLite database into Cloudflare D1 for the Cloudflare Worker deployment.

This is part of the dual-runtime migration. It does **not** remove or replace the existing NestJS + SQLite deployment path.

## Scope

Current SQLite source:

- Normal-server backend database, usually `cc-server/data/cc-server.sqlite`
- Or the path set by `CC_DB_PATH`

Cloudflare destination:

- D1 database bound as `CC_DB` in `cc-server/wrangler.toml`

Tables covered:

- `devices`
- `telemetry_logs`
- `device_runtime_state`
- `admin_sessions`
- `admin_api_tokens`
- `commands`
- `command_audit`
- `pty_sessions`
- `pty_audit`
- `file_transfers`
- `remote_browser_sessions`
- `rate_limit_buckets`
- `admin_login_failures`
- `admin_totp`
- `admin_totp_recovery`
- `admin_audit_log`

## Recommended migration strategy

Use a maintenance window and one-way snapshot import:

1. Stop writes to the normal-server backend.
2. Back up the SQLite file.
3. Export schema/data from SQLite to SQL.
4. Apply the Worker/D1 schema.
5. Import data into D1.
6. Validate counts and critical records.
7. Start Worker deployment against D1.
8. Keep the original SQLite backup for rollback.

Do not run both runtimes in active-active write mode against separate databases unless a later synchronization design is implemented.

## Prerequisites

- Cloudflare account and project access.
- Wrangler installed and authenticated.
- D1 database created.
- `wrangler.toml` updated with the real D1 `database_id`.
- Existing normal-server SQLite database is healthy.
- Backend test suite passes before export.

Validation before starting:

```bash
cd cc-server
npm test
npm run build
```

## Step 1 — Find the source SQLite database

Default path:

```text
cc-server/data/cc-server.sqlite
```

If `CC_DB_PATH` is set, use that path instead:

```bash
echo "$CC_DB_PATH"
```

## Step 2 — Stop writes

Stop the normal-server `cc-server` process or put the system into maintenance mode so no new device telemetry, commands, sessions, PTY events, or transfer updates are written during export.

## Step 3 — Back up SQLite

Example:

```bash
mkdir -p backups
cp data/cc-server.sqlite "backups/cc-server-$(date -u +%Y%m%dT%H%M%SZ).sqlite"
```

If WAL files exist, checkpoint first when possible:

```bash
sqlite3 data/cc-server.sqlite 'PRAGMA wal_checkpoint(FULL);'
```

Then copy:

```bash
cp data/cc-server.sqlite backups/cc-server.sqlite
```

## Step 4 — Create/apply D1 schema

The Worker-side schema source lives in:

```text
cc-server/src/worker/db/d1-schema.ts
```

For production migration, convert `D1_SCHEMA_SQL` into a plain SQL migration file, for example:

```text
cc-server/migrations/0001_initial.sql
```

Then apply locally first:

```bash
cd cc-server
npx wrangler d1 migrations apply cc-server --local
```

Apply to remote when validated:

```bash
npx wrangler d1 migrations apply cc-server --remote
```

If migrations are not yet wired, use `wrangler d1 execute` with a generated schema SQL file:

```bash
npx wrangler d1 execute cc-server --remote --file migrations/0001_initial.sql
```

## Step 5 — Export data from SQLite

Use SQLite dump for data-only export after schema is already applied to D1.

Example:

```bash
sqlite3 data/cc-server.sqlite .dump > backups/cc-server-full-dump.sql
```

For a D1 import, prefer a cleaned data-only file that excludes SQLite pragmas and `CREATE TABLE` statements if schema was already applied.

Example starting point:

```bash
sqlite3 data/cc-server.sqlite ".mode insert" ".output backups/cc-server-data.sql" ".dump"
```

Review the generated SQL before importing. Remove or adjust statements that D1 should not execute, such as:

- `PRAGMA` lines
- transaction wrappers if incompatible with the chosen import command
- duplicate `CREATE TABLE` statements when schema already exists
- SQLite-specific internals such as `sqlite_sequence`, unless intentionally preserving AUTOINCREMENT state

## Step 6 — Import into D1

Local dry run first:

```bash
npx wrangler d1 execute cc-server --local --file backups/cc-server-data.sql
```

Remote import after local validation:

```bash
npx wrangler d1 execute cc-server --remote --file backups/cc-server-data.sql
```

Large databases may need chunking by table. Import parent tables before dependent tables:

1. `devices`
2. admin/session/token/TOTP/rate-limit tables
3. telemetry/runtime tables
4. command and command audit tables
5. PTY session and PTY audit tables
6. file transfer and remote browser tables

## Step 7 — Validate D1 data

Compare counts between SQLite and D1 for each table.

SQLite example:

```bash
sqlite3 data/cc-server.sqlite 'SELECT COUNT(*) FROM devices;'
sqlite3 data/cc-server.sqlite 'SELECT COUNT(*) FROM telemetry_logs;'
sqlite3 data/cc-server.sqlite 'SELECT COUNT(*) FROM commands;'
```

D1 example:

```bash
npx wrangler d1 execute cc-server --remote --command 'SELECT COUNT(*) AS count FROM devices;'
npx wrangler d1 execute cc-server --remote --command 'SELECT COUNT(*) AS count FROM telemetry_logs;'
npx wrangler d1 execute cc-server --remote --command 'SELECT COUNT(*) AS count FROM commands;'
```

Critical validation queries:

```sql
SELECT id, name, status, os FROM devices ORDER BY lastSeenAt DESC LIMIT 10;
SELECT commandId, deviceId, state, commandType FROM commands ORDER BY submittedAt DESC LIMIT 10;
SELECT transferId, deviceId, state, direction FROM file_transfers ORDER BY updatedAt DESC LIMIT 10;
SELECT deviceId, updatedAt FROM device_runtime_state ORDER BY updatedAt DESC LIMIT 10;
```

## Step 8 — Worker smoke test

After import, deploy or run Worker preview and validate:

- `GET /v1/health`
- admin login/session flow once implemented
- device list once route parity is implemented
- logs query once route parity is implemented
- command creation/dispatch once Durable Object transport is implemented

Current Phase 2 only establishes D1 storage foundations; full API parity happens in later phases.

## Rollback

Rollback means routing traffic back to the normal-server deployment using the original SQLite database backup.

Steps:

1. Stop Worker traffic or switch DNS/routes back to the normal-server backend.
2. Restore SQLite backup if needed:

```bash
cp backups/cc-server.sqlite data/cc-server.sqlite
```

3. Start normal-server backend.
4. Validate:

```bash
cd cc-server
npm test
npm run build
```

## Important caveats

- This runbook is for a one-way snapshot import.
- It does not provide bidirectional replication between SQLite and D1.
- Session-like tables (`admin_sessions`, `rate_limit_buckets`) can be migrated, but it may be safer to clear them for production cutover and require new logins.
- Active PTY, transfer, remote browser, and WebSocket session states may be stale after migration. Prefer a maintenance window where active operations are stopped first.
- Secrets are already stored hashed/encrypted by the current app; do not transform secret values during migration unless a specific compatibility issue is discovered.
