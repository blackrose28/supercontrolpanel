import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { once } from 'node:events';
import { promisify } from 'node:util';
import { request as httpRequest } from 'node:http';
import { readFile } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';
import { Client as ModernClient, StreamableHTTPClientTransport as ModernTransport } from '@modelcontextprotocol/client';
import { createInterface } from 'node:readline';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';

// Public, fixture-only credential. Never load desktop settings or real secrets.
const authorization = 'Bearer slice0-fixture-only';

async function fixture(t, mode = 'json') {
  const child = spawn(new URL('../target/debug/scp-mcp-http-slice0', import.meta.url).pathname, [mode], { stdio: ['ignore', 'pipe', 'inherit'] });
  t.after(async () => {
    if (child.pid && child.exitCode === null) { child.kill('SIGTERM'); await once(child, 'exit'); }
  });
  const lines = createInterface({ input: child.stdout });
  const [line] = await Promise.race([
    once(lines, 'line'),
    once(child, 'error').then(([error]) => { throw error; }),
    once(child, 'exit').then(([code]) => { throw new Error(`fixture exited: ${code}`); }),
  ]);
  lines.close();
  return new URL(line);
}

for (const [era, ClientClass, TransportClass, options, revision] of [
  ['legacy', Client, StreamableHTTPClientTransport, {}, '2025-11-25'],
  ['modern', ModernClient, ModernTransport, { versionNegotiation: { mode: { pin: '2026-07-28' } } }, '2026-07-28'],
]) for (const mode of ['json', 'sse']) {
  test(`${era} official SDK: discovery, resource, success and denial (${mode} preference)`, { timeout: 20000 }, async t => {
    const url = await fixture(t, mode);
    const client = new ClientClass({ name: `slice0-${era}`, version: '1' }, { capabilities: {}, ...options });
    t.after(() => client.close());
    const trace = [];
    const transport = new TransportClass(url, { requestInit: { headers: { authorization } }, fetch: async (input, init) => {
      const response = await fetch(input, init);
      trace.push({ method: init?.body ? JSON.parse(init.body).method : init?.method, version: new Headers(init?.headers).get('mcp-protocol-version'), type: response.headers.get('content-type'), status: response.status });
      return response;
    } });
    await client.connect(transport);
    assert.equal(client.getServerVersion().name, 'slice0-fixture');
    assert.equal(era === 'modern' ? client.getNegotiatedProtocolVersion() : transport.protocolVersion, revision);
    const tools = await client.listTools();
    assert.deepEqual(tools.tools.map(tool => tool.name).sort(), ['fixture_echo', 'fixture_slow']);
    const result = await client.callTool({ name: 'fixture_echo', arguments: { value: 'hello' } });
    assert.deepEqual(result.structuredContent, { value: 'hello' });
    const denied = await client.callTool({ name: 'fixture_echo', arguments: { value: 'deny' } });
    assert.equal(denied.isError, true);
    assert.equal((await client.listResources()).resources[0].uri, 'fixture://status');
    assert.equal((await client.readResource({ uri: 'fixture://status' })).contents[0].text, '{"fixture":true}');
    const response = trace.find(row => row.method === 'tools/call');
    assert.match(response.type, era === 'modern' && mode === 'json' ? /application\/json/ : /text\/event-stream/);
    assert.equal(Boolean(transport.sessionId), era === 'legacy');
    assert.equal(trace.some(row => row.method === (era === 'modern' ? 'server/discover' : 'initialize')), true);
    if (era === 'modern') assert.equal(trace.some(row => row.method === 'initialize'), false);
    t.diagnostic(JSON.stringify({ era, mode, revision, trace }));
  });
}

test('HTTP rejects missing bearer and untrusted Origin/Host', { timeout: 10000 }, async t => {
  const url = await fixture(t);
  const request = (headers) => fetch(url, { method: 'POST', headers: { accept: 'application/json, text/event-stream', 'content-type': 'application/json', ...headers }, body: '{}' });
  assert.equal((await request({})).status, 401);
  assert.equal((await request({ authorization, origin: 'https://untrusted.invalid' })).status, 403);
  const hostStatus = await new Promise((resolve, reject) => {
    const req = httpRequest(url, { method: 'POST', headers: { authorization, host: 'untrusted.invalid', accept: 'application/json, text/event-stream', 'content-type': 'application/json' } }, response => { response.resume(); resolve(response.statusCode); });
    req.on('error', reject); req.end('{}');
  });
  assert.equal(hostStatus, 403);
});

