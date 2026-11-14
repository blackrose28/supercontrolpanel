#!/usr/bin/env node
/**
 * Builds, verifies dependency/linkage purity, packages, and checksums `scpctl`.
 */
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const REPO_ROOT = path.resolve(new URL('..', import.meta.url).pathname);
const FORBIDDEN_CRATES = new Set([
  'tauri',
  'tauri-build',
  'tauri-runtime',
  'wry',
  'tao',
  'webkit2gtk',
  'gtk',
  'glib',
  'libdbus-sys',
  'axum',
  'hyper',
  'reqwest',
]);
const FORBIDDEN_SHARED_LIBS = [
  'libgtk',
  'libwebkit',
  'libgdk',
  'libglib',
  'libgio',
  'libX11',
  'libwayland',
  'libayatana',
  'libappindicator',
  'libdbus-1',
];
const COMPLETIONS = [
  ['bash', 'scpctl.bash'],
  ['zsh', '_scpctl'],
  ['fish', 'scpctl.fish'],
  ['powershell', 'scpctl.ps1'],
];

/** Parses CLI flags for packaging or checksum/manifest verification. */
function parseArgs(argv) {
  const opts = {
    outDir: path.join(REPO_ROOT, 'dist-cli'),
    binPath: null,
    target: null,
    skipCargoTree: false,
    verifyDir: null,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--out-dir') opts.outDir = path.resolve(requireArgValue(argv, ++i, arg));
    else if (arg === '--bin-path') opts.binPath = path.resolve(requireArgValue(argv, ++i, arg));
    else if (arg === '--target') opts.target = requireArgValue(argv, ++i, arg);
    else if (arg === '--verify') opts.verifyDir = path.resolve(requireArgValue(argv, ++i, arg));
    else if (arg === '--skip-cargo-tree') opts.skipCargoTree = true;
    else throw new Error(`Unknown option: ${arg}`);
  }
  return opts;
}

/** Reads the required argument value following a CLI flag. */
function requireArgValue(argv, idx, flag) {
  if (idx >= argv.length || !argv[idx]) {
    throw new Error(`Missing value for ${flag}`);
  }
  return argv[idx];
}

/** Normalizes host OS/arch into release artifact target strings. */
function resolveTarget(overrideTarget) {
  if (overrideTarget) {
    const parts = overrideTarget.split('-');
    if (parts.length !== 2 || !parts[0] || !parts[1]) {
      throw new Error(`Invalid --target format '${overrideTarget}', expected <os>-<arch>`);
    }
    return { os: parts[0], arch: parts[1] };
  }
  const osMap = { linux: 'linux', darwin: 'macos', win32: 'windows' };
  const archMap = { x64: 'x86_64', arm64: 'aarch64' };
  return {
    os: osMap[process.platform] || process.platform,
    arch: archMap[process.arch] || process.arch,
  };
}

/** Runs a child command synchronously and returns trimmed UTF-8 stdout. */
function runTool(cmd, args, opts = {}) {
  return execFileSync(cmd, args, {
    cwd: opts.cwd || REPO_ROOT,
    env: opts.env || process.env,
    encoding: 'utf8',
    maxBuffer: 16 * 1024 * 1024,
  });
}

/** Computes lowercase hex SHA-256 of a file on disk. */
function sha256File(filePath) {
  return createHash('sha256').update(readFileSync(filePath)).digest('hex');
}

/** Resolves or builds the release `scpctl` executable. */
function resolveBinary(binPath) {
  if (binPath) {
    if (!existsSync(binPath)) throw new Error(`Binary not found: ${binPath}`);
    return binPath;
  }
  runTool('cargo', ['build', '--locked', '--release', '-p', 'scp-cli']);
  const exe = process.platform === 'win32' ? 'scpctl.exe' : 'scpctl';
  const built = path.join(REPO_ROOT, 'target/release', exe);
  if (!existsSync(built)) throw new Error(`Built release binary missing: ${built}`);
  return built;
}

/** Inspects `scpctl --version` and `schema --json` without touching user profiles. */
function probeBinaryMetadata(binPath) {
  const env = { ...process.env, SCPCTL_CONFIG_DIR: '/nonexistent/scpctl-package-probe' };
  const versionRaw = runTool(binPath, ['--version'], { env }).trim();
  const match = versionRaw.match(/scpctl\s+(\S+)/);
  if (!match) throw new Error(`Unexpected --version output: ${versionRaw}`);
  const schemaEnvelope = JSON.parse(runTool(binPath, ['schema', '--json'], { env }));
  const data = schemaEnvelope.data || {};
  return {
    version: match[1],
    schemaCommandCount: Array.isArray(data.commands) ? data.commands.length : 0,
    mcpOperationCount: Array.isArray(data.mcpOperations) ? data.mcpOperations.length : 0,
    mcpResourceCount: Array.isArray(data.mcpResources) ? data.mcpResources.length : 0,
  };
}

