// Run by the desktop Rust integration test against its real protocol router.
import assert from 'node:assert/strict';
import { request } from 'node:http';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { Client as ModernClient, StreamableHTTPClientTransport as ModernTransport } from '@modelcontextprotocol/client';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
const base = process.env.SLICE2_URL;
const authorization = 'Bearer test-only';
const uri = 'scp://status/functions';
const clients = [];
const statuses = [{ functionId: "boundary-auth", state: "running", message: "", boundaryTokenSource: "app", sessionId: "synthetic-session", updatedAtMs: 123 }];
try {
  for (const [era, C, T, options] of [
    ['legacy', Client, StreamableHTTPClientTransport, {}],
    ['modern', ModernClient, ModernTransport, { versionNegotiation: { mode: { pin: '2026-07-28' } } }],
  ]) {
    const client = new C({ name: 'desktop-slice2-test', version: '1' }, options);
    clients.push(client);
    const types = [];
    const transport = new T(new URL(`${base}/mcp`), {
      requestInit: { headers: { authorization } },
      fetch: async (input, init) => {
        const response = await fetch(input, init);
        if (init?.body && JSON.parse(init.body).method === 'tools/call') types.push(response.headers.get('content-type'));
        return response;
      },
    });
    await client.connect(transport);
    assert.equal(era === 'modern' ? client.getNegotiatedProtocolVersion() : transport.protocolVersion, era === 'modern' ? '2026-07-28' : '2025-11-25');
    assert.equal(client.getServerVersion().name, 'super-control-panel');
    const { tools } = await client.listTools();
    assert.deepEqual(tools.map(t => t.name), ['function_status_list']);
    assert.equal(tools[0].inputSchema.additionalProperties, false);
    assert.equal(tools[0].outputSchema.properties.statuses.type, 'array');
    const result = await client.callTool({ name: 'function_status_list', arguments: {} });
    assert.deepEqual(result.structuredContent, { statuses });
    assert.match(types[0], era === 'modern' && process.env.SLICE2_MODE === 'json' ? /application\/json/ : /text\/event-stream/);
    assert.deepEqual(JSON.parse(result.content[0].text), result.structuredContent);
    assert.equal((await client.listResources()).resources[0].uri, uri);
    const resource = (await client.readResource({ uri })).contents[0];
    assert.equal(resource.mimeType, 'application/json');
    assert.deepEqual(JSON.parse(resource.text), { statuses });
    await assert.rejects(client.callTool({ name: 'function_status_list', arguments: { typo: true } }), /Invalid|invalid/);
    await assert.rejects(client.callTool({ name: 'server_tunnel_start', arguments: {} }));
    await assert.rejects(client.readResource({ uri: 'scp://unknown' }));
    console.log(`${era}: discovery, schemas, success, resource and invalid calls pass`);
  }
  for (const path of ['/mcp', '/health', '/mcp/tools', '/mcp/resources', '/mcp/status', '/mcp/tools/call', '/mcp/resources/read']) {
    assert.equal((await fetch(base + path)).status, 401, path);
    assert.equal((await fetch(base + path, { headers: { authorization: 'Bearer wrong' } })).status, 401, path);
    assert.equal((await fetch(base + path, { headers: { authorization, origin: 'https://evil.invalid' } })).status, 403, path);
    const status = await new Promise((resolve, reject) => {
      const req = request(base + path, { headers: { authorization, host: 'evil.invalid' } }, res => { res.resume(); resolve(res.statusCode); });
      req.on('error', reject); req.end();
    });
    assert.equal(status, 403, path);
  }
  const absoluteAuthorityStatus = await new Promise((resolve, reject) => {
    const req = request(base, { path: 'http://evil.invalid/health', headers: { authorization } }, res => { res.resume(); resolve(res.statusCode); });
    req.on('error', reject); req.end();
  });
  assert.equal(absoluteAuthorityStatus, 403, 'absolute request authority must also be trusted');
  const unauthorized = await fetch(`${base}/health`);
  assert.equal((await unauthorized.json()).message, 'Unauthorized');
  const oversized = await fetch(`${base}/mcp/tools/call`, { method: 'POST', headers: { authorization }, body: 'x'.repeat(1024 * 1024 + 1) });
  assert.equal(oversized.status, 413);
  const legacy = await fetch(`${base}/mcp/tools/call`, { method: 'POST', headers: { authorization, 'content-type': 'application/json' }, body: JSON.stringify({ toolName: 'function_status_list', args: {} }) });
  assert.deepEqual(await legacy.json(), { ok: true, message: 'MCP operation completed', data: statuses, warnings: [], timestamp: 123 });
  const helper = process.env.SLICE2_HELPER;
  const { stdout } = await promisify(execFile)(helper, ['--json', '--endpoint', base, '--token', 'test-only', 'function', 'list']);
  assert.deepEqual(JSON.parse(stdout).data, statuses);
  const unsupported = await fetch(`${base}/mcp`, { method: 'POST', headers: { authorization, accept: 'application/json, text/event-stream', 'content-type': 'application/json', 'MCP-Protocol-Version': '2099-01-01', 'Mcp-Method': 'tools/list' }, body: JSON.stringify({ jsonrpc: '2.0', id: 42, method: 'tools/list', params: {} }) });
  assert.equal(unsupported.status, 400);
  await fetch(`${base}/test/deny`, { method: 'POST', headers: { authorization } });
  for (const client of clients) {
    assert.deepEqual((await client.listTools()).tools, []);
    assert.deepEqual((await client.listResources()).resources, []);
    assert.equal((await client.callTool({ name: 'function_status_list', arguments: {} })).isError, true);
    await assert.rejects(client.readResource({ uri }));
  }
  console.log('auth/Origin/Host on all routes, retained REST envelope, version errors and live policy denial pass');
} finally {
  for (const client of clients) await client.close();
}
