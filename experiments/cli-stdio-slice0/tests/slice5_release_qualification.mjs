import { test } from 'node:test';
import assert from 'node:assert/strict';
import { access, copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { Client as ModernClient } from '@modelcontextprotocol/client';
import { StdioClientTransport as ModernStdioTransport } from '@modelcontextprotocol/client/stdio';
import {
  buildHeadlessEnv,
  runCommand,
  startSshdDaemon,
} from '../../../examples/standalone-cli/lib/agent-helpers.mjs';

const repoRoot = path.resolve(new URL('../../..', import.meta.url).pathname);
const scpctlBin = process.env.SCPCTL_BIN
  ? path.resolve(process.env.SCPCTL_BIN)
  : path.join(repoRoot, 'target/release/scpctl');
const sshdFixtureBin = process.env.SSHD_FIXTURE_BIN
  ? path.resolve(process.env.SSHD_FIXTURE_BIN)
  : path.join(repoRoot, 'target/release/examples/sshd_fixture_daemon');
const inspectorLauncher = path.join(
  repoRoot,
  'experiments/cli-stdio-slice0/node_modules/@modelcontextprotocol/inspector/clients/launcher/build/index.js',
);

async function startSshdFixture(t) {
  const started = await startSshdDaemon(sshdFixtureBin);
  t.after(() => started.stop());
  return started.info;
}

test('package-scpctl.mjs produces deterministic archive, SHA256SUMS, purity manifest, and rejects tampering', { timeout: 30000 }, async t => {
  const work = await mkdtemp(path.join(tmpdir(), 'scp-slice5-pkg-'));
  t.after(() => rm(work, { recursive: true, force: true }));

  const outDir = path.join(work, 'dist');
  const pkgRes = await runCommand(process.execPath, [
    path.join(repoRoot, 'scripts/package-scpctl.mjs'),
    '--bin-path',
    scpctlBin,
    '--out-dir',
    outDir,
  ]);
  assert.equal(pkgRes.code, 0, `package failed: ${pkgRes.stderr}`);

  const manifestPath = path.join(outDir, 'build-manifest.json');
  const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  assert.equal(manifest.dependencyIsolationPassed, true);
  assert.equal(manifest.dynamicLinkageIsolationPassed, true);
  assert.equal(manifest.schemaCommandCount, 31);
  assert.equal(manifest.mcpOperationCount, 45);
  assert.equal(manifest.mcpResourceCount, 6);

  const archivePath = path.join(outDir, manifest.artifactName);
  const sumsPath = path.join(outDir, 'SHA256SUMS');
  await access(archivePath);
  await access(sumsPath);
  await access(path.join(outDir, 'scpctl-cargo-tree.txt'));

  const verifyOk = await runCommand(process.execPath, [
    path.join(repoRoot, 'scripts/package-scpctl.mjs'),
    '--verify',
    outDir,
  ]);
  assert.equal(verifyOk.code, 0, `verify failed: ${verifyOk.stderr}`);

  const shaCheck = await runCommand('sha256sum', ['-c', 'SHA256SUMS'], { cwd: outDir });
  assert.equal(shaCheck.code, 0, `sha256sum failed: ${shaCheck.stderr}`);

  // Tamper test: verify + installer must both reject corrupted archives.
  const tamperedDir = path.join(work, 'tampered');
  await mkdir(tamperedDir, { recursive: true });
  const tamperedArchive = path.join(tamperedDir, manifest.artifactName);
  const tamperedSums = path.join(tamperedDir, 'SHA256SUMS');
  await copyFile(archivePath, tamperedArchive);
  await copyFile(sumsPath, tamperedSums);
  await copyFile(manifestPath, path.join(tamperedDir, 'build-manifest.json'));
  const raw = await readFile(tamperedArchive);
  raw[raw.length - 1] ^= 0xff;
  await writeFile(tamperedArchive, raw);

  const verifyBad = await runCommand(process.execPath, [
    path.join(repoRoot, 'scripts/package-scpctl.mjs'),
    '--verify',
    tamperedDir,
  ]);
  assert.notEqual(verifyBad.code, 0, 'tampered package verification must fail');

  const badPrefix = path.join(work, 'bad-prefix');
  const installBad = await runCommand('bash', [
    path.join(repoRoot, 'scripts/install-scpctl.sh'),
    '--archive',
    tamperedArchive,
    '--checksums',
    tamperedSums,
    '--prefix',
    badPrefix,
  ]);
  assert.notEqual(installBad.code, 0, 'installer must fail on checksum mismatch');
  await assert.rejects(() => access(path.join(badPrefix, 'bin/scpctl')));
});

test('install-scpctl.sh installs isolated prefix and passes GUI-free dbus-run-session SSH-key + stdio MCP smoke', { timeout: 45000 }, async t => {
  const work = await mkdtemp(path.join(tmpdir(), 'scp-slice5-install-'));
  t.after(() => rm(work, { recursive: true, force: true }));

  const outDir = path.join(work, 'dist');
  const prefix = path.join(work, 'prefix');
  const pkgRes = await runCommand(process.execPath, [
    path.join(repoRoot, 'scripts/package-scpctl.mjs'),
    '--bin-path',
    scpctlBin,
    '--out-dir',
    outDir,
    '--skip-cargo-tree',
  ]);
  assert.equal(pkgRes.code, 0, pkgRes.stderr);
  const manifest = JSON.parse(await readFile(path.join(outDir, 'build-manifest.json'), 'utf8'));

  const installRes = await runCommand('bash', [
    path.join(repoRoot, 'scripts/install-scpctl.sh'),
    '--archive',
    path.join(outDir, manifest.artifactName),
    '--checksums',
    path.join(outDir, 'SHA256SUMS'),
    '--prefix',
    prefix,
  ]);
  assert.equal(installRes.code, 0, `install failed: ${installRes.stderr}`);

  const installedBin = path.join(prefix, 'bin/scpctl');
  for (const rel of [
    'bin/scpctl',
    'share/bash-completion/completions/scpctl',
    'share/zsh/site-functions/_scpctl',
    'share/fish/vendor_completions.d/scpctl.fish',
    'share/scpctl/completions/scpctl.ps1',
  ]) {
    const content = await readFile(path.join(prefix, rel));
    assert.ok(content.length > 0, `expected non-empty installed file ${rel}`);
  }

  const fx = await startSshdFixture(t);
  const cliRoot = path.join(work, 'cli-root');
  const desktopDir = path.join(work, 'desktop');
  const profileDir = path.join(cliRoot, 'profiles', 'gui-free');
  const knownHosts = path.join(work, 'known_hosts');
  const localSrc = path.join(work, 'upload.txt');
  const localDst = path.join(work, 'download.txt');
  await mkdir(profileDir, { recursive: true });
  await mkdir(desktopDir, { recursive: true });
  await writeFile(knownHosts, `[127.0.0.1]:${fx.port} ${fx.hostPubAlgoAndKey}\n`);
  await writeFile(localSrc, 'slice5-gui-free-payload\n');
  await writeFile(
    path.join(profileDir, 'stdio-policy.json'),
    JSON.stringify({
      sensitiveActionPolicy: 'allow-local-policy',
      allowedToolCategories: ['read-only', 'runtime-control', 'config-mutation', 'file-transfer'],
    }),
  );

  const env = buildHeadlessEnv({
    SCPCTL_DESKTOP_CONFIG_DIR: desktopDir,
    SCPCTL_SSH_KNOWN_HOSTS: knownHosts,
    SCPCTL_SSH_IDENTITY_FILE: fx.userKey,
  });

  // 1. Inside dbus-run-session (real D-Bus, zero org.freedesktop.secrets): keyring is unavailable.
  const caps = await runCommand(
    'dbus-run-session',
    [
      '--',
      installedBin,
      '--config-dir',
      cliRoot,
      '--profile',
      'gui-free',
      'capabilities',
      '--json',
      '--non-interactive',
    ],
    { env },
  );
  assert.equal(caps.code, 0, caps.stderr);
  assert.equal(JSON.parse(caps.stdout).data.keyring.state, 'unavailable');

  const secretFail = await runCommand(
    'dbus-run-session',
    [
      '--',
      installedBin,
      '--config-dir',
      cliRoot,
      '--profile',
      'gui-free',
      'config',
      'secret',
      'set',
      '--key',
      'openvpn-password',
      '--secret-stdin',
      '--json',
      '--non-interactive',
    ],
    { env, input: 'nope\n' },
  );
  assert.equal(secretFail.code, 4);
  assert.equal(JSON.parse(secretFail.stdout).error.reason, 'keyring_unavailable');

  // 2. SSH-key server upsert + one-shot SCP upload/download succeed without Secret Service.
  const serverFile = path.join(work, 'server.json');
  await writeFile(
    serverFile,
    JSON.stringify({
      id: 'srv-gui-free',
      name: 'Headless SSH Fixture',
      host: '127.0.0.1',
      os: 'linux',
      connectionMethod: 'ssh',
      sshPort: fx.port,
      sshUser: fx.user,
    }),
  );
  const up = await runCommand(
    installedBin,
    [
      '--config-dir',
      cliRoot,
      '--profile',
      'gui-free',
      'server',
      'upsert',
      '--file',
      serverFile,
      '--json',
      '--non-interactive',
    ],
    { env },
  );
  assert.equal(up.code, 0, up.stderr);

  const remoteFile = path.posix.join(fx.remoteSandbox, 'gui-free-upload.txt');
  const scpUp = await runCommand(
    installedBin,
    [
      '--config-dir',
      cliRoot,
      '--profile',
      'gui-free',
      'scp',
      'upload',
      '--server-id',
      'srv-gui-free',
      '--local',
      localSrc,
      '--remote',
      remoteFile,
      '--json',
      '--non-interactive',
    ],
    { env },
  );
  assert.equal(scpUp.code, 0, scpUp.stderr);

  const scpDown = await runCommand(
    installedBin,
    [
      '--config-dir',
      cliRoot,
      '--profile',
      'gui-free',
      'scp',
      'download',
      '--server-id',
      'srv-gui-free',
      '--remote',
      remoteFile,
      '--local',
      localDst,
      '--overwrite',
      '--json',
      '--non-interactive',
    ],
    { env },
  );
  assert.equal(scpDown.code, 0, scpDown.stderr);
  assert.equal(await readFile(localDst, 'utf8'), 'slice5-gui-free-payload\n');

  // 3. Installed scpctl stdio MCP inside dbus-run-session with @modelcontextprotocol/client + Inspector CLI.
  const client = new ModernClient(
    { name: 'slice5-installed-smoke', version: '1' },
    { capabilities: {}, versionNegotiation: { mode: { pin: '2026-07-28' } } },
  );
  t.after(async () => {
    try {
      await client.close();
    } catch {}
  });
  await client.connect(
    new ModernStdioTransport({
      command: 'dbus-run-session',
      args: ['--', installedBin, '--config-dir', cliRoot, '--profile', 'gui-free', 'mcp', 'serve', '--stdio'],
      env,
      stderr: 'pipe',
    }),
  );
  const opened = await client.callTool({
    name: 'remote_browser_open',
    arguments: { serverId: 'srv-gui-free' },
  });
  assert.equal(Boolean(opened.isError), false, JSON.stringify(opened));
  const browserId = opened.structuredContent.result.browserId;

  const listed = await client.callTool({
    name: 'remote_browser_list_children',
    arguments: { browserId, path: fx.remoteSandbox },
  });
  assert.equal(listed.structuredContent.result.some(e => e.name === 'gui-free-upload.txt'), true);

  const asyncSrc = path.join(work, 'async-mcp.txt');
  const asyncRemote = path.posix.join(fx.remoteSandbox, 'async-mcp.txt');
  await writeFile(asyncSrc, 'slice5-async-mcp\n');
  const started = await client.callTool({
    name: 'scp_start_upload',
    arguments: { serverId: 'srv-gui-free', localPath: asyncSrc, remotePath: asyncRemote },
  });
  assert.equal(Boolean(started.isError), false, JSON.stringify(started));
  const sessionId = started.structuredContent.result.sessionId;
  assert.ok(sessionId);

  let done = null;
  for (let i = 0; i < 60; i++) {
    const listedTx = await client.callTool({ name: 'scp_transfer_list', arguments: {} });
    const match = listedTx.structuredContent.result.find(tx => tx.sessionId === sessionId);
    if (match && (match.state === 'success' || match.state === 'error')) {
      const txGet = await client.callTool({ name: 'scp_transfer_get', arguments: { transferId: match.id } });
      done = txGet.structuredContent.result;
      break;
    }
    await new Promise(r => setTimeout(r, 50));
  }
  assert.equal(done?.state, 'success');

  await client.callTool({ name: 'remote_browser_close', arguments: { browserId } });
  await client.close();

  // Inspector CLI 2.14.0 smoke against installed binary (modern & legacy).
  for (const era of ['modern', 'legacy']) {
    const insp = await runCommand(
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
        '--method',
        'tools/call',
        '--tool-name',
        'server_get',
        '--tool-args-json',
        JSON.stringify({ serverId: 'srv-gui-free' }),
        '--',
        installedBin,
        '--config-dir',
        cliRoot,
        '--profile',
        'gui-free',
        'mcp',
        'serve',
        '--stdio',
      ],
      { env },
    );
    assert.equal(insp.code, 0, `inspector ${era} failed: ${insp.stderr}`);
    assert.match(insp.stdout, /srv-gui-free/);
  }
});