/** Verifies `cargo tree` contains zero GUI/Tauri/D-Bus/HTTP crates and writes the log. */
function checkCargoTree(outDir, skip) {
  const treePath = path.join(outDir, 'scpctl-cargo-tree.txt');
  if (skip) {
    writeFileSync(treePath, 'Skipped via --skip-cargo-tree\n', 'utf8');
    return true;
  }
  const tree = runTool('cargo', ['tree', '--locked', '-p', 'scp-cli', '-e', 'normal,build']);
  writeFileSync(treePath, tree, 'utf8');
  for (const line of tree.split(/\r?\n/)) {
    const match = line.match(/([A-Za-z0-9_-]+)\s+v\d/);
    if (match && FORBIDDEN_CRATES.has(match[1])) {
      throw new Error(`Forbidden dependency crate in scp-cli graph: ${match[1]}`);
    }
  }
  return true;
}

/** Verifies Linux dynamic linkage excludes GUI/desktop/D-Bus libraries. */
function checkDynamicLinkage(binPath, osName) {
  if (osName !== 'linux' || process.platform !== 'linux') {
    return { passed: true, libraries: [] };
  }
  const output = runTool('ldd', [binPath]);
  const libs = output
    .split(/\r?\n/)
    .map(line => line.trim())
    .filter(Boolean)
    .map(line => line.split(/\s+/)[0]);
  for (const lib of libs) {
    if (FORBIDDEN_SHARED_LIBS.some(bad => lib.includes(bad))) {
      throw new Error(`Forbidden dynamic library linked into scpctl: ${lib}`);
    }
  }
  return { passed: true, libraries: libs };
}

/** Generates package README instructions included inside the release archive. */
function buildPackageReadme(meta, target) {
  const archiveExt = target.os === 'windows' ? 'zip' : 'tar.gz';
  return [
    `scpctl ${meta.version} (${target.os}-${target.arch})`,
    '',
    'Standalone headless CLI and stdio MCP server for Super Control Panel.',
    'Runs with the desktop application closed and HTTP MCP disabled.',
    '',
    'Contents:',
    '  bin/scpctl        Standalone CLI & `scpctl mcp serve --stdio` host',
    '  completions/      Shell completions for bash, zsh, fish, and PowerShell',
    '',
    'Quick verify & run:',
    `  sha256sum -c SHA256SUMS   # against scpctl-v${meta.version}-${target.os}-${target.arch}.${archiveExt}`,
    '  ./bin/scpctl --version',
    '  ./bin/scpctl schema --json',
    '  ./bin/scpctl capabilities --no-profile --json',
    '  ./bin/scpctl mcp serve --stdio',
    '',
    'Security & profile notes:',
    '  - Uses an isolated CLI profile directory (never shares the desktop config directory).',
    '  - Stdio MCP defaults to read-only (`stdio-policy.json`); diagnostics go to stderr only.',
    '',
  ].join('\n');
}

/** Creates the release archive (`.tar.gz` on POSIX, `.zip` on Windows). */
function createArchive({ stageRoot, packageDirName, outDir, target }) {
  const isWin = target.os === 'windows';
  const archiveName = `${packageDirName}.${isWin ? 'zip' : 'tar.gz'}`;
  const archivePath = path.join(outDir, archiveName);
  if (isWin) {
    if (process.platform === 'win32') {
      runTool(
        'powershell.exe',
        [
          '-NoProfile',
          '-Command',
          `Compress-Archive -Path '${path.join(stageRoot, packageDirName)}' -DestinationPath '${archivePath}' -Force`,
        ],
        { cwd: stageRoot },
      );
    } else {
      runTool('zip', ['-qr', archivePath, packageDirName], { cwd: stageRoot });
    }
  } else {
    runTool('tar', ['-czf', archivePath, '-C', stageRoot, packageDirName]);
  }
  return { archiveName, archivePath };
}

/** Safely queries a short tool/git string, returning `"unknown"` on failure. */
function probeToolString(cmd, args) {
  try {
    return runTool(cmd, args).trim() || 'unknown';
  } catch {
    return 'unknown';
  }
}

