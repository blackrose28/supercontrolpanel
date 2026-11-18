import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { access, mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

const repoRoot = path.resolve(new URL('../../..', import.meta.url).pathname);
const installScript = path.join(repoRoot, 'scripts/install-scpctl-local.mjs');
const scpctlBin = process.env.SCPCTL_BIN
  ? path.resolve(process.env.SCPCTL_BIN)
  : path.join(repoRoot, 'target/release/scpctl');

function run(cmd, args, env = {}) {
  return new Promise(resolve => {
    const child = spawn(cmd, args, {
      env: { ...process.env, ...env },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', d => (stdout += d));
    child.stderr.on('data', d => (stderr += d));
    child.on('close', code => resolve({ code, stdout, stderr }));
  });
}

const fastArgs = ['--bin-path', scpctlBin, '--skip-cargo-tree'];

test('package.json exposes cli:install wired to the install script', async () => {
  const pkg = JSON.parse(await readFile(path.join(repoRoot, 'package.json'), 'utf8'));
  assert.equal(pkg.scripts['cli:install'], 'node scripts/install-scpctl-local.mjs');
});

test('cli:install packages, verifies, and installs scpctl into --prefix', { timeout: 60000 }, async t => {
  const work = await mkdtemp(path.join(tmpdir(), 'scp-cli-install-'));
  t.after(() => rm(work, { recursive: true, force: true }));
  const prefix = path.join(work, 'prefix');
  const outDir = path.join(work, 'dist');

  const res = await run(process.execPath, [
    installScript,
    ...fastArgs,
    '--prefix',
    prefix,
    '--out-dir',
    outDir,
  ]);
  assert.equal(res.code, 0, `install failed: ${res.stderr}`);

  const installed = path.join(prefix, 'bin/scpctl');
  assert.ok(((await stat(installed)).mode & 0o111) !== 0, 'installed binary must be executable');
  await access(path.join(prefix, 'share/bash-completion/completions/scpctl'));
  await access(path.join(outDir, 'SHA256SUMS'));

  const version = await run(installed, ['--version'], { SCPCTL_CONFIG_DIR: '/nonexistent/scpctl-install-test' });
  assert.equal(version.code, 0, version.stderr);
  assert.match(version.stdout, /^scpctl \S+/);

  assert.match(res.stdout, new RegExp(`Installed scpctl to ${prefix.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}/bin/scpctl`));
});

test('cli:install defaults the prefix to $HOME/.local and warns when bin is not on PATH', { timeout: 60000 }, async t => {
  const work = await mkdtemp(path.join(tmpdir(), 'scp-cli-install-home-'));
  t.after(() => rm(work, { recursive: true, force: true }));
  const home = path.join(work, 'home');

  const res = await run(
    process.execPath,
    [installScript, ...fastArgs, '--out-dir', path.join(work, 'dist')],
    { HOME: home, PATH: '/usr/bin:/bin' },
  );
  assert.equal(res.code, 0, `install failed: ${res.stderr}`);
  await access(path.join(home, '.local/bin/scpctl'));
  assert.match(res.stdout, /not on your PATH/);
});

test('cli:install fails without installing when the binary is missing or options are unknown', { timeout: 60000 }, async t => {
  const work = await mkdtemp(path.join(tmpdir(), 'scp-cli-install-bad-'));
  t.after(() => rm(work, { recursive: true, force: true }));
  const prefix = path.join(work, 'prefix');

  const missing = await run(process.execPath, [
    installScript,
    '--bin-path',
    path.join(work, 'does-not-exist'),
    '--skip-cargo-tree',
    '--prefix',
    prefix,
    '--out-dir',
    path.join(work, 'dist'),
  ]);
  assert.notEqual(missing.code, 0);
  await assert.rejects(() => access(path.join(prefix, 'bin/scpctl')));

  const unknown = await run(process.execPath, [installScript, '--bogus']);
  assert.notEqual(unknown.code, 0);
  assert.match(unknown.stderr, /Unknown option: --bogus/);
});
