import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import { setTimeout as delay } from 'node:timers/promises';
import { Client as ModernClient } from '@modelcontextprotocol/client';
import { StdioClientTransport as ModernStdioTransport } from '@modelcontextprotocol/client/stdio';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

const execFileAsync = promisify(execFile);
const binPath = new URL('../target/debug/scp-cli-stdio-slice0', import.meta.url).pathname;
const inspectorLauncher = new URL(
  '../node_modules/@modelcontextprotocol/inspector/clients/launcher/build/index.js',
  import.meta.url,
).pathname;

function processAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function waitUntil(timeoutMs, predicate) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await predicate()) return true;
    await delay(20);
  }
  return Boolean(await predicate());
}

function attachRawStdioHost(t, envOverrides = {}) {
  const child = spawn(binPath, ['serve-stdio'], {
    stdio: ['pipe', 'pipe', 'pipe'],
    env: { ...process.env, ...envOverrides },
  });
  const stderrChunks = [];
  const nonJsonStdoutLines = [];
  const messages = [];
  const waiters = new Map();
  let buffered = '';

  child.stderr.on('data', chunk => {
    stderrChunks.push(chunk.toString('utf8'));
  });

  const onStdoutData = chunk => {
    buffered += chunk.toString('utf8');
    let nlIndex;
    while ((nlIndex = buffered.indexOf('\n')) !== -1) {
      const line = buffered.slice(0, nlIndex).replace(/\r$/, '');
      buffered = buffered.slice(nlIndex + 1);
      if (line.length === 0) continue;
      let parsed;
      try {
        parsed = JSON.parse(line);
      } catch {
        nonJsonStdoutLines.push(line);
        continue;
      }
      messages.push(parsed);
      if (parsed.id !== undefined && waiters.has(parsed.id)) {
        const resolve = waiters.get(parsed.id);
        waiters.delete(parsed.id);
        resolve(parsed);
      }
    }
  };
  child.stdout.on('data', onStdoutData);

  const exitPromise = once(child, 'exit').then(([code, signal]) => ({ code, signal }));

  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill('SIGKILL');
      await exitPromise.catch(() => {});
    }
  });

  return {
    child,
    stderr: () => stderrChunks.join(''),
    nonJsonStdoutLines,
    messages,
    exitPromise,
    send(payload) {
      const line = typeof payload === 'string' ? payload : JSON.stringify(payload);
      child.stdin.write(`${line}\n`);
    },
    waitForId(id, timeoutMs = 4000) {
      const existing = messages.find(m => m.id === id);
      if (existing) return Promise.resolve(existing);
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          waiters.delete(id);
          reject(new Error(`Timed out waiting for response id=${id}. stderr=${stderrChunks.join('')}`));
        }, timeoutMs);
        waiters.set(id, msg => {
          clearTimeout(timer);
          resolve(msg);
        });
      });
    },
    pauseStdout() {
      child.stdout.off('data', onStdoutData);
      child.stdout.pause();
    },
    closeStdin() {
      child.stdin.end();
    },
  };
}

function wrapTransportTrace(transport) {
  const sent = [];
  const received = [];
  let wrappedHandler = transport.onmessage;
  Object.defineProperty(transport, 'onmessage', {
    configurable: true,
    enumerable: true,
    get() {
      return wrappedHandler;
    },
    set(handler) {
      wrappedHandler = handler
        ? (msg, extra) => {
            received.push(msg);
            return handler(msg, extra);
          }
        : undefined;
    },
  });
  const origSend = transport.send.bind(transport);
  transport.send = (msg, options) => {
    sent.push(msg);
    return origSend(msg, options);
  };
  return { sent, received };
}