/** Packages the standalone binary, completions, manifest, and SHA256SUMS. */
function packageRelease(opts) {
  mkdirSync(opts.outDir, { recursive: true });
  const binPath = resolveBinary(opts.binPath);
  const target = resolveTarget(opts.target);
  const meta = probeBinaryMetadata(binPath);
  const depOk = checkCargoTree(opts.outDir, opts.skipCargoTree);
  const linkage = checkDynamicLinkage(binPath, target.os);

  const packageDirName = `scpctl-v${meta.version}-${target.os}-${target.arch}`;
  const stageRoot = mkdtempSync(path.join(tmpdir(), 'scpctl-stage-'));
  try {
    const pkgDir = path.join(stageRoot, packageDirName);
    const binDir = path.join(pkgDir, 'bin');
    const compDir = path.join(pkgDir, 'completions');
    mkdirSync(binDir, { recursive: true });
    mkdirSync(compDir, { recursive: true });

    const binFileName = target.os === 'windows' ? 'scpctl.exe' : 'scpctl';
    const stagedBin = path.join(binDir, binFileName);
    copyFileSync(binPath, stagedBin);
    if (process.platform !== 'win32') chmodSync(stagedBin, 0o755);

    const probeEnv = { ...process.env, SCPCTL_CONFIG_DIR: '/nonexistent/scpctl-package-probe' };
    for (const [shell, fileName] of COMPLETIONS) {
      const script = runTool(binPath, ['completion', shell], { env: probeEnv });
      writeFileSync(path.join(compDir, fileName), script, 'utf8');
    }
    writeFileSync(path.join(pkgDir, 'README.txt'), buildPackageReadme(meta, target), 'utf8');

    const { archiveName, archivePath } = createArchive({
      stageRoot,
      packageDirName,
      outDir: opts.outDir,
      target,
    });
    const archiveSha256 = sha256File(archivePath);
    const binarySha256 = sha256File(stagedBin);
    writeFileSync(path.join(opts.outDir, 'SHA256SUMS'), `${archiveSha256}  ${archiveName}\n`, 'utf8');

    const manifest = {
      version: meta.version,
      artifactName: archiveName,
      os: target.os,
      arch: target.arch,
      binarySha256,
      archiveSha256,
      gitCommit: probeToolString('git', ['rev-parse', '--short', 'HEAD']),
      rustcVersion: probeToolString('rustc', ['--version']),
      cargoVersion: probeToolString('cargo', ['--version']),
      schemaCommandCount: meta.schemaCommandCount,
      mcpOperationCount: meta.mcpOperationCount,
      mcpResourceCount: meta.mcpResourceCount,
      dependencyIsolationPassed: depOk,
      dynamicLinkageIsolationPassed: linkage.passed,
      linkedLibraries: linkage.libraries,
      completions: COMPLETIONS.map(([, fileName]) => fileName),
    };
    writeFileSync(
      path.join(opts.outDir, 'build-manifest.json'),
      `${JSON.stringify(manifest, null, 2)}\n`,
      'utf8',
    );
    return manifest;
  } finally {
    rmSync(stageRoot, { recursive: true, force: true });
  }
}

/** Verifies SHA256SUMS and build-manifest.json in an existing release directory. */
function verifyReleaseDir(dir) {
  const sumsPath = path.join(dir, 'SHA256SUMS');
  const manifestPath = path.join(dir, 'build-manifest.json');
  if (!existsSync(sumsPath) || !existsSync(manifestPath)) {
    throw new Error(`Missing SHA256SUMS or build-manifest.json in ${dir}`);
  }
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  for (const field of ['version', 'artifactName', 'archiveSha256', 'binarySha256']) {
    if (!manifest[field]) throw new Error(`Invalid manifest field: ${field}`);
  }
  if (!manifest.dependencyIsolationPassed || !manifest.dynamicLinkageIsolationPassed) {
    throw new Error('Release manifest reports failed isolation check');
  }
  const archivePath = path.join(dir, manifest.artifactName);
  if (!existsSync(archivePath)) throw new Error(`Missing archive: ${archivePath}`);

  const lines = readFileSync(sumsPath, 'utf8').trim().split(/\r?\n/);
  const matchedLine = lines.find(l => l.endsWith(`  ${manifest.artifactName}`));
  if (!matchedLine) throw new Error(`Missing SHA256SUMS entry for ${manifest.artifactName}`);
  const expected = matchedLine.split(/\s+/)[0];
  const actual = sha256File(archivePath);
  if (expected !== actual || expected !== manifest.archiveSha256) {
    throw new Error(`Checksum mismatch for ${manifest.artifactName}`);
  }
  return { ok: true, artifactName: manifest.artifactName, sha256: actual };
}

try {
  const opts = parseArgs(process.argv.slice(2));
  const result = opts.verifyDir ? verifyReleaseDir(opts.verifyDir) : packageRelease(opts);
  process.stdout.write(`${JSON.stringify(result)}\n`);
} catch (err) {
  process.stderr.write(`${err.message}\n`);
  process.exit(1);
}
