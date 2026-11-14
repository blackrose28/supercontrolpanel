import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { once } from 'node:events';
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { promisify } from 'node:util';
import { Client as ModernClient } from '@modelcontextprotocol/client';
import { StdioClientTransport as ModernStdioTransport } from '@modelcontextprotocol/client/stdio';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { ResourceUpdatedNotificationSchema } from '@modelcontextprotocol/sdk/types.js';

const execFileAsync = promisify(execFile);
const scpctlBin = process.env.SCPCTL_BIN
  ? path.resolve(process.env.SCPCTL_BIN)
  : new URL('../../../target/debug/scpctl', import.meta.url).pathname;
const sshdFixtureBin = process.env.SSHD_FIXTURE_BIN
  ? path.resolve(process.env.SSHD_FIXTURE_BIN)
  : new URL('../../../target/debug/examples/sshd_fixture_daemon', import.meta.url).pathname;
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
    await delay(25);
  }
  return Boolean(await predicate());
}

async function reserveFreePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const addr = srv.address();
      const port = typeof addr === 'object' && addr ? addr.port : 0;
      srv.close(err => (err ? reject(err) : resolve(port)));
    });
  });
}

async function startSshdFixture(t) {
  const daemon = spawn(sshdFixtureBin, [], { stdio: ['pipe', 'pipe', 'pipe'] });
  t.after(async () => {
    try {
      daemon.stdin.destroy();
    } catch {}
    try {
      daemon.stdout.destroy();
    } catch {}
    try {
      daemon.stderr.destroy();
    } catch {}
    if (daemon.exitCode === null) {
      const done = await Promise.race([
        once(daemon, 'exit').then(() => true),
        delay(500).then(() => false),
      ]);
      if (!done && daemon.exitCode === null) {
        daemon.kill('SIGKILL');
      }
    }
  });

  const firstLine = await new Promise((resolve, reject) => {
    let line = '';
    const onData = chunk => {
      line += chunk.toString('utf8');
      const nl = line.indexOf('\n');
      if (nl !== -1) {
        cleanup();
        resolve(line.slice(0, nl).trim());
      }
    };
    const onExit = ([code]) => {
      cleanup();
      reject(new Error(`sshd_fixture_daemon exited early with code ${code}`));
    };
    const timer = setTimeout(() => {
      cleanup();
      reject(new Error('timed out waiting for sshd_fixture_daemon descriptor line'));
    }, 5000);
    const cleanup = () => {
      clearTimeout(timer);
      daemon.stdout.off('data', onData);
      daemon.off('exit', onExit);
    };
    daemon.stdout.on('data', onData);
    daemon.once('exit', onExit);
  });
  assert.ok(firstLine.length > 0, 'sshd_fixture_daemon must print descriptor JSON line');
  return { daemon, fx: JSON.parse(firstLine) };
}