for (const era of ['modern', 'legacy']) for (const mode of ['json', 'sse']) {
  test(`Inspector CLI real client: ${era} discovery, call and resource (${mode} preference)`, { timeout: 30000 }, async t => {
    const url = await fixture(t, mode);
    const run = async args => {
      const { stdout } = await promisify(execFile)(process.execPath, [
        new URL('../node_modules/@modelcontextprotocol/inspector/clients/launcher/build/index.js', import.meta.url).pathname,
        '--cli', '--server-url', url.href, '--transport', 'http', '--protocol-era', era,
        '--header', `Authorization: ${authorization}`, '--format', 'json', '--quiet', ...args,
      ], { timeout: 10000 });
      return JSON.parse(stdout);
    };
    const tools = await run(['--method', 'tools/list']);
    assert.deepEqual(tools.result.tools.map(tool => tool.name).sort(), ['fixture_echo', 'fixture_slow']);
    const result = await run(['--method', 'tools/call', '--tool-name', 'fixture_echo', '--tool-args-json', '{"value":"inspector"}']);
    assert.deepEqual(result.result.structuredContent, { value: 'inspector' });
    const resource = await run(['--method', 'resources/read', '--uri', 'fixture://status']);
    assert.equal(resource.result.contents[0].text, '{"fixture":true}');
    t.diagnostic(JSON.stringify({ era, mode, tools, result, resource }));
  });
}

for (const [era, ClientClass, TransportClass, options] of [
  ['legacy', Client, StreamableHTTPClientTransport, {}],
  ['modern', ModernClient, ModernTransport, { versionNegotiation: { mode: { pin: '2026-07-28' } } }],
]) {
  test(`${era}: cancelling a slow request releases server work`, { timeout: 12000 }, async t => {
    const url = await fixture(t, 'sse');
    const client = new ClientClass({ name: 'slice0-cancel', version: '1' }, options);
    t.after(() => client.close());
    await client.connect(new TransportClass(url, { requestInit: { headers: { authorization } } }));
    const active = async () => Number((await client.readResource({ uri: 'fixture://active' })).contents[0].text);
    const controller = new AbortController();
    const args = { name: 'fixture_slow', arguments: { value: 'wait' } };
    const pending = era === 'modern' ? client.callTool(args, { signal: controller.signal }) : client.callTool(args, undefined, { signal: controller.signal });
    const rejected = assert.rejects(pending);
    for (let attempt = 0; attempt < 50 && await active() !== 1; attempt++) await delay(20);
    assert.equal(await active(), 1);
    controller.abort();
    await rejected;
    for (let attempt = 0; attempt < 50 && await active() !== 0; attempt++) await delay(20);
    assert.equal(await active(), 0, 'server must cancel work, not only reject the client promise');
  });
}

test('inventory accounts for every current tool, resource and registered desktop command', async () => {
  const root = new URL('../../../', import.meta.url);
  const inventory = await readFile(new URL('docs/mcp/DESKTOP_STREAMABLE_HTTP_INVENTORY.md', root), 'utf8');
  const tools = await readFile(new URL('src-tauri/src/mcp/tools.rs', root), 'utf8');
  const names = [...tools.matchAll(/name: "([^"]+)"/g)].map(match => match[1]);
  assert.equal(names.length, 40);
  for (const name of names) assert.ok(inventory.includes(`OP-${name}`), name);
  const resources = await readFile(new URL('src-tauri/src/mcp/resources.rs', root), 'utf8');
  for (const [, uri] of resources.matchAll(/uri: "([^"]+)"/g)) assert.ok(inventory.includes(uri), uri);
  const main = await readFile(new URL('src-tauri/src/main.rs', root), 'utf8');
  const registrations = main.split('tauri::generate_handler![')[1].split('])')[0];
  for (const command of registrations.split(',').map(value => value.trim().split('::').at(-1)).filter(Boolean)) assert.ok(inventory.includes(command), command);
});

test('modern version/header mismatch and invalid tool input fail before successful dispatch', { timeout: 10000 }, async t => {
  const url = await fixture(t);
  const call = async (version, headers = {}, value = 'safe') => {
    const response = await fetch(url, { method: 'POST', headers: {
      authorization, accept: 'application/json, text/event-stream', 'content-type': 'application/json',
      'MCP-Protocol-Version': version, 'Mcp-Method': 'tools/call', 'Mcp-Name': 'fixture_echo', ...headers,
    }, body: JSON.stringify({ jsonrpc: '2.0', id: 100, method: 'tools/call', params: { name: 'fixture_echo', arguments: { value },
      _meta: { 'io.modelcontextprotocol/protocolVersion': version, 'io.modelcontextprotocol/clientInfo': { name: 'slice0-raw', version: '1' }, 'io.modelcontextprotocol/clientCapabilities': {} },
    } }) });
    return { status: response.status, body: await response.json() };
  };
  const unknown = await call('2099-01-01');
  assert.equal(unknown.status, 400); assert.equal(unknown.body.error.code, -32022);
  const mismatch = await call('2026-07-28', { 'Mcp-Name': 'different' });
  assert.equal(mismatch.status, 400); assert.ok(mismatch.body.error);
  const badInput = await call('2026-07-28', {}, 17);
  assert.equal(badInput.body.error.code, -32602);
  t.diagnostic(JSON.stringify({ unknown, mismatch, badInput }));
});
