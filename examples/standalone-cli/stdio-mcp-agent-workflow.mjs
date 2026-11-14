#!/usr/bin/env node
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import {
  loadModernMcpClient,
  openTargetFixture,
  parseExampleArgs,
  runScpctlJson,
} from './lib/agent-helpers.mjs';

const repoRoot = path.resolve(new URL('../..', import.meta.url).pathname);

function baseArgs(ctx) {
  return ['--config-dir', ctx.cliRoot, '--profile', 'mcp-agent'];
}

async function connectAndDiscover(scpctlBin, ctx) {
  const { Client, StdioClientTransport } = await loadModernMcpClient(repoRoot);
  const client = new Client(
    { name: 'stdio-mcp-agent-workflow', version: '1.0.0' },
    { capabilities: {}, versionNegotiation: { mode: { pin: '2026-07-28' } } },
  );
  const transport = new StdioClientTransport({
    command: scpctlBin,
    args: [...baseArgs(ctx), 'mcp', 'serve', '--stdio'],
    env: ctx.env,
    stderr: 'pipe',
  });
  await client.connect(transport);

  const [{ tools }, { resources }] = await Promise.all([
    client.listTools(),
    client.listResources(),
  ]);
  assert.equal(tools.length, 41, `expected 41 MCP tools, got ${tools.length}`);
  assert.equal(resources.length, 6, `expected 6 MCP resources, got ${resources.length}`);

  for (const [name, args] of [
    ['server_rdp_start', { serverId: 'srv-mcp-agent' }],
    ['config_sync_pull', { repoRoot: ctx.localWork }],
  ]) {
    const res = await client.callTool({ name, arguments: args });
    assert.equal(res.isError, true, `expected error for ${name}`);
    assert.match(res.content[0].text, /^CAPABILITY_UNAVAILABLE:/);
  }

  await assert.rejects(
    () =>
      client.callTool({
        name: 'sync_settings_save_partial',
        arguments: { patch: { mcpEnabled: true } },
      }),
    /Invalid tool arguments/,
  );

  const tamperedConfig = await client.callTool({
    name: 'config_save',
    arguments: {
      config: {
        app: { defaultTerminalCols: 100, defaultTerminalRows: 30 },
        openvpn: { executablePath: '/evil/openvpn' },
      },
    },
  });
  assert.equal(tamperedConfig.isError, true, 'expected POLICY_DENIED on executable override');
  assert.match(tamperedConfig.content[0].text, /^POLICY_DENIED:/);

  return { client, status: 'passed' };
}

async function verifyOwnerConflict(scpctlBin, ctx) {
  const conflict = await runScpctlJson(
    scpctlBin,
    [...baseArgs(ctx), 'status', '--json', '--non-interactive'],
    { env: ctx.env },
  );
  assert.equal(conflict.code, 5, `expected exit 5 owner conflict, got ${conflict.code}`);
  assert.equal(conflict.json?.ok, false);
  assert.equal(conflict.json?.error?.code, 'CONFLICT');
  assert.equal(conflict.json?.error?.reason, 'profile_owner_conflict');
  assert.equal(conflict.json?.error?.nextAction, 'use_owner_connection');
  return 'passed';
}

async function runUpsertAndBrowse(client, ctx, serverPayload) {
  const upserted = await client.callTool({
    name: 'server_upsert',
    arguments: { server: serverPayload },
  });
  assert.equal(Boolean(upserted.isError), false, JSON.stringify(upserted));
  assert.equal(
    upserted.structuredContent.result.some(s => s.id === serverPayload.id),
    true,
  );

  const opened = await client.callTool({
    name: 'remote_browser_open',
    arguments: { serverId: serverPayload.id },
  });
  assert.equal(Boolean(opened.isError), false, JSON.stringify(opened));
  const browserId = opened.structuredContent.result.browserId;

  const remoteDir = path.posix.join(ctx.info.remoteSandbox, 'mcp-agent-work');
  const mk = await client.callTool({
    name: 'remote_browser_create_directory',
    arguments: { browserId, path: remoteDir },
  });
  assert.equal(Boolean(mk.isError), false, JSON.stringify(mk));

  const listed = await client.callTool({
    name: 'remote_browser_list_children',
    arguments: { browserId, path: ctx.info.remoteSandbox },
  });
  assert.equal(
    listed.structuredContent.result.some(entry => entry.name === 'mcp-agent-work'),
    true,
  );

  const st = await client.callTool({
    name: 'remote_browser_stat_path',
    arguments: { browserId, path: remoteDir },
  });
  assert.equal(st.structuredContent.result.exists, true);
  assert.equal(st.structuredContent.result.kind, 'directory');

  return { browserId, remoteDir, status: 'passed' };
}