async function createIsolatedWorkspace(t, fx, opts = {}) {
  const root = await mkdtemp(path.join(tmpdir(), 'scp-slice4-e2e-'));
  t.after(() => rm(root, { recursive: true, force: true }));

  const cliRoot = path.join(root, 'cli_root');
  const desktopDir = path.join(root, 'desktop');
  const profileDir = path.join(cliRoot, 'profiles', 'stdio-s4');
  const caddyDir = path.join(root, 'caddy-site');
  const repoDir = path.join(root, 'sync-repo');
  const binDir = path.join(root, 'bin');
  const ovpnFile = path.join(root, 'client.ovpn');
  const vpnLockPath = path.join(root, 'vpn.lock');
  const keyringFile = path.join(root, 'keyring.json');
  const pidsLog = path.join(root, 'pids.log');
  const knownHostsPath = path.join(root, 'known_hosts');

  await Promise.all([
    mkdir(profileDir, { recursive: true }),
    mkdir(desktopDir, { recursive: true }),
    mkdir(caddyDir, { recursive: true }),
    mkdir(repoDir, { recursive: true }),
    mkdir(binDir, { recursive: true }),
  ]);

  const tunnelPort = await reserveFreePort();
  const khLines = [
    `[127.0.0.1]:${fx.port} ${fx.hostPubAlgoAndKey}`,
    `[127.0.0.1]:${tunnelPort} ${fx.hostPubAlgoAndKey}`,
  ];
  await writeFile(knownHostsPath, `${khLines.join('\n')}\n`);
  await writeFile(path.join(caddyDir, 'Caddyfile'), ':0 {\n}\n');
  await writeFile(ovpnFile, 'client\nremote 127.0.0.1 1194\n');

  const fakeBoundary = path.join(binDir, 'fake-boundary');
  const fakeCaddy = path.join(binDir, 'fake-caddy');
  const fakeVpn = path.join(binDir, 'fake-openvpn');
  const fakeSudo = path.join(binDir, 'fake-sudo');

  await writeFile(
    fakeBoundary,
    `#!/usr/bin/env python3
import json, os, signal, sys, time
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
args = sys.argv[1:]
if args[:2] == ["targets", "list"]:
    print(json.dumps({"items": [{"id": "ttcp_1", "name": "Prod-Box", "type": "tcp"}]}))
    sys.exit(0)
if args[:2] == ["authenticate", "password"]:
    _pw = sys.stdin.read()
    print(json.dumps({"item": {"attributes": {"token": "tbat_s4_e2e_tok"}}}))
    sys.exit(0)
if args and args[0] == "connect":
    with open(${JSON.stringify(pidsLog)}, "a", encoding="utf-8") as f:
        f.write(f"boundary:{os.getpid()}\\n")
    os.execv(${JSON.stringify(fx.boundaryScript)}, [${JSON.stringify(fx.boundaryScript)}, *args])
time.sleep(30)
`,
  );

  // Emit raw ANSI/CRLF noise onto child output to assert MCP stdout isolation.
  await writeFile(
    fakeCaddy,
    `#!/usr/bin/env python3
import os, signal, socket, sys, time
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
with open(${JSON.stringify(pidsLog)}, "a", encoding="utf-8") as f:
    f.write(f"caddy:{os.getpid()}\\n")
s = socket.socket()
s.bind(("127.0.0.1", 0))
s.listen(1)
sys.stdout.write("\\x1b[31mnoisy caddy stderr/pty prelude\\x1b[0m\\r\\n")
sys.stdout.write("serving initial configuration\\r\\n")
sys.stdout.write("\\x1b[32mCaddy ready JSON-trap: {not-mcp-json}\\x1b[0m\\r\\n")
sys.stdout.flush()
time.sleep(30)
`,
  );

  await writeFile(
    fakeVpn,
    `#!/bin/sh
echo "openvpn:$$" >> ${JSON.stringify(pidsLog)}
trap 'exit 0' TERM INT
echo "Enter Auth Username:"
read _u
echo "Enter Auth Password:"
read _p
echo "Initialization Sequence Completed"
while true; do sleep 1; done
`,
  );

  await writeFile(
    fakeSudo,
    `#!/bin/sh
if [ "$1" = "-n" ] && [ "$2" = "true" ]; then exit 0; fi
if [ "$1" = "-n" ]; then shift; fi
exec "$@"
`,
  );

  await Promise.all([
    chmod(fakeBoundary, 0o755),
    chmod(fakeCaddy, 0o755),
    chmod(fakeVpn, 0o755),
    chmod(fakeSudo, 0o755),
  ]);

  if (opts.slowScpMatch) {
    const slowScp = path.join(binDir, 'scp');
    const slowPidFile = path.join(root, 'slow-scp.pid');
    await writeFile(
      slowScp,
      `#!/bin/sh
case "$*" in
  *${opts.slowScpMatch}*)
    echo $$ > ${JSON.stringify(slowPidFile)}
    trap 'exit 0' TERM INT
    while true; do sleep 1; done
    ;;
esac
exec /usr/bin/scp "$@"
`,
    );
    await chmod(slowScp, 0o755);
  }

  await writeFile(
    path.join(profileDir, 'config.json'),
    JSON.stringify(
      {
        app: { defaultTerminalCols: 100, defaultTerminalRows: 30 },
        openvpn: { username: 'alice', extraArgs: '' },
        boundary: {
          addr: 'https://boundary.example',
          authMethodId: 'ampw_1',
          loginName: 'alice',
          authKind: 'password',
          connectSshOptions: '',
        },
      },
      null,
      2,
    ),
  );

  await writeFile(
    path.join(profileDir, 'servers.json'),
    JSON.stringify(
      {
        servers: [
          {
            id: 'srv-fx',
            name: 'Fixture-Direct',
            host: '127.0.0.1',
            os: 'linux',
            connectionMethod: 'ssh',
            sshPort: fx.port,
            sshUser: fx.user,
            services: [],
          },
          {
            id: 'srv-bnd',
            name: 'Fixture-Boundary',
            host: '127.0.0.1',
            os: 'linux',
            connectionMethod: 'boundary',
            sshPort: fx.port,
            sshUser: fx.user,
            boundaryTargetId: 'ttcp_1',
            boundaryListenPort: tunnelPort,
            services: [],
          },
          ...(opts.extraServers || []),
        ],
      },
      null,
      2,
    ),
  );

  await writeFile(
    path.join(profileDir, 'sync-local.json'),
    JSON.stringify(
      {
        repoRoot: repoDir,
        openvpnExecutablePath: fakeVpn,
        openvpnOvpnPath: ovpnFile,
        boundaryCliPath: fakeBoundary,
        caddyFolder: caddyDir,
        mcpEnabled: true,
        mcpBindAddress: '127.0.0.1',
        mcpPort: 41999,
        mcpSensitiveActionPolicy: 'allow-with-token',
        mcpAllowedToolCategories: ['read-only'],
        mcpBearerToken: 'secret-mcp-token-e2e',
        ccEnabled: false,
        ccAgentToken: 'secret-cc-token-e2e',
        firstRunCompleted: true,
      },
      null,
      2,
    ),
  );

  if (opts.stdioPolicy) {
    await writeFile(
      path.join(profileDir, 'stdio-policy.json'),
      JSON.stringify(opts.stdioPolicy, null, 2),
    );
  }

  const env = {
    ...process.env,
    PATH: `${binDir}:${process.env.PATH || ''}`,
    SCPCTL_DESKTOP_CONFIG_DIR: desktopDir,
    SCPCTL_TEST_KEYRING_FILE: keyringFile,
    SCPCTL_SSH_KNOWN_HOSTS: knownHostsPath,
    SCPCTL_SSH_IDENTITY_FILE: fx.userKey,
    SCPCTL_CADDY_PATH: fakeCaddy,
    SCPCTL_SUDO_PATH: fakeSudo,
    SCPCTL_VPN_LOCK_PATH: vpnLockPath,
    SCPCTL_OPENVPN_CANDIDATES: fakeVpn,
    ...opts.extraEnv,
  };

  return {
    root,
    cliRoot,
    desktopDir,
    profileDir,
    caddyDir,
    repoDir,
    ovpnFile,
    keyringFile,
    pidsLog,
    tunnelPort,
    fakeBoundary,
    fakeCaddy,
    fakeVpn,
    slowPidFile: path.join(root, 'slow-scp.pid'),
    env,
  };
}

