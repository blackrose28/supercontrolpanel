import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { Client as ModernClient } from '@modelcontextprotocol/client';
import { StdioClientTransport as ModernStdioTransport } from '@modelcontextprotocol/client/stdio';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

const scpctlBin = new URL('../../../target/debug/scpctl', import.meta.url).pathname;

async function setupSeededEnv(t, stdioPolicy = null) {
  const base = await mkdtemp(path.join(tmpdir(), 'scpctl-slice1-e2e-'));
  t.after(() => rm(base, { recursive: true, force: true }));
  const cliRoot = path.join(base, 'cli-root');
  const desktopDir = path.join(base, 'desktop');
  const profileDir = path.join(cliRoot, 'profiles', 'stdio-e2e');
  await mkdir(profileDir, { recursive: true });
  await writeFile(
    path.join(profileDir, 'servers.json'),
    JSON.stringify({
      servers: [
        {
          id: 'srv-1',
          name: 'app-box',
          host: '10.1.2.3',
          os: 'linux',
          connectionMethod: 'ssh',
          sshPort: 22,
          sshUser: 'deploy',
        },
      ],
    }),
  );
  await writeFile(
    path.join(profileDir, 'sync-local.json'),
    JSON.stringify({
      mcpSensitiveActionPolicy: 'allow-with-token',
      mcpAllowedToolCategories: ['read-only', 'runtime-control', 'config-mutation', 'file-transfer'],
      mcpAuthToken: 'secret-mcp-token-e2e',
      ccAgentToken: 'secret-cc-token-e2e',
    }),
  );
  if (stdioPolicy) {
    await writeFile(path.join(profileDir, 'stdio-policy.json'), JSON.stringify(stdioPolicy));
  }
  return { cliRoot, desktopDir };
}

for (const [era, ClientClass, TransportClass, options, expectedProtocol] of [
  ['legacy', Client, StdioClientTransport, {}, '2025-11-25'],
  [
    'modern',
    ModernClient,
    ModernStdioTransport,
    { versionNegotiation: { mode: { pin: '2026-07-28' } } },
    '2026-07-28',
  ],
]) {
  test(`production scpctl stdio (${era}): discovery, resources, read-only calls, redaction, policy denials`, { timeout: 15000 }, async t => {
    const { cliRoot, desktopDir } = await setupSeededEnv(t, {
      sensitiveActionPolicy: 'read-only',
      allowedToolCategories: ['read-only', 'file-transfer'],
    });
    const client = new ClientClass(
      { name: `slice1-prod-${era}`, version: '1' },
      { capabilities: {}, ...options },
    );
    t.after(() => client.close());
    await client.connect(
      new TransportClass({
        command: scpctlBin,
        args: ['--config-dir', cliRoot, '--profile', 'stdio-e2e', 'mcp', 'serve', '--stdio'],
        env: { ...process.env, SCPCTL_DESKTOP_CONFIG_DIR: desktopDir },
        stderr: 'pipe',
      }),
    );

    assert.equal(client.getServerVersion().name, 'scpctl-stdio');
    if (era === 'modern') {
      assert.equal(client.getNegotiatedProtocolVersion(), expectedProtocol);
    }

    const listed = await client.listTools();
    assert.deepEqual(
      listed.tools.map(tool => tool.name).sort(),
      [
        'app_get_overview',
        'boundary_get_status',
        'boundary_search_targets',
        'config_get_paths',
        'detect_openvpn_executable',
        'function_status_get',
        'function_status_list',
        'openvpn_get_status',
        'remote_browser_list_children',
        'remote_browser_stat_path',
        'scp_transfer_get',
        'scp_transfer_list',
        'server_get',
        'servers_list',
        'sync_settings_get',
      ],
    );

    const resources = await client.listResources();
    assert.deepEqual(
      resources.resources.map(r => r.uri).sort(),
      [
        'scp://config/paths',
        'scp://servers/list',
        'scp://status/boundary',
        'scp://status/functions',
        'scp://status/openvpn',
        'scp://status/transfers',
      ],
    );
    for (const res of resources.resources) {
      const read = await client.readResource({ uri: res.uri });
      assert.equal(read.contents.length, 1);
      JSON.parse(read.contents[0].text);
    }

    const overview = await client.callTool({ name: 'app_get_overview', arguments: {} });
    assert.equal(overview.structuredContent.result.knownFunctionCount, 3);

    const fnList = await client.callTool({ name: 'function_status_list', arguments: {} });
    assert.equal(fnList.structuredContent.statuses.length, 3);

    const servers = await client.callTool({ name: 'servers_list', arguments: {} });
    assert.equal(servers.structuredContent.result.length, 1);
    assert.equal(servers.structuredContent.result[0].id, 'srv-1');

    const srv = await client.callTool({ name: 'server_get', arguments: { serverId: 'srv-1' } });
    assert.equal(srv.structuredContent.result.host, '10.1.2.3');

    const pathsRes = await client.callTool({ name: 'config_get_paths', arguments: {} });
    assert.equal(pathsRes.structuredContent.result.appConfigPath.endsWith('config.json'), true);

    const settings = await client.callTool({ name: 'sync_settings_get', arguments: {} });
    const serialized = JSON.stringify(settings);
    assert.equal(serialized.includes('secret-mcp-token-e2e'), false);
    assert.equal(serialized.includes('secret-cc-token-e2e'), false);

    const transfers = await client.callTool({ name: 'scp_transfer_list', arguments: {} });
    assert.deepEqual(transfers.structuredContent.result, []);

    const missingTx = await client.callTool({ name: 'scp_transfer_get', arguments: { transferId: 'missing' } });
    assert.equal(missingTx.isError, true);
    assert.match(missingTx.content[0].text, /^NOT_FOUND:/);

    for (const [deniedName, args] of [
      ['openvpn_start', {}],
      ['boundary_auth_start', {}],
      ['caddy_start', { folder: '/tmp/caddy' }],
      ['config_sync_push', { repoRoot: '/tmp/repo' }],
      [
        'server_upsert',
        {
          server: {
            id: 'srv-2',
            name: 'beta',
            host: '10.1.2.4',
            connectionMethod: 'ssh',
            sshPort: 22,
            sshUser: 'deploy',
          },
        },
      ],
    ]) {
      const denied = await client.callTool({ name: deniedName, arguments: args });
      assert.equal(denied.isError, true, `expected denial for ${deniedName}`);
      assert.match(denied.content[0].text, /^(POLICY_DENIED|CAPABILITY_UNAVAILABLE):/);
    }
  });
}