for (const [era, ClientClass, TransportClass, options, revision] of [
  ['legacy', Client, StdioClientTransport, {}, '2025-11-25'],
  [
    'modern',
    ModernClient,
    ModernStdioTransport,
    { versionNegotiation: { mode: { pin: '2026-07-28' } } },
    '2026-07-28',
  ],
]) {
  test(`${era} official SDK (stdio): discovery, resource, success, denial and concurrent batch`, { timeout: 15000 }, async t => {
    const client = new ClientClass({ name: `stdio-slice0-${era}`, version: '1' }, { capabilities: {}, ...options });
    t.after(() => client.close());
    const transport = new TransportClass({
      command: binPath,
      args: ['serve-stdio'],
      stderr: 'pipe',
    });
    const trace = wrapTransportTrace(transport);

    await client.connect(transport);
    assert.equal(client.getServerVersion().name, 'scpctl-stdio-spike');

    if (era === 'modern') {
      assert.equal(client.getNegotiatedProtocolVersion(), revision);
      assert.equal(trace.sent.some(m => m.method === 'initialize'), false);
    } else {
      const initReply = trace.received.find(m => m.result?.protocolVersion);
      assert.equal(initReply?.result?.protocolVersion, revision);
      assert.equal(trace.sent.some(m => m.method === 'initialize'), true);
    }

    const tools = await client.listTools();
    assert.deepEqual(tools.tools.map(tool => tool.name).sort(), ['probe_echo', 'probe_hold']);
    if (era === 'modern') {
      assert.equal(
        trace.sent.some(
          m => m.params?._meta?.['io.modelcontextprotocol/protocolVersion'] === revision,
        ),
        true,
      );
    }

    const okRes = await client.callTool({ name: 'probe_echo', arguments: { value: 'hello' } });
    assert.deepEqual(okRes.structuredContent, { value: 'hello', noisyChildReaped: false });

    const denied = await client.callTool({ name: 'probe_echo', arguments: { value: 'deny' } });
    assert.equal(denied.isError, true);

    const resources = await client.listResources();
    assert.deepEqual(
      resources.resources.map(r => r.uri).sort(),
      ['probe://active', 'probe://status'],
    );
    const status = await client.readResource({ uri: 'probe://status' });
    assert.deepEqual(JSON.parse(status.contents[0].text), {
      transport: 'stdio',
      server: 'scpctl-stdio-spike',
    });

    const batch = await Promise.all(
      ['c0', 'c1', 'c2', 'c3'].map(value => client.callTool({ name: 'probe_echo', arguments: { value } })),
    );
    assert.deepEqual(
      batch.map(item => item.structuredContent.value),
      ['c0', 'c1', 'c2', 'c3'],
    );
    t.diagnostic(JSON.stringify({ era, revision, sentMethods: trace.sent.map(m => m.method) }));
  });
}

for (const era of ['modern', 'legacy']) {
  test(`Inspector CLI real client (stdio): ${era} discovery, call and resource`, { timeout: 25000 }, async t => {
    const tmpDir = await mkdtemp(path.join(tmpdir(), `scp-inspector-${era}-`));
    t.after(() => rm(tmpDir, { recursive: true, force: true }));

    const runInspector = async extraArgs => {
      const { stdout } = await execFileAsync(
        process.execPath,
        [
          inspectorLauncher,
          '--cli',
          '--transport',
          'stdio',
          '--protocol-era',
          era,
          '--format',
          'json',
          '--quiet',
          ...extraArgs,
          '--',
          binPath,
          'serve-stdio',
        ],
        {
          timeout: 15000,
          env: {
            ...process.env,
            MCP_CATALOG_PATH: path.join(tmpDir, 'catalog.json'),
            MCP_CLIENT_CONFIG_PATH: path.join(tmpDir, 'client.json'),
          },
        },
      );
      return JSON.parse(stdout);
    };

    const tools = await runInspector(['--method', 'tools/list']);
    assert.deepEqual(tools.result.tools.map(tool => tool.name).sort(), ['probe_echo', 'probe_hold']);

    const called = await runInspector([
      '--method',
      'tools/call',
      '--tool-name',
      'probe_echo',
      '--tool-args-json',
      '{"value":"inspector"}',
    ]);
    assert.deepEqual(called.result.structuredContent, {
      value: 'inspector',
      noisyChildReaped: false,
    });

    const resource = await runInspector(['--method', 'resources/read', '--uri', 'probe://status']);
    assert.deepEqual(JSON.parse(resource.result.contents[0].text), {
      transport: 'stdio',
      server: 'scpctl-stdio-spike',
    });
    t.diagnostic(JSON.stringify({ era, tools, called, resource }));
  });
}

test('modern raw server/discover followed by stdin EOF exits 0 without error', { timeout: 10000 }, async t => {
  const host = attachRawStdioHost(t);
  host.send({
    jsonrpc: '2.0',
    id: 1,
    method: 'server/discover',
    params: {
      _meta: {
        'io.modelcontextprotocol/protocolVersion': '2026-07-28',
        'io.modelcontextprotocol/clientInfo': { name: 'raw-discover', version: '1' },
        'io.modelcontextprotocol/clientCapabilities': {},
      },
    },
  });
  const resp = await host.waitForId(1);
  assert.equal(resp.result._meta['io.modelcontextprotocol/serverInfo'].name, 'scpctl-stdio-spike');
  assert.equal(resp.result.supportedVersions.includes('2026-07-28'), true);
  host.closeStdin();
  const exited = await host.exitPromise;
  assert.equal(exited.code, 0, `expected clean exit 0 on discover+EOF, got stderr=${host.stderr()}`);
  assert.equal(host.nonJsonStdoutLines.length, 0);
});