async function readTrackedPids(pidsLog) {
  try {
    const raw = await readFile(pidsLog, 'utf8');
    return raw
      .split('\n')
      .map(line => line.trim())
      .filter(Boolean)
      .map(line => {
        const [kind, pidStr] = line.split(':');
        return { kind, pid: Number(pidStr) };
      })
      .filter(entry => Number.isInteger(entry.pid) && entry.pid > 0);
  } catch {
    return [];
  }
}

function attachRawStdioHost(t, ws, extraEnv = {}, spawnOpts = {}) {
  const child = spawn(
    scpctlBin,
    ['--config-dir', ws.cliRoot, '--profile', 'stdio-s4', 'mcp', 'serve', '--stdio'],
    {
      stdio: ['pipe', 'pipe', 'pipe'],
      env: { ...ws.env, ...extraEnv },
      ...spawnOpts,
    },
  );
  const exitPromise = once(child, 'exit').then(([code, signal]) => ({ code, signal }));
  t.after(async () => {
    if (child.exitCode === null) {
      child.kill('SIGKILL');
      await exitPromise.catch(() => {});
    }
  });

  const nonJsonStdoutLines = [];
  const messages = [];
  const waiters = new Map();
  let buf = '';
  const stderrChunks = [];

  const rejectAllWaiters = reason => {
    for (const [id, entry] of waiters.entries()) {
      clearTimeout(entry.timer);
      entry.reject(new Error(`${reason} (id=${id}); stderr=${Buffer.concat(stderrChunks).toString('utf8')}`));
    }
    waiters.clear();
  };

  child.once('exit', (code, signal) => {
    rejectAllWaiters(`child exited (code=${code}, signal=${signal})`);
  });

  const onStdoutData = chunk => {
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
          const entry = waiters.get(msg.id);
          waiters.delete(msg.id);
          clearTimeout(entry.timer);
          entry.resolve(msg);
        }
      } catch {
        nonJsonStdoutLines.push(line);
      }
    }
  };
  child.stdout.on('data', onStdoutData);
  child.stderr.on('data', d => stderrChunks.push(d));

  t.after(async () => {
    rejectAllWaiters('test cleanup');
    try {
      child.stdin.destroy();
    } catch {}
    try {
      child.stdout.destroy();
    } catch {}
    try {
      child.stderr.destroy();
    } catch {}
  });

  return {
    child,
    exitPromise,
    messages,
    nonJsonStdoutLines,
    send(payload) {
      const raw = typeof payload === 'string' ? payload : JSON.stringify(payload);
      child.stdin.write(`${raw}\n`);
    },
    waitForId(id, timeoutMs = 5000) {
      const hit = messages.find(m => m.id === id);
      if (hit) return Promise.resolve(hit);
      if (child.exitCode !== null) {
        return Promise.reject(new Error(`child already exited (code=${child.exitCode}) before id=${id}`));
      }
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          waiters.delete(id);
          reject(new Error(`timeout waiting for id=${id}; stderr=${Buffer.concat(stderrChunks).toString('utf8')}`));
        }, timeoutMs);
        waiters.set(id, { timer, resolve, reject });
      });
    },
    pauseStdout() {
      child.stdout.off('data', onStdoutData);
      child.stdout.pause();
    },
    closeStdin() {
      child.stdin.end();
    },
    stderr() {
      return Buffer.concat(stderrChunks).toString('utf8');
    },
  };
}