test('production scpctl stdio: default policy exposes 11 tools, keeps stdout pure NDJSON, exits 0 on EOF, blocks second owner', { timeout: 15000 }, async t => {
  const { cliRoot, desktopDir } = await setupSeededEnv(t);
  const child = spawn(
    scpctlBin,
    ['--config-dir', cliRoot, '--profile', 'stdio-e2e', 'mcp', 'serve', '--stdio'],
    {
      stdio: ['pipe', 'pipe', 'pipe'],
      env: { ...process.env, SCPCTL_DESKTOP_CONFIG_DIR: desktopDir },
    },
  );
  const exitPromise = once(child, 'exit').then(([code]) => code);
  t.after(async () => {
    if (child.exitCode === null) {
      child.kill('SIGKILL');
      await exitPromise.catch(() => {});
    }
  });

  const nonJson = [];
  const messages = [];
  const waiters = new Map();
  let buf = '';
  child.stdout.on('data', chunk => {
    buf += chunk.toString('utf8');
    let nl;
    while ((nl = buf.indexOf('\n')) !== -1) {
      const line = buf.slice(0, nl).replace(/\r$/, '');
      buf = buf.slice(nl + 1);
      if (!line) continue;
      try {
        const msg = JSON.parse(line);
        messages.push(msg);
        if (msg.id !== undefined && waiters.has(msg.id)) {
          waiters.get(msg.id)(msg);
          waiters.delete(msg.id);
        }
      } catch {
        nonJson.push(line);
      }
    }
  });
  const sendAndWait = (id, method, params = {}) =>
    new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout id=${id}`)), 4000);
      waiters.set(id, msg => {
        clearTimeout(timer);
        resolve(msg);
      });
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });

  await sendAndWait(1, 'initialize', {
    protocolVersion: '2025-11-25',
    capabilities: {},
    clientInfo: { name: 'raw-prod', version: '1' },
  });
  child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);

  const tools = await sendAndWait(2, 'tools/list');
  assert.equal(tools.result.tools.length, 11);

  const conflictChild = spawn(
    scpctlBin,
    ['--config-dir', cliRoot, '--profile', 'stdio-e2e', 'mcp', 'serve', '--stdio'],
    {
      stdio: ['pipe', 'pipe', 'pipe'],
      env: { ...process.env, SCPCTL_DESKTOP_CONFIG_DIR: desktopDir },
    },
  );
  const conflictStdout = [];
  conflictChild.stdout.on('data', d => conflictStdout.push(d));
  const [conflictCode] = await once(conflictChild, 'exit');
  assert.equal(conflictCode, 5);
  assert.equal(Buffer.concat(conflictStdout).length, 0);

  child.stdin.end();
  const code = await exitPromise;
  assert.equal(code, 0);
  assert.deepEqual(nonJson, []);
});