async function waitForTransferSuccess(client, sessionId) {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    const listed = await client.callTool({ name: 'scp_transfer_list', arguments: {} });
    const match = listed.structuredContent.result.find(tx => tx.sessionId === sessionId);
    if (match && (match.state === 'success' || match.state === 'error')) {
      return match;
    }
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  throw new Error(`Timed out waiting for transfer session ${sessionId}`);
}

async function runAsyncCompletion(client, ctx, serverId, remoteDir) {
  const localSrc = path.join(ctx.localWork, 'mcp-async.txt');
  await writeFile(localSrc, 'slice5-stdio-mcp-async-payload\n');

  const started = await client.callTool({
    name: 'scp_start_upload',
    arguments: { serverId, localPath: localSrc, remotePath: remoteDir },
  });
  assert.equal(Boolean(started.isError), false, JSON.stringify(started));
  const { sessionId } = started.structuredContent.result;
  assert.ok(sessionId);

  const completed = await waitForTransferSuccess(client, sessionId);
  assert.equal(completed.state, 'success', JSON.stringify(completed));

  const fetched = await client.callTool({
    name: 'scp_transfer_get',
    arguments: { transferId: completed.id },
  });
  assert.equal(fetched.structuredContent.result.id, completed.id);
  assert.equal(fetched.structuredContent.result.state, 'success');

  const resRead = await client.readResource({ uri: 'scp://status/transfers' });
  const parsed = JSON.parse(resRead.contents[0].text);
  const transfers = parsed.result ?? parsed;
  assert.equal(transfers.some(tx => tx.id === completed.id), true);

  return { transferId: completed.id, status: 'passed' };
}

async function runLostResponseRecovery(client, serverId, transferId) {
  const txs = await client.callTool({ name: 'scp_transfer_list', arguments: {} });
  assert.equal(txs.structuredContent.result.some(tx => tx.id === transferId), true);

  const srv = await client.callTool({
    name: 'server_get',
    arguments: { serverId },
  });
  assert.equal(Boolean(srv.isError), false, JSON.stringify(srv));
  assert.equal(srv.structuredContent.result.id, serverId);
  return 'passed';
}

async function runCleanup(client, scpctlBin, ctx, browserId) {
  const closed = await client.callTool({
    name: 'remote_browser_close',
    arguments: { browserId },
  });
  assert.equal(Boolean(closed.isError), false, JSON.stringify(closed));
  await client.close();

  const postClose = await runScpctlJson(
    scpctlBin,
    [...baseArgs(ctx), 'status', '--json', '--non-interactive'],
    { env: ctx.env },
  );
  assert.equal(postClose.code, 0, postClose.stderr);
  assert.equal(postClose.json?.ok, true);
  return 'passed';
}

async function main() {
  const { scpctlBin, sshdFixtureBin } = parseExampleArgs(process.argv.slice(2), repoRoot);
  const workDir = await mkdtemp(path.join(tmpdir(), 'scp-mcp-workflow-'));
  const ctx = await openTargetFixture({
    sshdFixtureBin,
    workDir,
    profileName: 'mcp-agent',
    stdioPolicy: {
      sensitiveActionPolicy: 'allow-local-policy',
      allowedToolCategories: [
        'read-only',
        'runtime-control',
        'config-mutation',
        'file-transfer',
      ],
    },
  });

  let client = null;
  try {
    const serverPayload = {
      id: 'srv-mcp-agent',
      name: 'Stdio MCP Agent Target',
      host: '127.0.0.1',
      os: 'linux',
      connectionMethod: 'ssh',
      sshPort: ctx.info.port,
      sshUser: ctx.info.user,
    };

    const discovered = await connectAndDiscover(scpctlBin, ctx);
    client = discovered.client;
    const ownerConflict = await verifyOwnerConflict(scpctlBin, ctx);
    const browsed = await runUpsertAndBrowse(client, ctx, serverPayload);
    const asyncDone = await runAsyncCompletion(client, ctx, serverPayload.id, browsed.remoteDir);
    const lostResponseRecovery = await runLostResponseRecovery(
      client,
      serverPayload.id,
      asyncDone.transferId,
    );
    const cleanup = await runCleanup(client, scpctlBin, ctx, browsed.browserId);
    client = null;

    process.stdout.write(
      `${JSON.stringify({
        ok: true,
        workflow: 'stdio-mcp',
        steps: {
          connectAndDiscover: discovered.status,
          ownerConflict,
          upsertAndBrowse: browsed.status,
          asyncCompletion: asyncDone.status,
          lostResponseRecovery,
          cleanup,
        },
      })}\n`,
    );
  } finally {
    if (client) {
      await client.close().catch(err => {
        process.stderr.write(`stdio cleanup close warning: ${err.message}\n`);
      });
    }
    await ctx.stop();
    await rm(workDir, { recursive: true, force: true });
  }
}

await main();