test('Slice 4 production scpctl stdio: live policy reload, 41 workflows, host-neutral exclusions, SFTP/SCP/Boundary/Caddy/OpenVPN/Config & notifications', { timeout: 45000 }, async t => {
  const { fx } = await startSshdFixture(t);
  const ws = await createIsolatedWorkspace(t, fx);

  const notifications = [];
  const client = new Client({ name: 'slice4-sdk-e2e', version: '1' }, { capabilities: {} });
  t.after(async () => {
    try {
      await client.close();
    } catch {}
  });
  const transport = new StdioClientTransport({
    command: scpctlBin,
    args: ['--config-dir', ws.cliRoot, '--profile', 'stdio-s4', 'mcp', 'serve', '--stdio'],
    stderr: 'pipe',
    env: ws.env,
  });
  await client.connect(transport);
  client.setNotificationHandler(ResourceUpdatedNotificationSchema, notif => {
    notifications.push(notif.params.uri);
  });

  // 1. Default policy exposes 11 read-only tools.
  const defaultList = await client.listTools();
  assert.equal(defaultList.tools.length, 11);

  // 2. Live reload to permissive policy unlocks 41 tools without restarting scpctl.
  const policyPath = path.join(ws.profileDir, 'stdio-policy.json');
  const permissive = {
    sensitiveActionPolicy: 'allow-local-policy',
    allowedToolCategories: ['read-only', 'runtime-control', 'config-mutation', 'file-transfer'],
  };
  await writeFile(policyPath, JSON.stringify(permissive));

  const fullList = await client.listTools();
  assert.equal(fullList.tools.length, 41);

  // 3. Corrupt stdio-policy.json fails closed immediately.
  await writeFile(policyPath, '{corrupt');
  const corruptList = await client.listTools();
  assert.equal(corruptList.tools.length, 0);
  const deniedWhileCorrupt = await client.callTool({ name: 'servers_list', arguments: {} });
  assert.equal(deniedWhileCorrupt.isError, true);
  assert.match(deniedWhileCorrupt.content[0].text, /^POLICY_DENIED:/);
  assert.equal(deniedWhileCorrupt.content[0].text.toLowerCase().includes('desktop'), false);

  await writeFile(policyPath, JSON.stringify(permissive));

  // 4. Excluded operations remain CAPABILITY_UNAVAILABLE with host-neutral errors.
  for (const [name, args] of [
    ['server_rdp_start', { serverId: 'srv-fx' }],
    ['config_sync_pull', { repoRoot: ws.repoDir }],
    ['config_git_pull', { repoRoot: ws.repoDir }],
    ['config_git_push', { repoRoot: ws.repoDir }],
  ]) {
    const res = await client.callTool({ name, arguments: args });
    assert.equal(res.isError, true, `expected error for ${name}`);
    assert.match(res.content[0].text, /^CAPABILITY_UNAVAILABLE:/, name);
    assert.equal(res.content[0].text.toLowerCase().includes('desktop'), false, name);
  }

  // 5. Subscribe to status resource so Caddy start triggers invalidation notification.
  await client.subscribeResource({ uri: 'scp://status/functions' });

  // 6. Config save protection, keyring persistence, partial updates, sync push.
  const tampered = await client.callTool({
    name: 'config_save',
    arguments: {
      config: {
        app: { defaultTerminalCols: 100, defaultTerminalRows: 30 },
        openvpn: { executablePath: '/evil/openvpn' },
        boundary: { cliPath: ws.fakeBoundary },
      },
    },
  });
  assert.equal(tampered.isError, true);
  assert.match(tampered.content[0].text, /^POLICY_DENIED:/);
  assert.equal(tampered.content[0].text.toLowerCase().includes('desktop'), false);

  const savedCfg = await client.callTool({
    name: 'config_save',
    arguments: {
      config: {
        app: { defaultTerminalCols: 100, defaultTerminalRows: 30 },
        openvpn: {
          command: '',
          executablePath: ws.fakeVpn,
          ovpnPath: ws.ovpnFile,
          extraArgs: '',
          username: 'alice',
          password: 'vpn-s4-keyring-secret',
        },
        boundary: {
          addr: 'https://boundary.example',
          authMethodId: 'ampw_1',
          loginName: 'alice',
          authKind: 'password',
          password: 'bnd-s4-keyring-secret',
          cliPath: ws.fakeBoundary,
          connectSshOptions: '',
        },
      },
    },
  });
  assert.equal(Boolean(savedCfg.isError), false, JSON.stringify(savedCfg));
  const diskConfigRaw = await readFile(path.join(ws.profileDir, 'config.json'), 'utf8');
  assert.equal(diskConfigRaw.includes('vpn-s4-keyring-secret'), false);
  assert.equal(diskConfigRaw.includes('bnd-s4-keyring-secret'), false);

  const patchedCfg = await client.callTool({
    name: 'config_save_partial',
    arguments: { patch: { app: { defaultTerminalCols: 112 } } },
  });
  assert.equal(patchedCfg.structuredContent.result.app.defaultTerminalCols, 112);

  const syncPatched = await client.callTool({
    name: 'sync_settings_save_partial',
    arguments: { patch: { repoRoot: ws.repoDir } },
  });
  assert.equal(syncPatched.structuredContent.result.repoRoot, ws.repoDir);

  const syncPushed = await client.callTool({
    name: 'config_sync_push',
    arguments: { repoRoot: ws.repoDir },
  });
  assert.equal(Boolean(syncPushed.isError), false);
  const pushedFiles = await readFile(path.join(ws.repoDir, 'app-config', 'servers.json'), 'utf8');
  assert.ok(pushedFiles.includes('srv-fx'));

  // 7. Boundary targets + auth, OpenVPN detect + start/stop, noisy Caddy start/stop.
  const targets = await client.callTool({ name: 'boundary_search_targets', arguments: {} });
  assert.equal(targets.structuredContent.result[0].id, 'ttcp_1');

  const detectedVpn = await client.callTool({ name: 'detect_openvpn_executable', arguments: {} });
  assert.equal(detectedVpn.structuredContent.result.path, ws.fakeVpn);

  const bndStart = await client.callTool({ name: 'boundary_auth_start', arguments: {} });
  assert.equal(bndStart.structuredContent.result.functionId, 'boundary-auth');
  const bndStop = await client.callTool({ name: 'boundary_auth_stop', arguments: {} });
  assert.equal(Boolean(bndStop.isError), false);

  const vpnStart = await client.callTool({ name: 'openvpn_start', arguments: {} });
  assert.equal(vpnStart.structuredContent.result.functionId, 'openvpn-connect');
  const vpnStop = await client.callTool({ name: 'openvpn_stop', arguments: {} });
  assert.equal(Boolean(vpnStop.isError), false);

  notifications.length = 0;
  const caddyStart = await client.callTool({
    name: 'caddy_start',
    arguments: { folder: ws.caddyDir },
  });
  assert.equal(caddyStart.structuredContent.result.functionId, 'caddy-run');
  assert.equal(
    await waitUntil(2500, () => notifications.includes('scp://status/functions')),
    true,
    'expected notifications/resources/updated after caddy_start',
  );
  const caddyStop = await client.callTool({ name: 'caddy_stop', arguments: {} });
  assert.equal(Boolean(caddyStop.isError), false);

  // 8. Server upsert, SSH password set/clear, SSH session start/stop, Boundary tunnel + SFTP + SCP.
  const upserted = await client.callTool({
    name: 'server_upsert',
    arguments: {
      server: {
        id: 'srv-extra',
        name: 'Extra',
        host: '10.0.0.5',
        connectionMethod: 'ssh',
        sshPort: 22,
        sshUser: 'ops',
      },
    },
  });
  assert.equal(upserted.structuredContent.result.some(s => s.id === 'srv-extra'), true);

  const pwSet = await client.callTool({
    name: 'server_ssh_password_set',
    arguments: { serverId: 'srv-fx', password: 'ssh-temp-secret' },
  });
  assert.equal(Boolean(pwSet.isError), false);
  const pwClear = await client.callTool({
    name: 'server_ssh_password_clear',
    arguments: { serverId: 'srv-fx' },
  });
  assert.equal(Boolean(pwClear.isError), false);

  const sshStarted = await client.callTool({
    name: 'server_ssh_start',
    arguments: { serverId: 'srv-fx' },
  });
  assert.equal(sshStarted.structuredContent.result.functionId, 'server-ssh-srv-fx');
  const sshStopped = await client.callTool({
    name: 'server_ssh_stop',
    arguments: { serverId: 'srv-fx' },
  });
  assert.equal(Boolean(sshStopped.isError), false);

  const tunnelStarted = await client.callTool({
    name: 'server_tunnel_start',
    arguments: { serverId: 'srv-bnd' },
  });
  assert.equal(tunnelStarted.structuredContent.result.functionId, 'server-tunnel-srv-bnd');

  // Persistent SFTP browser over active Boundary tunnel.
  const opened = await client.callTool({
    name: 'remote_browser_open',
    arguments: { serverId: 'srv-bnd' },
  });
  const browserId = opened.structuredContent.result.browserId;
  const remoteWork = path.join(fx.remoteSandbox, 'mcp-work');
  await client.callTool({
    name: 'remote_browser_create_directory',
    arguments: { browserId, path: remoteWork },
  });
  const statDir = await client.callTool({
    name: 'remote_browser_stat_path',
    arguments: { browserId, path: remoteWork },
  });
  assert.equal(statDir.structuredContent.result.exists, true);
  assert.equal(statDir.structuredContent.result.kind, 'directory');

  const localUpload = path.join(ws.root, 'upload-sample.txt');
  await writeFile(localUpload, 'hello slice 4 stdio\n');

  // Profile-dir files must be blocked from SCP transfer.
  const blockedUpload = await client.callTool({
    name: 'scp_upload',
    arguments: {
      serverId: 'srv-bnd',
      localPath: path.join(ws.profileDir, 'config.json'),
      remotePath: remoteWork,
    },
  });
  assert.equal(blockedUpload.isError, true);
  assert.match(blockedUpload.content[0].text, /^POLICY_DENIED:/);

  const uploaded = await client.callTool({
    name: 'scp_upload',
    arguments: { serverId: 'srv-bnd', localPath: localUpload, remotePath: remoteWork },
  });
  assert.equal(Boolean(uploaded.isError), false, JSON.stringify(uploaded));

  const children = await client.callTool({
    name: 'remote_browser_list_children',
    arguments: { browserId, path: remoteWork },
  });
  assert.equal(children.structuredContent.result.some(e => e.name === 'upload-sample.txt'), true);

  const dlDir = path.join(ws.root, 'downloads');
  await mkdir(dlDir, { recursive: true });
  const downloaded = await client.callTool({
    name: 'scp_download',
    arguments: {
      serverId: 'srv-bnd',
      remotePath: path.join(remoteWork, 'upload-sample.txt'),
      localPath: dlDir,
    },
  });
  assert.equal(Boolean(downloaded.isError), false, JSON.stringify(downloaded));
  assert.equal(await readFile(path.join(dlDir, 'upload-sample.txt'), 'utf8'), 'hello slice 4 stdio\n');

  // Existing local file target overwrite must return CONFLICT.
  const conflictDl = await client.callTool({
    name: 'scp_download',
    arguments: {
      serverId: 'srv-bnd',
      remotePath: path.join(remoteWork, 'upload-sample.txt'),
      localPath: path.join(dlDir, 'upload-sample.txt'),
    },
  });
  assert.equal(conflictDl.isError, true);
  assert.match(conflictDl.content[0].text, /^CONFLICT:/);

  // Async start + transfer tracking.
  const asyncUpFile = path.join(ws.root, 'async-sample.txt');
  await writeFile(asyncUpFile, 'async payload\n');
  const asyncUp = await client.callTool({
    name: 'scp_start_upload',
    arguments: { serverId: 'srv-bnd', localPath: asyncUpFile, remotePath: remoteWork },
  });
  assert.ok(asyncUp.structuredContent.result.sessionId.length > 0);
  await waitUntil(4000, async () => {
    const txs = await client.callTool({ name: 'scp_transfer_list', arguments: {} });
    const list = txs.structuredContent.result;
    return list.some(tx => tx.sessionId === asyncUp.structuredContent.result.sessionId && tx.state === 'success');
  });
  const txs = await client.callTool({ name: 'scp_transfer_list', arguments: {} });
  const latest = txs.structuredContent.result[0];
  const txGet = await client.callTool({
    name: 'scp_transfer_get',
    arguments: { transferId: latest.id },
  });
  assert.equal(txGet.structuredContent.result.id, latest.id);

  // Clean up SFTP + tunnel + server.
  await client.callTool({
    name: 'remote_browser_delete_path',
    arguments: { browserId, path: remoteWork, recursive: true },
  });
  await client.callTool({ name: 'remote_browser_close', arguments: { browserId } });
  await client.callTool({ name: 'server_tunnel_stop', arguments: { serverId: 'srv-bnd' } });
  const afterDel = await client.callTool({
    name: 'server_delete',
    arguments: { id: 'srv-extra' },
  });
  assert.equal(afterDel.structuredContent.result.some(s => s.id === 'srv-extra'), false);

  await client.close();
});