for (const [era, ClientClass, TransportClass, options] of [
  ['legacy', Client, StdioClientTransport, {}],
  ['modern', ModernClient, ModernStdioTransport, { versionNegotiation: { mode: { pin: '2026-07-28' } } }],
]) {
  test(`${era} (stdio): AbortController cancellation releases probe_hold server work`, { timeout: 15000 }, async t => {
    const client = new ClientClass({ name: `stdio-cancel-${era}`, version: '1' }, { capabilities: {}, ...options });
    t.after(() => client.close());
    await client.connect(
      new TransportClass({
        command: binPath,
        args: ['serve-stdio'],
        stderr: 'pipe',
      }),
    );

    const active = async () =>
      JSON.parse((await client.readResource({ uri: 'probe://active' })).contents[0].text).active;

    const controller = new AbortController();
    const args = { name: 'probe_hold', arguments: { value: 'wait' } };
    const pending =
      era === 'modern'
        ? client.callTool(args, { signal: controller.signal })
        : client.callTool(args, undefined, { signal: controller.signal });
    const rejected = assert.rejects(pending);

    assert.equal(await waitUntil(2000, async () => (await active()) === 1), true);
    controller.abort();
    await rejected;
    assert.equal(
      await waitUntil(2000, async () => (await active()) === 0),
      true,
      'server must cancel probe_hold task rather than keeping worker occupied',
    );
  });
}

test('stdout stays pure NDJSON with noisy PTY+pipe children and recovers from invalid/oversized frames', { timeout: 15000 }, async t => {
  const host = attachRawStdioHost(t, { SCP_STDIO_MAX_LINE_BYTES: '4096' });

  // Unsupported inline modern version before session lifecycle selection returns -32022.
  host.send({
    jsonrpc: '2.0',
    id: 78,
    method: 'tools/call',
    params: {
      name: 'probe_echo',
      arguments: { value: 'safe' },
      _meta: {
        'io.modelcontextprotocol/protocolVersion': '2099-01-01',
        'io.modelcontextprotocol/clientInfo': { name: 'raw-framing', version: '1' },
        'io.modelcontextprotocol/clientCapabilities': {},
      },
    },
  });
  const badVer = await host.waitForId(78);
  assert.equal(badVer.error.code, -32022);

  host.send({
    jsonrpc: '2.0',
    id: 1,
    method: 'initialize',
    params: {
      protocolVersion: '2025-11-25',
      capabilities: {},
      clientInfo: { name: 'raw-framing', version: '1' },
    },
  });
  await host.waitForId(1);
  host.send({ jsonrpc: '2.0', method: 'notifications/initialized' });

  host.send({
    jsonrpc: '2.0',
    id: 2,
    method: 'tools/call',
    params: { name: 'probe_echo', arguments: { value: 'noisy-child' } },
  });
  const noisyResp = await host.waitForId(2);
  assert.deepEqual(noisyResp.result.structuredContent, {
    value: 'noisy-child',
    noisyChildReaped: true,
  });

  // Syntax-invalid JSON is ignored cleanly without terminating the transport.
  host.send('{not-json');

  // Structurally invalid JSON-RPC returns -32600 (rmcp emits id: null for unparseable Request envelopes).
  host.send({ jsonrpc: '2.0', id: 77, method: 123 });
  const gotBadRpc = await waitUntil(2000, () => host.messages.some(m => m.error?.code === -32600));
  assert.equal(gotBadRpc, true, 'expected -32600 error response for structurally invalid JSON-RPC');

  // Wrong tool argument type returns -32602.
  host.send({
    jsonrpc: '2.0',
    id: 79,
    method: 'tools/call',
    params: { name: 'probe_echo', arguments: { value: 42 } },
  });
  const badArg = await host.waitForId(79);
  assert.equal(badArg.error.code, -32602);

  // Oversized line (> 4096 bytes) is discarded without corrupting subsequent frames.
  host.send(
    JSON.stringify({
      jsonrpc: '2.0',
      id: 80,
      method: 'tools/call',
      params: { name: 'probe_echo', arguments: { value: 'Z'.repeat(6000) } },
    }),
  );
  host.send({
    jsonrpc: '2.0',
    id: 81,
    method: 'tools/call',
    params: { name: 'probe_echo', arguments: { value: 'after-oversized' } },
  });
  const recovered = await host.waitForId(81);
  assert.deepEqual(recovered.result.structuredContent, {
    value: 'after-oversized',
    noisyChildReaped: false,
  });

  host.closeStdin();
  const exited = await host.exitPromise;
  assert.equal(exited.code, 0);
  assert.deepEqual(host.nonJsonStdoutLines, []);
});

