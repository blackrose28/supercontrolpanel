#!/usr/bin/env node
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import {
  openTargetFixture,
  parseExampleArgs,
  runCommand,
  runScpctlJson,
} from './lib/agent-helpers.mjs';

const repoRoot = path.resolve(new URL('../..', import.meta.url).pathname);

function baseArgs(ctx) {
  return ['--config-dir', ctx.cliRoot, '--profile', 'shell-agent'];
}

async function runDiscover(scpctlBin, ctx) {
  const all = await runScpctlJson(scpctlBin, ['schema', '--json'], { env: ctx.env });
  assert.equal(all.code, 0, all.stderr);
  assert.equal(all.json.ok, true);
  assert.ok(Array.isArray(all.json.data?.commands), 'expected schema.commands array');
  assert.ok(all.json.data.commands.length >= 31, 'expected full CLI command catalog');

  const uploadSchema = await runScpctlJson(
    scpctlBin,
    ['schema', '--command', 'scp.upload', '--json'],
    { env: ctx.env },
  );
  assert.equal(uploadSchema.code, 0, uploadSchema.stderr);
  assert.equal(uploadSchema.json.ok, true);
  return 'passed';
}

async function runCapabilities(scpctlBin, ctx) {
  const caps = await runScpctlJson(
    scpctlBin,
    [...baseArgs(ctx), 'capabilities', '--json', '--non-interactive'],
    { env: ctx.env },
  );
  assert.equal(caps.code, 0, caps.stderr);
  assert.equal(caps.json.ok, true);
  assert.equal(typeof caps.json.data?.keyring?.state, 'string');
  return {
    status: 'passed',
    keyringAvailable: caps.json.data.keyring.state === 'available',
  };
}

async function runPreviewAndMutate(scpctlBin, ctx, serverPayload) {
  const serverFile = path.join(ctx.localWork, 'server.json');
  await writeFile(serverFile, JSON.stringify(serverPayload, null, 2));

  const preview = await runScpctlJson(
    scpctlBin,
    [
      ...baseArgs(ctx),
      'server',
      'upsert',
      '--file',
      serverFile,
      '--preview',
      '--json',
      '--non-interactive',
    ],
    { env: ctx.env },
  );
  assert.equal(preview.code, 0, preview.stderr);
  assert.equal(preview.json.data?.preview, true);
  const initialRevision = preview.json.data.revision;
  assert.equal(typeof initialRevision, 'string');

  const applied = await runScpctlJson(
    scpctlBin,
    [
      ...baseArgs(ctx),
      'server',
      'upsert',
      '--stdin',
      '--expected-revision',
      initialRevision,
      '--json',
      '--non-interactive',
    ],
    { env: ctx.env, input: JSON.stringify(serverPayload) },
  );
  assert.equal(applied.code, 0, applied.stderr);
  assert.equal(applied.json.data?.preview, false);
  assert.notEqual(applied.json.data?.revision, initialRevision);
  return { status: 'passed', initialRevision };
}

async function runCapabilityFailure(scpctlBin, ctx, serverId) {
  const rdp = await runScpctlJson(
    scpctlBin,
    [...baseArgs(ctx), 'server', 'rdp', '--server-id', serverId, '--non-interactive', '--json'],
    { env: ctx.env },
  );
  assert.equal(rdp.code, 4, `expected exit 4 for non-interactive server rdp, got ${rdp.code}`);
  assert.equal(rdp.json?.ok, false);
  assert.equal(rdp.json?.error?.code, 'CAPABILITY_UNAVAILABLE');
  assert.equal(rdp.json?.error?.reason, 'interactive_tty_required');

  const dummyBackup = path.join(ctx.localWork, 'dummy.scpbak');
  await writeFile(dummyBackup, 'placeholder');
  const imp = await runCommand(
    scpctlBin,
    [
      ...baseArgs(ctx),
      'settings',
      'import',
      '--input',
      dummyBackup,
      '--apply',
      '--non-interactive',
    ],
    { env: ctx.env },
  );
  assert.equal(imp.code, 4, `expected exit 4 for non-interactive settings import, got ${imp.code}`);
  assert.match(imp.stderr, /CAPABILITY_UNAVAILABLE/);
  return 'passed';
}

async function runLostResponseRecovery(scpctlBin, ctx, serverPayload, staleRevision) {
  const getRes = await runScpctlJson(
    scpctlBin,
    [...baseArgs(ctx), 'server', 'get', '--server-id', serverPayload.id, '--json', '--non-interactive'],
    { env: ctx.env },
  );
  assert.equal(getRes.code, 0, getRes.stderr);
  assert.equal(getRes.json.data?.host, '127.0.0.1');
  assert.equal(getRes.json.data?.sshPort, ctx.info.port);

  const conflict = await runScpctlJson(
    scpctlBin,
    [
      ...baseArgs(ctx),
      'server',
      'upsert',
      '--stdin',
      '--expected-revision',
      staleRevision,
      '--json',
      '--non-interactive',
    ],
    { env: ctx.env, input: JSON.stringify(serverPayload) },
  );
  assert.equal(conflict.code, 5, `expected exit 5 CONFLICT on stale revision, got ${conflict.code}`);
  assert.equal(conflict.json?.ok, false);
  assert.equal(conflict.json?.error?.code, 'CONFLICT');
  return 'passed';
}