test('Slice 4 production scpctl stdio: modern @modelcontextprotocol/client (2.4.0)', { timeout: 15000 }, async t => {
  const { fx } = await startSshdFixture(t);
  const ws = await createIsolatedWorkspace(t, fx, {
    stdioPolicy: {
      sensitiveActionPolicy: 'allow-local-policy',
      allowedToolCategories: ['read-only', 'runtime-control', 'config-mutation', 'file-transfer'],
    },
  });

  const modern = new ModernClient(
    { name: 'slice4-modern-client', version: '1' },
    { capabilities: {}, versionNegotiation: { mode: { pin: '2026-07-28' } } },
  );
  t.after(async () => {
    try {
      await modern.close();
    } catch {}
  });
  await modern.connect(
    new ModernStdioTransport({
      command: scpctlBin,
      args: ['--config-dir', ws.cliRoot, '--profile', 'stdio-s4', 'mcp', 'serve', '--stdio'],
      stderr: 'pipe',
      env: ws.env,
    }),
  );
  assert.equal(modern.getNegotiatedProtocolVersion(), '2026-07-28');
  assert.equal((await modern.listTools()).tools.length, 41);
  assert.equal((await modern.listResources()).resources.length, 6);
  const overview = await modern.callTool({ name: 'app_get_overview', arguments: {} });
  assert.equal(overview.structuredContent.result.knownFunctionCount, 3);
  const fnSnap = await modern.readResource({ uri: 'scp://status/functions' });
  assert.equal(JSON.parse(fnSnap.contents[0].text).statuses.length, 3);
  await modern.close();
});