test('worker saturation + stdin EOF exits within 2s and reaps supervised child PIDs', { timeout: 10000 }, async t => {
  const host = attachRawStdioHost(t, { SCP_STDIO_MAX_WORKERS: '2' });
  const spawnedPids = [];
  t.after(() => {
    for (const pid of spawnedPids) {
      if (processAlive(pid)) {
        try {
          process.kill(pid, 'SIGKILL');
        } catch {}
      }
    }
  });

  host.send({
    jsonrpc: '2.0',
    id: 1,
    method: 'initialize',
    params: {
      protocolVersion: '2025-11-25',
      capabilities: {},
      clientInfo: { name: 'raw-saturation', version: '1' },
    },
  });
  await host.waitForId(1);
  host.send({ jsonrpc: '2.0', method: 'notifications/initialized' });

  for (const id of [10, 11, 12]) {
    host.send({
      jsonrpc: '2.0',
      id,
      method: 'tools/call',
      params: { name: 'probe_hold', arguments: { value: 'hold-child' } },
    });
  }

  let probeSeq = 100;
  let lastSnapshot = { active: 0, childPids: [] };
  const saturated = await waitUntil(3000, async () => {
    const id = probeSeq++;
    host.send({
      jsonrpc: '2.0',
      id,
      method: 'resources/read',
      params: { uri: 'probe://active' },
    });
    const res = await host.waitForId(id);
    lastSnapshot = JSON.parse(res.result.contents[0].text);
    return lastSnapshot.active === 2 && lastSnapshot.childPids.length === 2;
  });
  assert.equal(saturated, true, `expected 2 saturated workers with child PIDs, got ${JSON.stringify(lastSnapshot)}`);
  spawnedPids.push(...lastSnapshot.childPids);

  const closeStart = Date.now();
  host.closeStdin();
  const exited = await host.exitPromise;
  const elapsedMs = Date.now() - closeStart;
  assert.equal(exited.code, 0);
  assert.ok(elapsedMs <= 2000, `EOF shutdown exceeded 2000ms (${elapsedMs}ms)`);

  const allReaped = await waitUntil(1500, () => spawnedPids.every(pid => !processAlive(pid)));
  assert.equal(allReaped, true, `supervised child PIDs leaked after EOF: ${spawnedPids.join(',')}`);
});

test('stalled stdout writer aborts within 2.5s and reaps supervised child PIDs', { timeout: 10000 }, async t => {
  const host = attachRawStdioHost(t, { SCP_STDIO_WRITE_TIMEOUT_MS: '400' });
  const trackedPids = [];
  t.after(() => {
    for (const pid of trackedPids) {
      if (processAlive(pid)) {
        try {
          process.kill(pid, 'SIGKILL');
        } catch {}
      }
    }
  });

  host.send({
    jsonrpc: '2.0',
    id: 1,
    method: 'initialize',
    params: {
      protocolVersion: '2025-11-25',
      capabilities: {},
      clientInfo: { name: 'raw-stall', version: '1' },
    },
  });
  await host.waitForId(1);
  host.send({ jsonrpc: '2.0', method: 'notifications/initialized' });

  host.send({
    jsonrpc: '2.0',
    id: 2,
    method: 'tools/call',
    params: { name: 'probe_hold', arguments: { value: 'hold-child' } },
  });

  let seq = 50;
  let captured = [];
  const hasChild = await waitUntil(3000, async () => {
    const id = seq++;
    host.send({
      jsonrpc: '2.0',
      id,
      method: 'resources/read',
      params: { uri: 'probe://active' },
    });
    const res = await host.waitForId(id);
    const snapshot = JSON.parse(res.result.contents[0].text);
    captured = snapshot.childPids;
    return snapshot.active === 1 && captured.length === 1;
  });
  assert.equal(hasChild, true, 'expected supervised child PID before inducing writer stall');
  trackedPids.push(...captured);

  host.pauseStdout();
  const stallStart = Date.now();
  const bigValue = 'X'.repeat(65_536);
  for (let i = 0; i < 6; i++) {
    host.send({
      jsonrpc: '2.0',
      id: 200 + i,
      method: 'tools/call',
      params: { name: 'probe_echo', arguments: { value: bigValue } },
    });
  }

  const exited = await host.exitPromise;
  const elapsedMs = Date.now() - stallStart;
  assert.notEqual(exited.code, 0, 'stalled stdout host must abort with non-zero exit code');
  assert.ok(elapsedMs <= 2500, `stalled writer took too long to abort: ${elapsedMs}ms`);

  const reaped = await waitUntil(1500, () => trackedPids.every(pid => !processAlive(pid)));
  assert.equal(reaped, true, `supervised child PID leaked after writer stall: ${trackedPids.join(',')}`);
});
