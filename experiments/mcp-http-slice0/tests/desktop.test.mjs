// Run by the desktop Rust integration test against its real protocol router.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
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
const fixtures = JSON.parse(readFileSync(new URL('./desktop-surface.json', import.meta.url)));
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
    assert.deepEqual(tools.map(t => t.name).sort(), Object.keys(fixtures).filter(n => !fixtures[n].unavailable).sort());
    assert.equal((await client.listResources()).resources.length, 5);
    for (const [name, fixture] of Object.entries(fixtures)) {
      const tool = tools.find(t => t.name === name);
      if (fixture.unavailable) {
        const denied = await client.callTool({ name, arguments: fixture.args });
        assert.equal(denied.isError, true, name);
        assert.match(denied.content[0].text, /CAPABILITY_UNAVAILABLE/);
        continue;
      }
      assert.equal(tool.inputSchema.additionalProperties, false, name);
      const result = await client.callTool({ name, arguments: fixture.args });
      assert.notEqual(result.isError, true, name);
      const expected = name === 'function_status_list' ? { statuses: fixture.data } : { result: fixture.data };
      assert.deepEqual(result.structuredContent, expected, name);
      assert.deepEqual(JSON.parse(result.content[0].text), expected, name);
      await assert.rejects(client.callTool({ name, arguments: { ...fixture.args, typo: true } }), /Invalid|invalid/, name);
      for (const key of tool.inputSchema.required) {
        const missing = { ...fixture.args }; delete missing[key];
        await assert.rejects(client.callTool({ name, arguments: missing }), /Invalid|invalid/, `${name}: missing ${key}`);
        await assert.rejects(client.callTool({ name, arguments: { ...fixture.args, [key]: null } }), /Invalid|invalid/, `${name}: null ${key}`);
      }
      if (fixture.uri) {
        const resource = (await client.readResource({ uri: fixture.uri })).contents[0];
        assert.equal(resource.mimeType, 'application/json');
        assert.deepEqual(JSON.parse(resource.text), expected, fixture.uri);
      }
    }
    await assert.rejects(client.callTool({ name: 'unknown_operation', arguments: {} }));
    for (const [name, args] of [
      ['config_save_partial', { patch: { cc: { enabled: true, agentToken: 'secret' } } }],
      ['config_save_partial', { patch: { openvpn: { command: 'arbitrary command' } } }],
      ['config_save', { config: { app: { defaultTerminalCols: 120, defaultTerminalRows: 36 }, mcpEnabled: true } }],
      ['sync_settings_save_partial', { patch: { repoRoot: '/tmp/config', mcpSensitiveActionPolicy: 'allow-with-token' } }],
    ]) await assert.rejects(client.callTool({ name, arguments: args }), /Invalid|invalid/);

    assert.equal(tools.find(t => t.name === 'function_status_list').outputSchema.properties.statuses.type, 'array');
    const result = await client.callTool({ name: 'function_status_list', arguments: {} });
    assert.deepEqual(result.structuredContent, { statuses });
    assert.match(types[0], era === 'modern' && process.env.SLICE2_MODE === 'json' ? /application\/json/ : /text\/event-stream/);
    assert.deepEqual(JSON.parse(result.content[0].text), result.structuredContent);
    assert.ok((await client.listResources()).resources.some(r => r.uri === uri));
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
  for (const mode of ['failure', 'invalid-output']) {
    await fetch(`${base}/test/${mode}`, { method: 'POST', headers: { authorization } });
    for (const client of clients) {
      for (const [name, fixture] of Object.entries(fixtures).filter(([, f]) => !f.unavailable)) {
        const failed = await client.callTool({ name, arguments: fixture.args });
        assert.equal(failed.isError, true, `${mode}: ${name}`);
        assert.match(failed.content[0].text, /OPERATION_FAILED/);
        assert.ok(!JSON.stringify(failed).includes('unlabelled-secret'));
        if (fixture.uri) await assert.rejects(client.readResource({ uri: fixture.uri }), /OPERATION_FAILED/);
      }
    }
  }
  await fetch(`${base}/test/approval`, { method: 'POST', headers: { authorization } });
  for (const client of clients) {
    const { tools } = await client.listTools();
    assert.ok(tools.length > 0);
    assert.ok(tools.every(t => t.annotations.readOnlyHint));
    assert.ok(!tools.some(t => t.name === 'server_tunnel_start'));
    const blocked = await client.callTool({ name: 'server_tunnel_start', arguments: fixtures.server_tunnel_start.args });
    assert.equal(blocked.isError, true);
    assert.match(blocked.content[0].text, /POLICY_DENIED/);
    assert.deepEqual((await client.callTool({ name: 'function_status_list', arguments: {} })).structuredContent, { statuses });
  }
  await fetch(`${base}/test/deny`, { method: 'POST', headers: { authorization } });
  for (const client of clients) {
    assert.deepEqual((await client.listTools()).tools, []);
    assert.deepEqual((await client.listResources()).resources, []);
    for (const [name, fixture] of Object.entries(fixtures)) {
      assert.equal((await client.callTool({ name, arguments: fixture.args })).isError, true, name);
      if (fixture.uri) await assert.rejects(client.readResource({ uri: fixture.uri }));
    }
  }
  console.log('auth/Origin/Host on all routes, retained REST envelope, version errors and live policy denial pass');
} finally {
  for (const client of clients) await client.close();
}
