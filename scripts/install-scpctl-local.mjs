#!/usr/bin/env node
/**
 * Builds, packages, verifies, and installs `scpctl` on this machine.
 *
 * Orchestrates the existing release tooling: `package-scpctl.mjs` (release build, isolation
 * checks, archive + SHA256SUMS), `package-scpctl.mjs --verify`, and the checksum-verified
 * `install-scpctl.sh` prefix installer. Never touches any scpctl or desktop profile directory.
 */
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';

const REPO_ROOT = path.resolve(new URL('..', import.meta.url).pathname);
const PACKAGE_SCRIPT = path.join(REPO_ROOT, 'scripts/package-scpctl.mjs');
const INSTALL_SCRIPT = path.join(REPO_ROOT, 'scripts/install-scpctl.sh');

const USAGE = `Usage: npm run cli:install -- [--prefix <DIR>] [--out-dir <DIR>] [--bin-path <PATH>] [--skip-cargo-tree]

  --prefix <DIR>      Install root (default: ~/.local, binary goes to <DIR>/bin/scpctl)
  --out-dir <DIR>     Where the archive and SHA256SUMS are written (default: dist-cli)
  --bin-path <PATH>   Package an existing scpctl binary instead of building a release one
  --skip-cargo-tree   Skip the dependency isolation check (faster, not for releases)`;

/** Parses CLI flags; throws on unknown options or missing values. */
function parseArgs(argv) {
  const opts = {
    prefix: path.join(homedir(), '.local'),
    outDir: path.join(REPO_ROOT, 'dist-cli'),
    binPath: null,
    skipCargoTree: false,
    help: false,
  };
  const value = (i, flag) => {
    if (i >= argv.length || !argv[i]) throw new Error(`Missing value for ${flag}`);
    return argv[i];
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--prefix') opts.prefix = path.resolve(value(++i, arg));
    else if (arg === '--out-dir') opts.outDir = path.resolve(value(++i, arg));
    else if (arg === '--bin-path') opts.binPath = path.resolve(value(++i, arg));
    else if (arg === '--skip-cargo-tree') opts.skipCargoTree = true;
    else if (arg === '-h' || arg === '--help') opts.help = true;
    else throw new Error(`Unknown option: ${arg}`);
  }
  return opts;
}

/** Runs a child process, streaming stderr and returning stdout; throws on non-zero exit. */
function step(label, cmd, args) {
  process.stdout.write(`==> ${label}\n`);
  const res = spawnSync(cmd, args, {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'],
    maxBuffer: 16 * 1024 * 1024,
  });
  if (res.error) throw new Error(`${label} failed: ${res.error.message}`);
  if (res.status !== 0) throw new Error(`${label} failed (exit ${res.status})`);
  return res.stdout;
}

/** Returns true when `dir` is one of the PATH entries. */
function isOnPath(dir) {
  return (process.env.PATH || '')
    .split(path.delimiter)
    .some(entry => entry && path.resolve(entry) === path.resolve(dir));
}

try {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    process.stdout.write(`${USAGE}\n`);
    process.exit(0);
  }
  if (process.platform === 'win32') {
    throw new Error(
      'cli:install uses the POSIX prefix installer and is not supported on Windows; use `npm run cli:package` and unpack the .zip manually.',
    );
  }

  const packageArgs = [PACKAGE_SCRIPT, '--out-dir', opts.outDir];
  if (opts.binPath) packageArgs.push('--bin-path', opts.binPath);
  if (opts.skipCargoTree) packageArgs.push('--skip-cargo-tree');
  step(opts.binPath ? 'Packaging scpctl' : 'Building and packaging scpctl (release)', process.execPath, packageArgs);
  step('Verifying package checksums', process.execPath, [PACKAGE_SCRIPT, '--verify', opts.outDir]);

  const manifest = JSON.parse(readFileSync(path.join(opts.outDir, 'build-manifest.json'), 'utf8'));
  const archive = path.join(opts.outDir, manifest.artifactName);
  if (!existsSync(archive)) throw new Error(`Packaged archive missing: ${archive}`);
  if (!archive.endsWith('.tar.gz')) throw new Error(`Unsupported archive type: ${manifest.artifactName}`);

  const installOut = step(`Installing into ${opts.prefix}`, 'bash', [
    INSTALL_SCRIPT,
    '--archive',
    archive,
    '--checksums',
    path.join(opts.outDir, 'SHA256SUMS'),
    '--prefix',
    opts.prefix,
  ]);
  process.stdout.write(installOut);

  const binDir = path.join(opts.prefix, 'bin');
  if (!isOnPath(binDir)) {
    process.stdout.write(
      `Note: ${binDir} is not on your PATH. Add it, e.g.: export PATH="${binDir}:$PATH"\n`,
    );
  }
} catch (err) {
  process.stderr.write(`${err.message}\n`);
  process.exit(1);
}