test('shipped shell and stdio MCP agent workflow examples execute cleanly against target scpctl', { timeout: 45000 }, async t => {
  const work = await mkdtemp(path.join(tmpdir(), 'scp-slice5-examples-'));
  t.after(() => rm(work, { recursive: true, force: true }));

  const env = buildHeadlessEnv();

  const shellRun = await runCommand(
    process.execPath,
    [
      path.join(repoRoot, 'examples/standalone-cli/shell-agent-workflow.mjs'),
      '--scpctl',
      scpctlBin,
      '--sshd-fixture',
      sshdFixtureBin,
    ],
    { env },
  );
  assert.equal(shellRun.code, 0, `shell example failed: ${shellRun.stderr}\n${shellRun.stdout}`);
  const shellJson = JSON.parse(shellRun.stdout.trim());
  assert.equal(shellJson.ok, true);
  assert.equal(shellJson.workflow, 'shell');
  for (const key of [
    'discover',
    'capabilities',
    'capabilityFailure',
    'previewAndMutate',
    'lostResponseRecovery',
    'transferAndInspect',
    'cleanup',
  ]) {
    assert.equal(shellJson.steps[key], 'passed', `shell step ${key} not passed`);
  }

  const mcpRun = await runCommand(
    process.execPath,
    [
      path.join(repoRoot, 'examples/standalone-cli/stdio-mcp-agent-workflow.mjs'),
      '--scpctl',
      scpctlBin,
      '--sshd-fixture',
      sshdFixtureBin,
    ],
    { env },
  );
  assert.equal(mcpRun.code, 0, `mcp example failed: ${mcpRun.stderr}\n${mcpRun.stdout}`);
  const mcpJson = JSON.parse(mcpRun.stdout.trim());
  assert.equal(mcpJson.ok, true);
  assert.equal(mcpJson.workflow, 'stdio-mcp');
  for (const key of [
    'connectAndDiscover',
    'ownerConflict',
    'upsertAndBrowse',
    'asyncCompletion',
    'lostResponseRecovery',
    'cleanup',
  ]) {
    assert.equal(mcpJson.steps[key], 'passed', `mcp step ${key} not passed`);
  }
});