test('Slice 4 production scpctl stdio: Inspector CLI (2.14.0 modern and legacy)', { timeout: 25000 }, async t => {
  const { fx } = await startSshdFixture(t);
  const ws = await createIsolatedWorkspace(t, fx, {
    stdioPolicy: {
      sensitiveActionPolicy: 'allow-local-policy',
      allowedToolCategories: ['read-only', 'runtime-control', 'config-mutation', 'file-transfer'],
    },
  });

  const inspectorTmp = path.join(ws.root, 'inspector');
  await mkdir(inspectorTmp, { recursive: true });
  for (const era of ['modern', 'legacy']) {
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
          scpctlBin,
          '--config-dir',
          ws.cliRoot,
          '--profile',
          'stdio-s4',
          'mcp',
          'serve',
          '--stdio',
        ],
        {
          timeout: 15000,
          env: {
            ...ws.env,
            MCP_CATALOG_PATH: path.join(inspectorTmp, `catalog-${era}.json`),
            MCP_CLIENT_CONFIG_PATH: path.join(inspectorTmp, `client-${era}.json`),
          },
        },
      );
      return JSON.parse(stdout);
    };

    const listedTools = await runInspector(['--method', 'tools/list']);
    assert.equal(listedTools.result.tools.length, 41, `inspector ${era} tools/list`);

    const listedResources = await runInspector(['--method', 'resources/list']);
    assert.equal(listedResources.result.resources.length, 6, `inspector ${era} resources/list`);

    const called = await runInspector([
      '--method',
      'tools/call',
      '--tool-name',
      'app_get_overview',
      '--tool-args-json',
      '{}',
    ]);
    assert.equal(called.result.structuredContent.result.knownFunctionCount, 3);
  }
});