async function runTransferAndInspect(scpctlBin, ctx, serverId) {
  const localSrc = path.join(ctx.localWork, 'upload.txt');
  const localDst = path.join(ctx.localWork, 'download.txt');
  const remotePath = path.posix.join(ctx.info.remoteSandbox, 'shell-agent-roundtrip.txt');
  const payload = 'slice5-shell-agent-workflow-payload\n';
  await writeFile(localSrc, payload);

  const up = await runScpctlJson(
    scpctlBin,
    [
      ...baseArgs(ctx),
      'scp',
      'upload',
      '--server-id',
      serverId,
      '--local',
      localSrc,
      '--remote',
      remotePath,
      '--json',
      '--non-interactive',
    ],
    { env: ctx.env },
  );
  assert.equal(up.code, 0, up.stderr);
  assert.equal(up.json.ok, true);

  const down = await runScpctlJson(
    scpctlBin,
    [
      ...baseArgs(ctx),
      'scp',
      'download',
      '--server-id',
      serverId,
      '--remote',
      remotePath,
      '--local',
      localDst,
      '--overwrite',
      '--json',
      '--non-interactive',
    ],
    { env: ctx.env },
  );
  assert.equal(down.code, 0, down.stderr);
  assert.equal(down.json.ok, true);
  assert.equal(await readFile(localDst, 'utf8'), payload);
  return 'passed';
}

async function runCleanup(scpctlBin, ctx, serverId, keyringAvailable) {
  const previewDel = await runScpctlJson(
    scpctlBin,
    [...baseArgs(ctx), 'server', 'delete', '--server-id', serverId, '--preview', '--json', '--non-interactive'],
    { env: ctx.env },
  );

  if (keyringAvailable) {
    assert.equal(previewDel.code, 0, previewDel.stderr);
    const rev = previewDel.json.data?.revision;
    assert.equal(typeof rev, 'string');

    const del = await runScpctlJson(
      scpctlBin,
      [
        ...baseArgs(ctx),
        'server',
        'delete',
        '--server-id',
        serverId,
        '--expected-revision',
        rev,
        '--json',
        '--non-interactive',
      ],
      { env: ctx.env },
    );
    assert.equal(del.code, 0, del.stderr);
    assert.equal(del.json.ok, true);
    return 'passed';
  }

  assert.equal(previewDel.code, 4, `expected exit 4 keyring_unavailable on server delete, got ${previewDel.code}`);
  assert.equal(previewDel.json?.ok, false);
  assert.equal(previewDel.json?.error?.code, 'CAPABILITY_UNAVAILABLE');
  assert.equal(previewDel.json?.error?.reason, 'keyring_unavailable');

  const st = await runScpctlJson(
    scpctlBin,
    [...baseArgs(ctx), 'status', '--json', '--non-interactive'],
    { env: ctx.env },
  );
  assert.equal(st.code, 0, st.stderr);
  assert.equal(st.json.ok, true);
  return 'passed';
}

async function main() {
  const { scpctlBin, sshdFixtureBin } = parseExampleArgs(process.argv.slice(2), repoRoot);
  const workDir = await mkdtemp(path.join(tmpdir(), 'scp-shell-workflow-'));
  const ctx = await openTargetFixture({
    sshdFixtureBin,
    workDir,
    profileName: 'shell-agent',
  });

  try {
    const serverPayload = {
      id: 'srv-shell-agent',
      name: 'Shell Agent Target',
      host: '127.0.0.1',
      os: 'linux',
      connectionMethod: 'ssh',
      sshPort: ctx.info.port,
      sshUser: ctx.info.user,
    };

    const discover = await runDiscover(scpctlBin, ctx);
    const caps = await runCapabilities(scpctlBin, ctx);
    const mutated = await runPreviewAndMutate(scpctlBin, ctx, serverPayload);
    const capabilityFailure = await runCapabilityFailure(scpctlBin, ctx, serverPayload.id);
    const lostResponseRecovery = await runLostResponseRecovery(
      scpctlBin,
      ctx,
      serverPayload,
      mutated.initialRevision,
    );
    const transferAndInspect = await runTransferAndInspect(scpctlBin, ctx, serverPayload.id);
    const cleanup = await runCleanup(scpctlBin, ctx, serverPayload.id, caps.keyringAvailable);

    process.stdout.write(
      `${JSON.stringify({
        ok: true,
        workflow: 'shell',
        steps: {
          discover,
          capabilities: caps.status,
          capabilityFailure,
          previewAndMutate: mutated.status,
          lostResponseRecovery,
          transferAndInspect,
          cleanup,
        },
      })}\n`,
    );
  } finally {
    await ctx.stop();
    await rm(workDir, { recursive: true, force: true });
  }
}

await main();
