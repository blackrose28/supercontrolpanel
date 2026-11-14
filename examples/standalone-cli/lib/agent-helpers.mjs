import { execFile, spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

/**
 * Resolves `--scpctl <path>` and `--sshd-fixture <path>` flags with repo defaults.
 * @param {string[]} argv
 * @param {string} repoRoot
 * @returns {{ scpctlBin: string, sshdFixtureBin: string }}
 */
export function parseExampleArgs(argv, repoRoot) {
  let scpctlBin = process.env.SCPCTL_BIN
    ? path.resolve(process.env.SCPCTL_BIN)
    : path.join(repoRoot, 'target/release/scpctl');
  let sshdFixtureBin = process.env.SSHD_FIXTURE_BIN
    ? path.resolve(process.env.SSHD_FIXTURE_BIN)
    : path.join(repoRoot, 'target/release/examples/sshd_fixture_daemon');

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--scpctl' && argv[i + 1]) {
      scpctlBin = path.resolve(argv[i + 1]);
      i += 1;
    } else if (arg === '--sshd-fixture' && argv[i + 1]) {
      sshdFixtureBin = path.resolve(argv[i + 1]);
      i += 1;
    }
  }
  return { scpctlBin, sshdFixtureBin };
}

/**
 * Builds a deterministic GUI-free environment with no test keyring override.
 * @param {Record<string, string | undefined>} [extra]
 * @returns {Record<string, string | undefined>}
 */
export function buildHeadlessEnv(extra = {}) {
  const env = { ...process.env, ...extra };
  delete env.SCPCTL_TEST_KEYRING_FILE;
  env.DISPLAY = '';
  env.WAYLAND_DISPLAY = '';
  return env;
}

/**
 * Executes a subprocess and returns its exit code, stdout, and stderr without throwing on non-zero exit.
 * @param {string} cmd
 * @param {string[]} args
 * @param {object} [opts]
 * @returns {Promise<{ code: number, stdout: string, stderr: string }>}
 */
/**
 * Executes a subprocess with closed or explicitly delivered stdin and bounded timeout,
 * returning exit code, stdout, and stderr without throwing on non-zero exit.
 * @param {string} cmd
 * @param {string[]} args
 * @param {object} [opts]
 * @returns {Promise<{ code: number, stdout: string, stderr: string }>}
 */
export function runCommand(cmd, args, opts = {}) {
  const { input, timeout = 30_000, ...execOpts } = opts;
  return new Promise(resolve => {
    const child = execFile(
      cmd,
      args,
      { encoding: 'utf8', timeout, killSignal: 'SIGKILL', ...execOpts },
      (err, stdout = '', stderr = '') => {
        if (!err) {
          resolve({ code: 0, stdout, stderr });
          return;
        }
        resolve({
          code: typeof err.code === 'number' ? err.code : 1,
          stdout: stdout || err.stdout || '',
          stderr: stderr || err.stderr || String(err.message || err),
        });
      },
    );
    if (child.stdin) {
      child.stdin.on('error', () => {});
      if (input !== undefined && input !== null) {
        child.stdin.end(input);
      } else {
        child.stdin.end();
      }
    }
  });
}

/**
 * Invokes `scpctl` and parses its single-line JSON stdout envelope alongside the exit code.
 * @param {string} scpctlBin
 * @param {string[]} args
 * @param {object} [opts]
 * @returns {Promise<{ code: number, stdout: string, stderr: string, json: any }>}
 */
export async function runScpctlJson(scpctlBin, args, opts = {}) {
  const res = await runCommand(scpctlBin, args, opts);
  const trimmed = res.stdout.trim();
  let json = null;
  if (trimmed) {
    try {
      json = JSON.parse(trimmed);
    } catch (err) {
      throw new Error(
        `Invalid JSON from ${path.basename(scpctlBin)} ${args.join(' ')}: ${err.message}\nstdout: ${res.stdout}\nstderr: ${res.stderr}`,
      );
    }
  }
  return { ...res, json };
}

async function stopChildDaemon(daemon) {
  if (daemon.stdin && !daemon.stdin.destroyed) {
    daemon.stdin.destroy();
  }
  if (daemon.exitCode !== null) return;
  await Promise.race([
    once(daemon, 'exit'),
    new Promise(resolve => setTimeout(resolve, 400)),
  ]);
  if (daemon.exitCode === null) {
    daemon.kill('SIGKILL');
    await Promise.race([
      once(daemon, 'exit'),
      new Promise(resolve => setTimeout(resolve, 200)),
    ]);
  }
}