test('Slice 4 production scpctl stdio: AbortController cancels sync scp_upload and worker saturation returns CONFLICT while tunnel survives', { timeout: 30000 }, async t => {
  const { fx } = await startSshdFixture(t);
  const ws = await createIsolatedWorkspace(t, fx, {
    stdioPolicy: {
      sensitiveActionPolicy: 'allow-local-policy',
      allowedToolCategories: ['read-only', 'runtime-control', 'config-mutation', 'file-transfer'],
    },
    slowScpMatch: 'slow-upload.bin',
    extraEnv: { SCP_STDIO_MAX_WORKERS: '1' },
  });

  const client = new Client({ name: 'slice4-cancel', version: '1' }, { capabilities: {} });
  t.after(async () => {
    try {
      await client.close();
    } catch {}
  });
  await client.connect(
    new StdioClientTransport({
      command: scpctlBin,
      args: ['--config-dir', ws.cliRoot, '--profile', 'stdio-s4', 'mcp', 'serve', '--stdio'],
      stderr: 'pipe',
      env: ws.env,
    }),
  );

  // Start Boundary tunnel first; note PID recorded in pidsLog.
  const tun = await client.callTool({
    name: 'server_tunnel_start',
    arguments: { serverId: 'srv-bnd' },
  });
  assert.equal(tun.structuredContent.result.functionId, 'server-tunnel-srv-bnd');
  const pids = await readTrackedPids(ws.pidsLog);
  const tunnelPid = pids.find(p => p.kind === 'boundary')?.pid;
  assert.ok(tunnelPid && processAlive(tunnelPid), 'expected live boundary tunnel PID');

  const slowFile = path.join(ws.root, 'slow-upload.bin');
  await writeFile(slowFile, 'hold-worker');
  const remoteWork = path.join(fx.remoteSandbox, 'cancel-work');
  await mkdir(remoteWork, { recursive: true });

  const controller = new AbortController();
  const pendingUpload = client.callTool(
    {
      name: 'scp_upload',
      arguments: { serverId: 'srv-bnd', localPath: slowFile, remotePath: remoteWork },
    },
    undefined,
    { signal: controller.signal },
  );
  const rejected = assert.rejects(pendingUpload);

  assert.equal(
    await waitUntil(4000, async () => {
      try {
        const pid = Number((await readFile(ws.slowPidFile, 'utf8')).trim());
        return pid > 0 && processAlive(pid);
      } catch {
        return false;
      }
    }),
    true,
    'expected slow scp child PID occupying the single worker',
  );
  const slowScpPid = Number((await readFile(ws.slowPidFile, 'utf8')).trim());

  // With SCP_STDIO_MAX_WORKERS=1 saturated, a concurrent tool call must fail with CONFLICT.
  const saturated = await client.callTool({ name: 'servers_list', arguments: {} });
  assert.equal(saturated.isError, true);
  assert.match(saturated.content[0].text, /^CONFLICT:/);

  // Abort sync upload; SCP child must reap cleanly, worker must free up, and tunnel must survive.
  controller.abort();
  await rejected;
  assert.equal(
    await waitUntil(4000, () => !processAlive(slowScpPid)),
    true,
    `slow scp PID ${slowScpPid} leaked after AbortController cancellation`,
  );

  const recoveredList = await client.callTool({ name: 'servers_list', arguments: {} });
  assert.equal(Boolean(recoveredList.isError), false);
  const tunStatus = await client.callTool({
    name: 'function_status_get',
    arguments: { functionId: 'server-tunnel-srv-bnd' },
  });
  assert.equal(tunStatus.structuredContent.result.state, 'running');
  assert.equal(processAlive(tunnelPid), true, 'running tunnel must survive cancelled SCP');

  await client.close();
  assert.equal(
    await waitUntil(3000, () => !processAlive(tunnelPid)),
    true,
    'tunnel PID must be reaped when stdio session closes',
  );
});

