// Opt-in Linux native qualification. Run under xvfb-run + dbus-run-session.
// Uses only an isolated XDG profile and inert server metadata; no network sessions/keyring writes.
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { createServer } from 'node:net';
import { randomUUID } from 'node:crypto';
import { promisify } from 'node:util';
import { Client as ModernClient, StreamableHTTPClientTransport as ModernTransport } from '@modelcontextprotocol/client';
import { Client as LegacyClient } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport as LegacyTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';

const root = await mkdtemp(join(tmpdir(), 'scp-native-mcp-'));
const config = join(root, 'config', 'com.chuonglv.supercontrolpanel');
const socket = createServer().listen(0, '127.0.0.1');
await once(socket, 'listening');
const port = socket.address().port;
await new Promise(resolve => socket.close(resolve));
const base = `http://127.0.0.1:${port}`;
const binary = resolve(process.env.SCP_NATIVE_BINARY ?? 'src-tauri/target/debug/super-control-panel');
const inspector = new URL('../node_modules/@modelcontextprotocol/inspector/clients/launcher/build/index.js', import.meta.url).pathname;
const fixture = JSON.parse(await readFile(new URL('./desktop-surface.json', import.meta.url)));
const token = randomUUID();
let app;
let exit;
let activeToken = token;

async function stop() {
  if (!app) return;
  app.kill('SIGTERM');
  try {
    await Promise.race([exit, new Promise((_, reject) => {
      const timer = setTimeout(() => reject(new Error('native shutdown timed out')), 5000);
      timer.unref();
    })]);
  } finally {
    if (app.exitCode === null && app.signalCode === null) { app.kill('SIGKILL'); await exit; }
    app = null;
  }
}
async function start(policy, bearer) {
  // D-Bus-activated keyring/portal services must inherit the temporary profile too.
  const xdg = { XDG_CONFIG_HOME: join(root, 'config'), XDG_DATA_HOME: join(root, 'data'), XDG_CACHE_HOME: join(root, 'cache'), XDG_RUNTIME_DIR: join(root, 'runtime') };
  await mkdir(xdg.XDG_RUNTIME_DIR, { recursive: true, mode: 0o700 });
  await promisify(execFile)('dbus-update-activation-environment', Object.entries(xdg).map(([key, value]) => `${key}=${value}`));
  await mkdir(config, { recursive: true });
  await writeFile(join(config, 'sync-local.json'), JSON.stringify({
    firstRunCompleted: true, ccEnabled: false, mcpEnabled: true,
    mcpBindAddress: '127.0.0.1', mcpPort: port, mcpAuthToken: bearer,
    mcpAllowedToolCategories: ['read-only', 'runtime-control', 'config-mutation'],
    mcpSensitiveActionPolicy: policy,
  }));
  activeToken = bearer;
  app = spawn(binary, [], { env: {
    ...process.env, ...xdg, WEBKIT_DISABLE_COMPOSITING_MODE: '1',
  }, stdio: 'ignore' });
  exit = once(app, 'exit');
  for (let attempt = 0; attempt < 200; attempt++) {
    if (app.exitCode !== null || app.signalCode !== null) throw new Error('native application exited before readiness');
    try {
      const response = await fetch(`${base}/health`, { headers: { authorization: `Bearer ${bearer}` }, signal: AbortSignal.timeout(1000) });
      if (response.ok) {
        const { stdout: windows } = await promisify(execFile)('xdotool', ['search', '--sync', '--onlyvisible', '--pid', String(app.pid)], { timeout: 5000 });
        const { stdout: title } = await promisify(execFile)('xdotool', ['getwindowname', windows.trim().split('\n')[0]]);
        assert.match(title, /Super Control Panel/);
        return;
      }
    } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error('native application listener did not become ready');
}
async function run(era, args, allowMissing = false) {
  try {
    const { stdout } = await promisify(execFile)(process.execPath, [
      inspector, '--cli', '--server-url', `${base}/mcp`, '--transport', 'http',
      '--protocol-era', era, '--header', `Authorization: Bearer ${activeToken}`,
      '--format', 'json', '--quiet', ...args,
    ], { timeout: 15000 });
    return JSON.parse(stdout).result;
  } catch (error) {
    // execFile errors include command arguments (and bearer); never print them.
    let code;
    try { code = JSON.parse(error.stderr).error.code; } catch {}
    if (allowMissing && code === 'tool_not_found') return { clientError: code };
    throw new Error('Inspector invocation failed; inspect the isolated test locally without logging bearer arguments');
  }
}
async function call(era, name, args, allowMissing = false) {
  return run(era, ['--method', 'tools/call', '--tool-name', name, '--tool-args-json', JSON.stringify(args)], allowMissing);
}
try {
  await start('allow-with-token', token);
  assert.equal((await fetch(`${base}/health`)).status, 401);
  for (const era of ['modern', 'legacy']) {
    const discovery = await run(era, ['--method', 'tools/list']);
    assert.ok(discovery.tools.some(tool => tool.name === 'server_upsert'));
    const status = await call(era, 'function_status_list', {});
    assert.ok(Array.isArray(status.structuredContent.statuses));
    const changed = await call(era, 'server_upsert', fixture.server_upsert.args);
    assert.notEqual(changed.isError, true);
    assert.equal(changed.structuredContent.result[0].id, 'srv-1');
    assert.equal(JSON.parse(await readFile(join(config, 'servers.json'))).servers[0].id, 'srv-1');
    const snapshot = await run(era, ['--method', 'resources/read', '--uri', 'scp://servers/list']);
    assert.equal(JSON.parse(snapshot.contents[0].text).result[0].id, 'srv-1');
    console.log(`Inspector 2.14.0 ${era}: native discovery, safe query, persisted metadata mutation, resource, reconnect PASS`);
  }
  await stop();
  await start('require-ui-approval', randomUUID());
  assert.equal((await fetch(`${base}/health`, { headers: { authorization: `Bearer ${token}` } })).status, 401);
  for (const era of ['modern', 'legacy']) {
    const discovery = await run(era, ['--method', 'tools/list']);
    assert.ok(!discovery.tools.some(tool => tool.name === 'server_delete'));
    const refused = await call(era, 'server_delete', fixture.server_delete.args, true);
    assert.equal(refused.clientError, 'tool_not_found');
    // Inspector refuses hidden tools locally. Check server enforcement with the official SDK too.
    const C = era === 'modern' ? ModernClient : LegacyClient;
    const T = era === 'modern' ? ModernTransport : LegacyTransport;
    const client = new C({ name: 'native-qualification', version: '1' }, era === 'modern' ? { versionNegotiation: { mode: { pin: '2026-07-28' } } } : {});
    try {
      await client.connect(new T(new URL(`${base}/mcp`), { requestInit: { headers: { authorization: `Bearer ${activeToken}` } } }));
      const denied = await client.callTool({ name: 'server_delete', arguments: fixture.server_delete.args });
      assert.equal(denied.isError, true);
      assert.match(denied.content[0].text, /POLICY_DENIED/);
    } finally { await client.close(); }
    assert.equal(JSON.parse(await readFile(join(config, 'servers.json'))).servers[0].id, 'srv-1');
    assert.ok(Array.isArray((await call(era, 'function_status_list', {})).structuredContent.statuses));
    console.log(`Inspector 2.14.0 ${era}: native restart, old-token rejection, policy denial without mutation, reconnect PASS`);
  }
} finally {
  await stop();
  // The isolated document portal may mount this directory on demand.
  await promisify(execFile)('fusermount3', ['-u', join(root, 'runtime', 'doc')]).catch(() => {});
  await rm(root, { recursive: true, force: true });
}