function readFirstJsonLine(daemon, label) {
  return new Promise((resolve, reject) => {
    let buf = '';
    const timer = setTimeout(
      () => reject(new Error(`${label} startup timeout`)),
      5000,
    );
    daemon.stdout.on('data', chunk => {
      buf += chunk.toString('utf8');
      const nl = buf.indexOf('\n');
      if (nl === -1) return;
      clearTimeout(timer);
      try {
        resolve(JSON.parse(buf.slice(0, nl).trim()));
      } catch (err) {
        reject(err);
      }
    });
    daemon.once('exit', code => {
      clearTimeout(timer);
      reject(new Error(`${label} exited early with ${code}`));
    });
  });
}

/**
 * Spawns `sshd_fixture_daemon`, reads its ephemeral connection metadata, and returns an idempotent stop hook.
 * @param {string} sshdFixtureBin
 * @returns {Promise<{ info: any, daemon: import('node:child_process').ChildProcess, stop: () => Promise<void> }>}
 */
export async function startSshdDaemon(sshdFixtureBin) {
  const daemon = spawn(sshdFixtureBin, [], { stdio: ['pipe', 'pipe', 'pipe'] });
  try {
    const info = await readFirstJsonLine(daemon, 'sshd_fixture_daemon');
    return { info, daemon, stop: () => stopChildDaemon(daemon) };
  } catch (err) {
    await stopChildDaemon(daemon);
    throw err;
  }
}

/**
 * Provisions an isolated CLI/desktop config workspace, `known_hosts`, optional `stdio-policy.json`,
 * and an ephemeral SSH/SFTP target daemon for headless agent workflows.
 * @param {{ sshdFixtureBin: string, workDir: string, profileName?: string, stdioPolicy?: object | null }} opts
 * @returns {Promise<{ info: any, cliRoot: string, desktopDir: string, profileDir: string, localWork: string, knownHostsPath: string, env: Record<string, string | undefined>, stop: () => Promise<void> }>}
 */
export async function openTargetFixture({
  sshdFixtureBin,
  workDir,
  profileName = 'agent',
  stdioPolicy = null,
}) {
  const cliRoot = path.join(workDir, 'cli-config');
  const desktopDir = path.join(workDir, 'desktop-config');
  const profileDir = path.join(cliRoot, 'profiles', profileName);
  const localWork = path.join(workDir, 'local-work');
  const knownHostsPath = path.join(workDir, 'known_hosts');

  await Promise.all([
    mkdir(profileDir, { recursive: true }),
    mkdir(desktopDir, { recursive: true }),
    mkdir(localWork, { recursive: true }),
  ]);

  const started = await startSshdDaemon(sshdFixtureBin);
  try {
    await writeFile(
      knownHostsPath,
      `[127.0.0.1]:${started.info.port} ${started.info.hostPubAlgoAndKey}\n`,
    );
    if (stdioPolicy) {
      await writeFile(
        path.join(profileDir, 'stdio-policy.json'),
        JSON.stringify(stdioPolicy, null, 2),
      );
    }
    const env = buildHeadlessEnv({
      SCPCTL_DESKTOP_CONFIG_DIR: desktopDir,
      SCPCTL_SSH_KNOWN_HOSTS: knownHostsPath,
      SCPCTL_SSH_IDENTITY_FILE: started.info.userKey,
    });
    return {
      info: started.info,
      cliRoot,
      desktopDir,
      profileDir,
      localWork,
      knownHostsPath,
      env,
      stop: started.stop,
    };
  } catch (err) {
    await started.stop();
    throw err;
  }
}

async function importWithFallback(pkgSpecifier, fallbackRelPath, repoRoot) {
  try {
    return await import(pkgSpecifier);
  } catch {
    return import(pathToFileURL(path.join(repoRoot, fallbackRelPath)).href);
  }
}

/**
 * Loads `@modelcontextprotocol/client` and `@modelcontextprotocol/client/stdio`,
 * falling back to the repo's Slice 0 test dependencies when no root `node_modules` exists.
 * @param {string} repoRoot
 * @returns {Promise<{ Client: any, StdioClientTransport: any }>}
 */
export async function loadModernMcpClient(repoRoot) {
  const clientMod = await importWithFallback(
    '@modelcontextprotocol/client',
    'experiments/cli-stdio-slice0/node_modules/@modelcontextprotocol/client/dist/index.mjs',
    repoRoot,
  );
  const stdioMod = await importWithFallback(
    '@modelcontextprotocol/client/stdio',
    'experiments/cli-stdio-slice0/node_modules/@modelcontextprotocol/client/dist/stdio.mjs',
    repoRoot,
  );
  return {
    Client: clientMod.Client,
    StdioClientTransport: stdioMod.StdioClientTransport,
  };
}