test('Slice 4 production scpctl stdio: framing bounds, write-timeout stall, EOF/SIGTERM/SIGINT reaping, no-TTY, duplicate owner', { timeout: 45000 }, async t => {
  const { fx } = await startSshdFixture(t);

  // 1. Oversized frame recovery + pure NDJSON isolation + EOF reaping of Caddy child.
  {
    const ws = await createIsolatedWorkspace(t, fx, {
      stdioPolicy: {
        sensitiveActionPolicy: 'allow-local-policy',
        allowedToolCategories: ['read-only', 'runtime-control', 'config-mutation', 'file-transfer'],
      },
    });
    const host = attachRawStdioHost(t, ws, { SCP_STDIO_MAX_LINE_BYTES: '4096' });
    host.send({
      jsonrpc: '2.0',
      id: 1,
      method: 'initialize',
      params: {
        protocolVersion: '2025-11-25',
        capabilities: {},
        clientInfo: { name: 'raw-s4-framing', version: '1' },
      },
    });
    await host.waitForId(1);
    host.send({ jsonrpc: '2.0', method: 'notifications/initialized' });

    host.send({
      jsonrpc: '2.0',
      id: 2,
      method: 'tools/call',
      params: { name: 'caddy_start', arguments: { folder: ws.caddyDir } },
    });
    const started = await host.waitForId(2);
    assert.equal(started.result.isError, false);
    const caddyPid = (await readTrackedPids(ws.pidsLog)).find(p => p.kind === 'caddy')?.pid;
    assert.ok(caddyPid && processAlive(caddyPid));

    // Duplicate profile owner must exit 5 with zero bytes on stdout.
    const dup = spawn(
      scpctlBin,
      ['--config-dir', ws.cliRoot, '--profile', 'stdio-s4', 'mcp', 'serve', '--stdio'],
      { stdio: ['pipe', 'pipe', 'pipe'], env: ws.env },
    );
    const dupOut = [];
    dup.stdout.on('data', c => dupOut.push(c));
    const [dupCode] = await once(dup, 'exit');
    assert.equal(dupCode, 5);
    assert.equal(Buffer.concat(dupOut).length, 0);

    // Oversized line (> 4096) does not corrupt stdout framing; subsequent request still works.
    host.send(
      JSON.stringify({
        jsonrpc: '2.0',
        id: 80,
        method: 'tools/call',
        params: { name: 'server_get', arguments: { serverId: 'Z'.repeat(6000) } },
      }),
    );
    host.send({
      jsonrpc: '2.0',
      id: 81,
      method: 'tools/call',
      params: { name: 'servers_list', arguments: {} },
    });
    const recovered = await host.waitForId(81);
    assert.equal(recovered.result.isError, false);

    host.closeStdin();
    const exited = await host.exitPromise;
    assert.equal(exited.code, 0);
    assert.deepEqual(host.nonJsonStdoutLines, []);
    assert.equal(await waitUntil(2500, () => !processAlive(caddyPid)), true);
  }

  // 2. Write-timeout stall aborts host non-zero and reaps supervised child PID.
  {
    const extraServers = Array.from({ length: 16 }, (_, i) => ({
      id: `srv-pad-${i}`,
      name: `Pad-${i}-${'X'.repeat(160)}`,
      host: '10.0.0.10',
      os: 'linux',
      connectionMethod: 'ssh',
      sshPort: 22,
      sshUser: 'ops',
      services: Array.from({ length: 25 }, (_, j) => ({
        name: `svc-${j}-${'Y'.repeat(80)}`,
        port: 8000 + j,
      })),
    }));
    const ws = await createIsolatedWorkspace(t, fx, {
      stdioPolicy: {
        sensitiveActionPolicy: 'allow-local-policy',
        allowedToolCategories: ['read-only', 'runtime-control', 'config-mutation', 'file-transfer'],
      },
      extraServers,
    });
    const host = attachRawStdioHost(t, ws, { SCP_STDIO_WRITE_TIMEOUT_MS: '300' });
    host.send({
      jsonrpc: '2.0',
      id: 1,
      method: 'initialize',
      params: {
        protocolVersion: '2025-11-25',
        capabilities: {},
        clientInfo: { name: 'raw-s4-stall', version: '1' },
      },
    });
    await host.waitForId(1);
    host.send({ jsonrpc: '2.0', method: 'notifications/initialized' });

    host.send({
      jsonrpc: '2.0',
      id: 20,
      method: 'tools/call',
      params: { name: 'caddy_start', arguments: { folder: ws.caddyDir } },
    });
    await host.waitForId(20);
    const caddyPid = (await readTrackedPids(ws.pidsLog)).find(p => p.kind === 'caddy')?.pid;
    assert.ok(caddyPid && processAlive(caddyPid));

    host.pauseStdout();
    const stallStart = Date.now();
    for (let i = 0; i < 10; i++) {
      host.send({
        jsonrpc: '2.0',
        id: 100 + i,
        method: 'tools/call',
        params: { name: 'servers_list', arguments: {} },
      });
    }
    const exited = await host.exitPromise;
    assert.notEqual(exited.code, 0, 'stalled stdout writer must exit non-zero');
    assert.ok(Date.now() - stallStart <= 3500, 'writer stall took too long to abort');
    assert.equal(await waitUntil(2500, () => !processAlive(caddyPid)), true);
  }

  // 3. SIGTERM (exit 0), SIGINT (exit 130), and detached no-TTY child reaping.
  for (const [sig, expectedCode] of [
    ['SIGTERM', 0],
    ['SIGINT', 130],
  ]) {
    const ws = await createIsolatedWorkspace(t, fx, {
      stdioPolicy: {
        sensitiveActionPolicy: 'allow-local-policy',
        allowedToolCategories: ['read-only', 'runtime-control', 'config-mutation', 'file-transfer'],
      },
    });
    const raw = attachRawStdioHost(t, ws, {}, { detached: true });
    raw.send({
      jsonrpc: '2.0',
      id: 1,
      method: 'initialize',
      params: {
        protocolVersion: '2025-11-25',
        capabilities: {},
        clientInfo: { name: `no-tty-${sig}`, version: '1' },
      },
    });
    await raw.waitForId(1);
    raw.send({ jsonrpc: '2.0', method: 'notifications/initialized' });
    raw.send({
      jsonrpc: '2.0',
      id: 2,
      method: 'tools/call',
      params: {
        name: 'caddy_start',
        arguments: { folder: ws.caddyDir },
      },
    });
    await raw.waitForId(2);
    const caddyPid = (await readTrackedPids(ws.pidsLog)).find(p => p.kind === 'caddy')?.pid;
    assert.ok(caddyPid && processAlive(caddyPid));

    raw.child.kill(sig);
    const { code } = await raw.exitPromise;
    assert.equal(code, expectedCode, `expected exit ${expectedCode} on ${sig}`);
    assert.equal(
      await waitUntil(2500, () => !processAlive(caddyPid)),
      true,
      `caddy PID ${caddyPid} leaked after ${sig}`,
    );
  }
});
