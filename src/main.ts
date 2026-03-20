import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  open as openDialog,
  save as saveDialog,
} from "@tauri-apps/plugin-dialog";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "./styles.css";

type PtyOutputPayload = {
  id: string;
  data: string;
};

type OpenVpnLogPayload = {
  functionId: string;
  sessionId: string;
  data: string;
};

type PtyExitPayload = {
  id: string;
  exitCode: number | null;
  reason: string;
};

type FunctionStatus = {
  functionId: string;
  state: string;
  message: string;
  sessionId: string | null;
  updatedAtMs: number;
};

type StartFunctionResult = {
  functionId: string;
  sessionId: string;
};

type AppConfig = {
  app: {
    defaultTerminalCols: number;
    defaultTerminalRows: number;
  };
  openvpn: {
    executablePath: string;
    ovpnPath: string;
    extraArgs: string;
    command?: string;
    username: string;
    password: string;
    otpSecret: string;
    terminateOpenVpnWhenHelperExits?: boolean;
  };
  boundary?: {
    cliPath: string;
    addr: string;
    authKind?: "password" | "oidc";
    oidcLoginMode?: "browser" | "automated";
    authMethodId: string;
    loginName: string;
    password: string;
    oidcUsername?: string;
    oidcPassword?: string;
    oidcOtpSecret?: string;
    connectSshOptions?: string;
  };
};

type SyncSettings = {
  repoRoot: string;
  openvpnExecutablePath?: string;
  openvpnOvpnPath?: string;
  boundaryCliPath?: string;
  caddyFolder?: string;
};

type ServerService = { name: string; port: number };
type Server = {
  id: string;
  name: string;
  host: string;
  os: "linux" | "windows";
  connectionMethod: "ssh" | "boundary";
  sshPort: number;
  sshUser: string;
  boundaryTargetId: string;
  boundaryListenPort: number;
  services: ServerService[];
};

type BoundaryTargetSummary = {
  id: string;
  name: string;
  targetType?: string;
  description?: string;
  address?: string;
};

const BOUNDARY_AUTH_FUNCTION_ID = "boundary-auth";

function mustElement<T extends Element>(selector: string): T {
  const node = document.querySelector<T>(selector);
  if (!node) {
    throw new Error(`Missing required element: ${selector}`);
  }
  return node;
}

const terminalHost = mustElement<HTMLElement>("#terminal");
const caddyTerminalHost = mustElement<HTMLElement>("#caddy-terminal");
const navStatus = mustElement<HTMLButtonElement>("#nav-status");
const navOpenvpn = mustElement<HTMLButtonElement>("#nav-openvpn");
const navCaddy = mustElement<HTMLButtonElement>("#nav-caddy");
const navServers = mustElement<HTMLButtonElement>("#nav-servers");
const navBoundary = mustElement<HTMLButtonElement>("#nav-boundary");
const navConfig = mustElement<HTMLButtonElement>("#nav-config");
const viewStatus = mustElement<HTMLElement>("#view-status");
const viewOpenvpn = mustElement<HTMLElement>("#view-openvpn");
const viewCaddy = mustElement<HTMLElement>("#view-caddy");
const viewServers = mustElement<HTMLElement>("#view-servers");
const viewBoundary = mustElement<HTMLElement>("#view-boundary");
const viewConfig = mustElement<HTMLElement>("#view-config");
const statusList = mustElement<HTMLElement>("#status-list");
const openvpnConfigPreview = mustElement<HTMLElement>(
  "#openvpn-config-preview",
);
const vpnStartBtn = mustElement<HTMLButtonElement>("#vpn-start");
const vpnStopBtn = mustElement<HTMLButtonElement>("#vpn-stop");
const caddyStartBtn = mustElement<HTMLButtonElement>("#caddy-start");
const caddyStopBtn = mustElement<HTMLButtonElement>("#caddy-stop");
const cfgTerminalCols = mustElement<HTMLInputElement>("#cfg-terminal-cols");
const cfgTerminalRows = mustElement<HTMLInputElement>("#cfg-terminal-rows");
const cfgOpenvpnExecutable = mustElement<HTMLInputElement>(
  "#cfg-openvpn-executable",
);
const cfgOpenvpnBrowseBtn = mustElement<HTMLButtonElement>(
  "#cfg-openvpn-browse",
);
const cfgOpenvpnDetectBtn = mustElement<HTMLButtonElement>(
  "#cfg-openvpn-detect",
);
const cfgOpenvpnOvpn = mustElement<HTMLInputElement>("#cfg-openvpn-ovpn");
const cfgOpenvpnOvpnBrowseBtn = mustElement<HTMLButtonElement>(
  "#cfg-openvpn-ovpn-browse",
);
const cfgOpenvpnExtraArgs = mustElement<HTMLInputElement>(
  "#cfg-openvpn-extra-args",
);
const cfgOpenvpnUsername = mustElement<HTMLInputElement>(
  "#cfg-openvpn-username",
);
const cfgOpenvpnPassword = mustElement<HTMLInputElement>(
  "#cfg-openvpn-password",
);
const cfgOpenvpnOtpSecret = mustElement<HTMLInputElement>(
  "#cfg-openvpn-otp-secret",
);
const cfgOpenvpnTerminateWithHelper = mustElement<HTMLInputElement>(
  "#cfg-openvpn-terminate-with-helper",
);
const cfgSaveBtn = mustElement<HTMLButtonElement>("#cfg-save");
const cfgReloadBtn = mustElement<HTMLButtonElement>("#cfg-reload");
const cfgStatus = mustElement<HTMLElement>("#cfg-status");
const cfgBoundaryCli = mustElement<HTMLInputElement>("#cfg-boundary-cli");
const cfgBoundaryBrowseBtn = mustElement<HTMLButtonElement>(
  "#cfg-boundary-browse",
);
const cfgBoundaryAddr = mustElement<HTMLInputElement>("#cfg-boundary-addr");
const cfgBoundaryAuthKind = mustElement<HTMLSelectElement>(
  "#cfg-boundary-auth-kind",
);
const cfgBoundaryOidcLoginMode = mustElement<HTMLSelectElement>(
  "#cfg-boundary-oidc-login-mode",
);
const cfgBoundaryAuthMethod = mustElement<HTMLInputElement>(
  "#cfg-boundary-auth-method",
);
const cfgBoundaryConnectSshOptions = mustElement<HTMLInputElement>(
  "#cfg-boundary-connect-ssh-options",
);
const cfgBoundaryLogin = mustElement<HTMLInputElement>("#cfg-boundary-login");
const cfgBoundaryPassword = mustElement<HTMLInputElement>(
  "#cfg-boundary-password",
);
const cfgBoundaryOidcUsername = mustElement<HTMLInputElement>(
  "#cfg-boundary-oidc-username",
);
const cfgBoundaryOidcPassword = mustElement<HTMLInputElement>(
  "#cfg-boundary-oidc-password",
);
const cfgBoundaryOidcOtpSecret = mustElement<HTMLInputElement>(
  "#cfg-boundary-oidc-otp-secret",
);
const cfgBoundaryPasswordOnly = () =>
  Array.from(
    document.querySelectorAll<HTMLElement>(".cfg-boundary-password-only"),
  );
const cfgBoundaryOidcOnly = () =>
  Array.from(document.querySelectorAll<HTMLElement>(".cfg-boundary-oidc-only"));
const cfgBoundaryOidcAutomatedOnly = () =>
  Array.from(
    document.querySelectorAll<HTMLElement>(".cfg-boundary-oidc-automated-only"),
  );
const cfgSyncRepoRoot = mustElement<HTMLInputElement>("#cfg-sync-repo-root");
const cfgSyncAppPath = mustElement<HTMLElement>("#cfg-sync-app-path");
const cfgSyncPullBtn = mustElement<HTMLButtonElement>("#cfg-sync-pull");
const cfgSyncPushBtn = mustElement<HTMLButtonElement>("#cfg-sync-push");
const cfgSyncStatus = mustElement<HTMLElement>("#cfg-sync-status");
const cfgGitPullBtn = mustElement<HTMLButtonElement>("#cfg-git-pull");
const cfgGitPushBtn = mustElement<HTMLButtonElement>("#cfg-git-push");
const cfgGitStatus = mustElement<HTMLElement>("#cfg-git-status");
const cfgCaddyFolder = mustElement<HTMLInputElement>("#cfg-caddy-folder");
const cfgCaddyBrowseBtn = mustElement<HTMLButtonElement>("#cfg-caddy-browse");

const serversTbody = mustElement<HTMLElement>("#servers-tbody");
const serversStatus = mustElement<HTMLElement>("#servers-status");
const serversSearchInput = mustElement<HTMLInputElement>(
  "#servers-search-input",
);
const serversSearchClearBtn = mustElement<HTMLButtonElement>(
  "#servers-search-clear",
);
const boundarySearchInput = mustElement<HTMLInputElement>(
  "#boundary-search-input",
);
const boundarySearchBtn = mustElement<HTMLButtonElement>(
  "#boundary-search-btn",
);
const boundarySearchStatus = mustElement<HTMLElement>(
  "#boundary-search-status",
);
const boundarySearchTbody = mustElement<HTMLElement>("#boundary-search-tbody");
const serverAddBtn = mustElement<HTMLButtonElement>("#server-add");
const serverTunnelStartBtn = mustElement<HTMLButtonElement>(
  "#server-tunnel-start-btn",
);
const serverTunnelStopBtn = mustElement<HTMLButtonElement>(
  "#server-tunnel-stop-btn",
);
const serverSshBtn = mustElement<HTMLButtonElement>("#server-ssh-btn");
const serverRdpBtn = mustElement<HTMLButtonElement>("#server-rdp-btn");
const serverSshStopBtn = mustElement<HTMLButtonElement>("#server-ssh-stop-btn");
const serverUploadBtn = mustElement<HTMLButtonElement>("#server-upload-btn");
const serverDownloadBtn = mustElement<HTMLButtonElement>(
  "#server-download-btn",
);
const scpResultEl = mustElement<HTMLElement>("#scp-result");
const serverEditorOverlay = mustElement<HTMLElement>("#server-editor-overlay");
const seName = mustElement<HTMLInputElement>("#se-name");
const seHost = mustElement<HTMLInputElement>("#se-host");
const seOs = mustElement<HTMLSelectElement>("#se-os");
const seMethod = mustElement<HTMLSelectElement>("#se-method");
const sePort = mustElement<HTMLInputElement>("#se-port");
const seUser = mustElement<HTMLInputElement>("#se-user");
const seBtarget = mustElement<HTMLInputElement>("#se-btarget");
const seListenPort = mustElement<HTMLInputElement>("#se-listen-port");
const seBoundaryOnly = () =>
  Array.from(document.querySelectorAll<HTMLElement>(".se-boundary-only"));
const seServices = mustElement<HTMLTextAreaElement>("#se-services");
const seCancel = mustElement<HTMLButtonElement>("#se-cancel");
const seSave = mustElement<HTMLButtonElement>("#se-save");
const boundaryStartBtn = mustElement<HTMLButtonElement>("#boundary-start");
const boundaryStopBtn = mustElement<HTMLButtonElement>("#boundary-stop");
const serverTunnelVisibleToggle = mustElement<HTMLInputElement>(
  "#server-tunnel-visible",
);

const serverTerminalHost = mustElement<HTMLElement>("#server-terminal");
const serverTunnelTerminalHost = mustElement<HTMLElement>(
  "#server-tunnel-terminal",
);
const boundaryTerminalHost = mustElement<HTMLElement>("#boundary-terminal");

const term = new Terminal({
  cursorBlink: true,
  convertEol: true,
  scrollback: 4000,
});

const fitAddon = new FitAddon();
term.loadAddon(fitAddon);
term.open(terminalHost);
fitAddon.fit();

const caddyFitAddon = new FitAddon();
const caddyTerm = new Terminal({
  cursorBlink: true,
  convertEol: true,
  scrollback: 4000,
});
caddyTerm.loadAddon(caddyFitAddon);
caddyTerm.open(caddyTerminalHost);
caddyFitAddon.fit();

const serverFit = new FitAddon();
const serverTerm = new Terminal({
  cursorBlink: true,
  convertEol: true,
  scrollback: 3000,
});
serverTerm.loadAddon(serverFit);
serverTerm.open(serverTerminalHost);
serverFit.fit();

const serverTunnelFit = new FitAddon();
const serverTunnelTerm = new Terminal({
  cursorBlink: true,
  convertEol: true,
  scrollback: 3000,
});
serverTunnelTerm.loadAddon(serverTunnelFit);
serverTunnelTerm.open(serverTunnelTerminalHost);
serverTunnelFit.fit();

const boundaryFit = new FitAddon();
const boundaryTerm = new Terminal({
  cursorBlink: true,
  convertEol: true,
  scrollback: 3000,
});
boundaryTerm.loadAddon(boundaryFit);
boundaryTerm.open(boundaryTerminalHost);
boundaryFit.fit();

const OPENVPN_FUNCTION_ID = "openvpn-connect";
const CADDY_FUNCTION_ID = "caddy-run";
const SERVER_TUNNEL_PREFIX = "server-tunnel-";
const SERVER_SSH_PREFIX = "server-ssh-";
const statusMap = new Map<string, FunctionStatus>();
let openvpnSessionId: string | null = null;
let caddySessionId: string | null = null;
const serverTunnelSessions: Record<string, string> = {};
const serverSshSessions: Record<string, string> = {};
const serverTunnelBuffers: Record<string, string> = {};
const serverSshBuffers: Record<string, string> = {};
let boundarySessionId: string | null = null;
let serversList: Server[] = [];
let serversSearchTerm = "";
let selectedServerId: string | null = null;
let editingServerId: string | null = null;
let appConfig: AppConfig | null = null;
let syncSettings: SyncSettings | null = null;
let unlistenOutput: UnlistenFn | null = null;
let unlistenExit: UnlistenFn | null = null;
let unlistenFunctionStatus: UnlistenFn | null = null;
let unlistenOpenVpnLog: UnlistenFn | null = null;

async function attachListeners() {
  unlistenOutput = await listen<PtyOutputPayload>("pty-output", (event) => {
    const { id, data } = event.payload;
    if (id === openvpnSessionId) {
      term.write(data);
      return;
    }
    if (id === caddySessionId) {
      caddyTerm.write(data);
      return;
    }

    // Map PTY output back to per-server SSH/tunnel sessions so we can buffer
    for (const [sid, sessionId] of Object.entries(serverSshSessions)) {
      if (sessionId === id) {
        serverSshBuffers[sid] = (serverSshBuffers[sid] ?? "") + data;
        if (sid === selectedServerId) {
          serverTerm.write(data);
        }
        return;
      }
    }
    for (const [sid, sessionId] of Object.entries(serverTunnelSessions)) {
      if (sessionId === id) {
        serverTunnelBuffers[sid] = (serverTunnelBuffers[sid] ?? "") + data;
        if (sid === selectedServerId) {
          serverTunnelTerm.write(data);
        }
        return;
      }
    }

    if (id === boundarySessionId) {
      boundaryTerm.write(data);
    }
  });

  unlistenExit = await listen<PtyExitPayload>("pty-exit", (event) => {
    const { id, exitCode, reason } = event.payload;
    const codePart =
      exitCode === null ? "unknown code" : `exit code ${exitCode}`;
    const line = `\r\n[PTY exited: ${codePart}, reason=${reason}]\r\n`;
    if (id === openvpnSessionId) {
      term.write(line);
      openvpnSessionId = null;
      syncFunctionButtons();
    }
    if (id === caddySessionId) {
      caddyTerm.write(line);
      caddySessionId = null;
      syncCaddyButtons();
    }
    if (selectedServerId && id === serverSshSessions[selectedServerId]) {
      serverTerm.write(line);
      delete serverSshSessions[selectedServerId];
      syncServerActions();
    }
    for (const [sid, sessionId] of Object.entries(serverTunnelSessions)) {
      if (sessionId === id) {
        if (selectedServerId === sid) {
          serverTunnelTerm.write(line);
        }
        delete serverTunnelSessions[sid];
        syncServerActions();
        break;
      }
    }
    for (const [sid, sessionId] of Object.entries(serverSshSessions)) {
      if (sessionId === id) {
        if (selectedServerId === sid) {
          serverTerm.write(line);
        }
        delete serverSshSessions[sid];
        syncServerActions();
        break;
      }
    }
    if (id === boundarySessionId) {
      boundaryTerm.write(line);
      boundarySessionId = null;
      syncBoundaryButtons();
    }
  });

  unlistenFunctionStatus = await listen<FunctionStatus>(
    "function-status",
    (event) => {
      statusMap.set(event.payload.functionId, event.payload);
      renderStatusList();
      if (event.payload.functionId === OPENVPN_FUNCTION_ID) {
        openvpnSessionId = event.payload.sessionId;
        syncFunctionButtons();
      }
      if (event.payload.functionId === CADDY_FUNCTION_ID) {
        caddySessionId = event.payload.sessionId;
        syncCaddyButtons();
      }
      if (event.payload.functionId.startsWith(SERVER_TUNNEL_PREFIX)) {
        const serverId = event.payload.functionId.slice(
          SERVER_TUNNEL_PREFIX.length,
        );
        if (event.payload.sessionId) {
          serverTunnelSessions[serverId] = event.payload.sessionId;
        } else {
          delete serverTunnelSessions[serverId];
        }
        syncServerActions();
      }
      if (event.payload.functionId.startsWith(SERVER_SSH_PREFIX)) {
        const serverId = event.payload.functionId.slice(
          SERVER_SSH_PREFIX.length,
        );
        if (event.payload.sessionId) {
          serverSshSessions[serverId] = event.payload.sessionId;
        } else {
          delete serverSshSessions[serverId];
        }
        syncServerActions();
      }
      if (event.payload.functionId === BOUNDARY_AUTH_FUNCTION_ID) {
        boundarySessionId = event.payload.sessionId;
        syncBoundaryButtons();
      }
    },
  );

  unlistenOpenVpnLog = await listen<OpenVpnLogPayload>(
    "openvpn-log",
    (event) => {
      if (
        event.payload.functionId === OPENVPN_FUNCTION_ID &&
        event.payload.sessionId === openvpnSessionId
      ) {
        term.write(event.payload.data);
      }
    },
  );
}

async function detachListeners() {
  if (unlistenOutput) {
    await unlistenOutput();
    unlistenOutput = null;
  }
  if (unlistenExit) {
    await unlistenExit();
    unlistenExit = null;
  }
  if (unlistenFunctionStatus) {
    await unlistenFunctionStatus();
    unlistenFunctionStatus = null;
  }
  if (unlistenOpenVpnLog) {
    await unlistenOpenVpnLog();
    unlistenOpenVpnLog = null;
  }
}

function setActiveView(
  which: "status" | "openvpn" | "caddy" | "servers" | "boundary" | "config",
) {
  navStatus.classList.toggle("active", which === "status");
  navOpenvpn.classList.toggle("active", which === "openvpn");
  navCaddy.classList.toggle("active", which === "caddy");
  navServers.classList.toggle("active", which === "servers");
  navBoundary.classList.toggle("active", which === "boundary");
  navConfig.classList.toggle("active", which === "config");
  viewStatus.classList.toggle("active", which === "status");
  viewOpenvpn.classList.toggle("active", which === "openvpn");
  viewCaddy.classList.toggle("active", which === "caddy");
  viewServers.classList.toggle("active", which === "servers");
  viewBoundary.classList.toggle("active", which === "boundary");
  viewConfig.classList.toggle("active", which === "config");
  if (which === "caddy") {
    requestAnimationFrame(() => caddyFitAddon.fit());
  }
  if (which === "servers") {
    requestAnimationFrame(() => {
      serverFit.fit();
      serverTunnelFit.fit();
    });
  }
  if (which === "boundary") {
    requestAnimationFrame(() => boundaryFit.fit());
  }
  if (which === "status") {
    void refreshStatuses();
  }
}

const STATUS_CARD_TITLE: Record<string, string> = {
  [OPENVPN_FUNCTION_ID]: "OpenVPN",
  [CADDY_FUNCTION_ID]: "Caddy",
  [BOUNDARY_AUTH_FUNCTION_ID]: "Boundary CLI",
};

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function formatDate(ms: number): string {
  return new Date(ms).toLocaleTimeString();
}

function renderStatusList() {
  const serverIds = new Set(serversList.map((s) => s.id));
  const statuses = [...statusMap.values()]
    .filter((s) => {
      if (
        s.functionId.startsWith(SERVER_TUNNEL_PREFIX) ||
        s.functionId.startsWith(SERVER_SSH_PREFIX)
      ) {
        const serverId = s.functionId.startsWith(SERVER_TUNNEL_PREFIX)
          ? s.functionId.slice(SERVER_TUNNEL_PREFIX.length)
          : s.functionId.slice(SERVER_SSH_PREFIX.length);
        if (!serverIds.has(serverId)) return false;
        // Hide tunnel/SSH cards once they've been explicitly stopped (no PTY session, idle state)
        if (!s.sessionId && s.state === "idle") return false;
        return true;
      }
      return true;
    })
    .sort((a, b) => a.functionId.localeCompare(b.functionId));
  statusList.innerHTML = statuses
    .map((status) => {
      const sessionPart = status.sessionId
        ? `session=${status.sessionId}`
        : "session=none";
      let title = STATUS_CARD_TITLE[status.functionId];
      if (!title) {
        if (
          status.functionId.startsWith(SERVER_TUNNEL_PREFIX) ||
          status.functionId.startsWith(SERVER_SSH_PREFIX)
        ) {
          const isTunnel = status.functionId.startsWith(SERVER_TUNNEL_PREFIX);
          const serverId = isTunnel
            ? status.functionId.slice(SERVER_TUNNEL_PREFIX.length)
            : status.functionId.slice(SERVER_SSH_PREFIX.length);
          const server = serversList.find((s) => s.id === serverId);
          const serverName = server?.name ?? serverId;
          title = `${isTunnel ? "Tunnel" : "SSH"} \u2013 ${serverName}`;
        } else {
          title = status.functionId;
        }
      }
      const isBoundary = status.functionId === BOUNDARY_AUTH_FUNCTION_ID;
      const pill =
        isBoundary && status.state === "authenticated"
          ? '<span class="status-pill status-pill-ok">Token active</span>'
          : isBoundary && status.state === "authenticating"
            ? '<span class="status-pill status-pill-warn">Logging in…</span>'
            : isBoundary && status.state === "unauthenticated"
              ? '<span class="status-pill status-pill-muted">No token</span>'
              : "";
      const stateLine = isBoundary
        ? ""
        : `<div class="status-row">state=<b>${escapeHtml(status.state)}</b></div>`;
      return `<article class="status-card${isBoundary ? ` status-card-boundary status-boundary-${escapeHtml(status.state)}` : ""}" data-function-id="${escapeHtml(status.functionId)}">
        <div class="status-card-head">
          <div class="status-title">${escapeHtml(title)}</div>
          ${pill}
        </div>
        ${stateLine}
        <div class="status-body">${escapeHtml(status.message)}</div>
        <div class="status-meta">${sessionPart} · updated ${formatDate(status.updatedAtMs)}</div>
      </article>`;
    })
    .join("");
}

async function navigateFromStatusCard(functionId: string) {
  if (functionId === OPENVPN_FUNCTION_ID) {
    setActiveView("openvpn");
    requestAnimationFrame(() => {
      vpnStartBtn.focus();
      terminalHost.scrollIntoView({ block: "start", behavior: "smooth" });
    });
    return;
  }

  if (functionId === CADDY_FUNCTION_ID) {
    setActiveView("caddy");
    requestAnimationFrame(() => {
      caddyStartBtn.focus();
      caddyTerminalHost.scrollIntoView({ block: "start", behavior: "smooth" });
    });
    return;
  }

  if (functionId === BOUNDARY_AUTH_FUNCTION_ID) {
    setActiveView("boundary");
    requestAnimationFrame(() => {
      boundaryStartBtn.focus();
      boundaryTerminalHost.scrollIntoView({
        block: "start",
        behavior: "smooth",
      });
    });
    return;
  }

  if (
    functionId.startsWith(SERVER_TUNNEL_PREFIX) ||
    functionId.startsWith(SERVER_SSH_PREFIX)
  ) {
    const serverId = functionId.startsWith(SERVER_TUNNEL_PREFIX)
      ? functionId.slice(SERVER_TUNNEL_PREFIX.length)
      : functionId.slice(SERVER_SSH_PREFIX.length);
    setActiveView("servers");
    await loadServers();
    if (!serversList.some((s) => s.id === serverId)) {
      serversStatus.textContent =
        "The server for this status no longer exists.";
      return;
    }
    selectServer(serverId);
    requestAnimationFrame(() => {
      const rows = Array.from(
        serversTbody.querySelectorAll<HTMLTableRowElement>("tr[data-id]"),
      );
      const row = rows.find((r) => r.dataset.id === serverId);
      if (row) {
        row.tabIndex = -1;
        row.focus({ preventScroll: true });
        row.scrollIntoView({ block: "center", behavior: "smooth" });
      }
    });
  }
}

function syncFunctionButtons() {
  const status = statusMap.get(OPENVPN_FUNCTION_ID);
  const running = Boolean(openvpnSessionId) || Boolean(status?.sessionId);
  vpnStartBtn.disabled = running;
  vpnStopBtn.disabled = !running;
}

function syncCaddyButtons() {
  const status = statusMap.get(CADDY_FUNCTION_ID);
  const running = Boolean(caddySessionId) || Boolean(status?.sessionId);
  caddyStartBtn.disabled = running;
  caddyStopBtn.disabled = !running;
}

function renderConfigPreview() {
  if (!appConfig) {
    openvpnConfigPreview.textContent = "Config not loaded.";
    return;
  }
  const username = appConfig.openvpn.username || "(empty)";
  const passwordLabel = appConfig.openvpn.password ? "********" : "(empty)";
  const otpLabel = appConfig.openvpn.otpSecret ? "configured" : "(empty)";
  const executable =
    appConfig.openvpn.executablePath || appConfig.openvpn.command || "(empty)";
  const ovpn = appConfig.openvpn.ovpnPath || "(empty)";
  const link =
    appConfig.openvpn.terminateOpenVpnWhenHelperExits === true
      ? "helper close stops VPN"
      : "VPN survives helper close";
  openvpnConfigPreview.textContent = `Executable: ${executable} | OVPN: ${ovpn} | Username: ${username} | Password: ${passwordLabel} | OTP secret: ${otpLabel} | ${link}`;
}

function renderConfigForm() {
  if (!appConfig) {
    return;
  }
  cfgTerminalCols.value = String(appConfig.app.defaultTerminalCols);
  cfgTerminalRows.value = String(appConfig.app.defaultTerminalRows);
  cfgOpenvpnExecutable.value =
    appConfig.openvpn.executablePath || appConfig.openvpn.command || "";
  cfgOpenvpnOvpn.value = appConfig.openvpn.ovpnPath;
  cfgOpenvpnExtraArgs.value = appConfig.openvpn.extraArgs;
  cfgOpenvpnUsername.value = appConfig.openvpn.username;
  cfgOpenvpnPassword.value = appConfig.openvpn.password;
  cfgOpenvpnOtpSecret.value = appConfig.openvpn.otpSecret;
  cfgOpenvpnTerminateWithHelper.checked =
    appConfig.openvpn.terminateOpenVpnWhenHelperExits === true;
  const b = appConfig.boundary ?? {
    cliPath: "",
    addr: "",
    authKind: "oidc" as const,
    oidcLoginMode: "browser" as const,
    authMethodId: "",
    loginName: "",
    password: "",
    oidcUsername: "",
    oidcPassword: "",
    oidcOtpSecret: "",
    connectSshOptions: "",
  };
  cfgBoundaryCli.value = b.cliPath;
  cfgBoundaryAddr.value = b.addr;
  cfgBoundaryAuthKind.value = b.authKind === "password" ? "password" : "oidc";
  cfgBoundaryOidcLoginMode.value =
    b.oidcLoginMode === "automated" ? "automated" : "browser";
  cfgBoundaryAuthMethod.value = b.authMethodId;
  cfgBoundaryConnectSshOptions.value = b.connectSshOptions ?? "";
  cfgBoundaryLogin.value = b.loginName;
  cfgBoundaryPassword.value = b.password;
  cfgBoundaryOidcUsername.value = b.oidcUsername ?? "";
  cfgBoundaryOidcPassword.value = b.oidcPassword ?? "";
  cfgBoundaryOidcOtpSecret.value = b.oidcOtpSecret ?? "";
  syncBoundaryPasswordFieldsVisibility();
  cfgSyncRepoRoot.value = syncSettings?.repoRoot ?? "";
  cfgCaddyFolder.value = syncSettings?.caddyFolder ?? "";
  renderConfigPreview();
}

function syncBoundaryPasswordFieldsVisibility() {
  const isPassword = cfgBoundaryAuthKind.value === "password";
  const isOidc = !isPassword;
  const isOidcAutomated =
    isOidc && cfgBoundaryOidcLoginMode.value === "automated";
  for (const el of cfgBoundaryPasswordOnly()) {
    el.style.display = isPassword ? "" : "none";
  }
  for (const el of cfgBoundaryOidcOnly()) {
    el.style.display = isOidc ? "" : "none";
  }
  for (const el of cfgBoundaryOidcAutomatedOnly()) {
    el.style.display = isOidcAutomated ? "" : "none";
  }
}

function readConfigForm(): AppConfig | null {
  const cols = Number(cfgTerminalCols.value);
  const rows = Number(cfgTerminalRows.value);
  if (!Number.isInteger(cols) || cols < 40 || cols > 300) {
    cfgStatus.textContent = "Terminal cols must be 40-300.";
    return null;
  }
  if (!Number.isInteger(rows) || rows < 10 || rows > 120) {
    cfgStatus.textContent = "Terminal rows must be 10-120.";
    return null;
  }

  const pw = cfgOpenvpnPassword.value;
  const otp = cfgOpenvpnOtpSecret.value.trim();
  const bpw = cfgBoundaryPassword.value;
  const boidcPw = cfgBoundaryOidcPassword.value;
  const boidcOtp = cfgBoundaryOidcOtpSecret.value.trim();
  return {
    app: {
      defaultTerminalCols: cols,
      defaultTerminalRows: rows,
    },
    openvpn: {
      executablePath: cfgOpenvpnExecutable.value.trim(),
      ovpnPath: cfgOpenvpnOvpn.value.trim(),
      extraArgs: cfgOpenvpnExtraArgs.value.trim(),
      username: cfgOpenvpnUsername.value.trim(),
      password: pw || appConfig?.openvpn.password || "",
      otpSecret: otp || appConfig?.openvpn.otpSecret || "",
      terminateOpenVpnWhenHelperExits: cfgOpenvpnTerminateWithHelper.checked,
    },
    boundary: {
      cliPath: cfgBoundaryCli.value.trim(),
      addr: cfgBoundaryAddr.value.trim(),
      authKind: cfgBoundaryAuthKind.value === "password" ? "password" : "oidc",
      oidcLoginMode:
        cfgBoundaryOidcLoginMode.value === "automated"
          ? "automated"
          : "browser",
      authMethodId: cfgBoundaryAuthMethod.value.trim(),
      loginName: cfgBoundaryLogin.value.trim(),
      password: bpw || appConfig?.boundary?.password || "",
      oidcUsername: cfgBoundaryOidcUsername.value.trim(),
      oidcPassword: boidcPw || appConfig?.boundary?.oidcPassword || "",
      oidcOtpSecret: boidcOtp || appConfig?.boundary?.oidcOtpSecret || "",
      connectSshOptions: cfgBoundaryConnectSshOptions.value.trim(),
    },
  };
}

async function loadConfig() {
  appConfig = await invoke<AppConfig>("get_app_config");
  renderConfigForm();
  cfgStatus.textContent = "Config loaded.";
  void refreshConfigPaths();
}

async function saveConfig() {
  const nextConfig = readConfigForm();
  if (!nextConfig) {
    return;
  }
  try {
    appConfig = await invoke<AppConfig>("save_app_config", {
      config: nextConfig,
    });
    console.info(
      "[SCP] save_app_config ok; password in response:",
      Boolean(appConfig.openvpn.password),
    );
    renderConfigForm();
    cfgStatus.textContent = "Config saved.";
    void refreshConfigPaths();
  } catch (e) {
    console.error("[SCP] save_app_config failed", e);
    cfgStatus.textContent = `Save failed: ${String(e)}`;
  }
}

async function loadSyncSettings() {
  try {
    syncSettings = await invoke<SyncSettings>("get_sync_settings");
  } catch {
    syncSettings = { repoRoot: "" };
  }
  cfgSyncRepoRoot.value = syncSettings.repoRoot ?? "";
  cfgCaddyFolder.value = syncSettings.caddyFolder ?? "";
}

type ConfigPaths = {
  appConfigPath: string;
};

async function refreshConfigPaths() {
  try {
    const paths = await invoke<ConfigPaths>("get_config_paths");
    cfgSyncAppPath.textContent = paths.appConfigPath;
  } catch (e) {
    cfgSyncAppPath.textContent = "(failed to resolve config path)";
  }
}

async function runConfigSync(mode: "pull" | "push") {
  const repoRoot =
    cfgSyncRepoRoot.value.trim() || syncSettings?.repoRoot?.trim() || "";
  if (!repoRoot) {
    cfgSyncStatus.textContent = "Set config repo root first.";
    return;
  }
  try {
    const next: SyncSettings = { repoRoot };
    await invoke("save_sync_settings", { settings: next });
    syncSettings = next;
  } catch {
    // Non-fatal; still attempt to run sync.
  }
  cfgSyncStatus.textContent =
    mode === "pull" ? "Copying from repo to app…" : "Copying from app to repo…";
  try {
    if (mode === "pull") {
      await invoke("sync_config_pull", { repoRoot });
      cfgSyncStatus.textContent = "Copied from repo to app config.";
    } else {
      await invoke("sync_config_push", { repoRoot });
      cfgSyncStatus.textContent = "Copied from app config to repo.";
    }
  } catch (e) {
    cfgSyncStatus.textContent = String(e);
  }
}

async function runConfigGit(mode: "pull" | "push") {
  const repoRoot = cfgSyncRepoRoot.value.trim() || syncSettings?.repoRoot?.trim() || "";
  if (!repoRoot) {
    cfgGitStatus.textContent = "Set config repo root first.";
    return;
  }
  try {
    const next: SyncSettings = { repoRoot };
    await invoke("save_sync_settings", { settings: next });
    syncSettings = next;
  } catch {
    // Non-fatal; still attempt to run git.
  }
  cfgGitStatus.textContent = mode === "pull" ? "Running git pull…" : "Running git add/commit/push…";
  try {
    const out =
      mode === "pull"
        ? await invoke<string>("config_git_pull", { repoRoot })
        : await invoke<string>("config_git_push", { repoRoot });
    cfgGitStatus.textContent = out;
  } catch (e) {
    cfgGitStatus.textContent = String(e);
  }
}

async function browseOpenvpnExecutable() {
  const selected = await openDialog({
    directory: false,
    multiple: false,
    title: "Select OpenVPN executable",
  });
  if (typeof selected === "string") {
    cfgOpenvpnExecutable.value = selected;
    cfgStatus.textContent = "Executable selected.";
  }
}

async function browseOpenvpnOvpnFile() {
  const selected = await openDialog({
    directory: false,
    multiple: false,
    title: "Select OVPN config file",
    filters: [{ name: "OVPN config", extensions: ["ovpn"] }],
  });
  if (typeof selected === "string") {
    cfgOpenvpnOvpn.value = selected;
    cfgStatus.textContent = "OVPN file selected.";
  }
}

async function browseBoundaryCli() {
  const selected = await openDialog({
    directory: false,
    multiple: false,
    title: "Select Boundary CLI executable",
    filters: [
      { name: "Executable", extensions: ["exe"] },
      { name: "All files", extensions: ["*"] },
    ],
  });
  if (typeof selected === "string") {
    cfgBoundaryCli.value = selected;
    cfgStatus.textContent = "Boundary CLI selected.";
  }
}

async function browseCaddyFolder() {
  const selected = await openDialog({
    directory: true,
    multiple: false,
    title: "Select Caddy folder (contains Caddyfile)",
  });
  if (typeof selected === "string") {
    cfgCaddyFolder.value = selected;
    cfgStatus.textContent = "Caddy folder selected.";
    const repoRoot =
      cfgSyncRepoRoot.value.trim() || syncSettings?.repoRoot?.trim() || "";
    syncSettings = {
      ...(syncSettings ?? { repoRoot }),
      repoRoot: repoRoot || (syncSettings?.repoRoot ?? ""),
      caddyFolder: selected,
    };
    void invoke("save_sync_settings", { settings: syncSettings }).catch(() => {
      // Non-fatal: config is still usable for current session.
    });
  }
}

async function autoDetectOpenvpnExecutable() {
  try {
    const detected = await invoke<string>("detect_openvpn_executable");
    cfgOpenvpnExecutable.value = detected;
    cfgStatus.textContent = `Detected: ${detected}`;
    const saveNow = window.confirm("Use this executable and save config now?");
    if (saveNow) {
      const nextConfig = readConfigForm();
      if (nextConfig) {
        appConfig = await invoke<AppConfig>("save_app_config", {
          config: nextConfig,
        });
        renderConfigForm();
        cfgStatus.textContent = "Config saved with detected executable.";
      }
    }
  } catch (e) {
    cfgStatus.textContent = String(e);
  }
}

async function refreshStatuses() {
  const statuses = await invoke<FunctionStatus[]>("list_function_statuses");
  for (const status of statuses) {
    statusMap.set(status.functionId, status);
    if (status.functionId === OPENVPN_FUNCTION_ID) {
      openvpnSessionId = status.sessionId;
    }
    if (status.functionId === CADDY_FUNCTION_ID) {
      caddySessionId = status.sessionId;
    }
    if (status.functionId.startsWith(SERVER_TUNNEL_PREFIX)) {
      const serverId = status.functionId.slice(SERVER_TUNNEL_PREFIX.length);
      if (status.sessionId) {
        serverTunnelSessions[serverId] = status.sessionId;
      } else {
        delete serverTunnelSessions[serverId];
      }
    }
    if (status.functionId.startsWith(SERVER_SSH_PREFIX)) {
      const serverId = status.functionId.slice(SERVER_SSH_PREFIX.length);
      if (status.sessionId) {
        serverSshSessions[serverId] = status.sessionId;
      } else {
        delete serverSshSessions[serverId];
      }
    }
    if (status.functionId === BOUNDARY_AUTH_FUNCTION_ID) {
      boundarySessionId = status.sessionId;
    }
  }
  renderStatusList();
  syncFunctionButtons();
  syncCaddyButtons();
  syncServerActions();
  syncBoundaryButtons();
}

async function startOpenVpnFunction() {
  if (!appConfig) {
    console.warn("[SCP] Start blocked: appConfig null");
    term.writeln(
      "\r\n[Config not loaded. Open Config page and reload/save first.]\r\n",
    );
    return;
  }
  if (
    !appConfig.openvpn.executablePath ||
    !appConfig.openvpn.ovpnPath ||
    !appConfig.openvpn.username ||
    !appConfig.openvpn.password
  ) {
    console.warn("[SCP] Start blocked: incomplete config", {
      executablePath: Boolean(appConfig.openvpn.executablePath),
      ovpnPath: Boolean(appConfig.openvpn.ovpnPath),
      username: Boolean(appConfig.openvpn.username),
      password: Boolean(appConfig.openvpn.password),
    });
    term.writeln(
      "\r\n[OpenVPN config incomplete. Fill executable/ovpn/username/password in Config page.]\r\n",
    );
    return;
  }

  term.clear();
  term.writeln("\x1b[32m[Starting OpenVPN function]\x1b[0m");
  try {
    const result = await invoke<StartFunctionResult>("start_openvpn_function", {
      command: "",
      username: "",
      password: "",
    });
    openvpnSessionId = result.sessionId;
    console.info(
      "[SCP] start_openvpn_function ok, sessionId=",
      result.sessionId,
    );
    syncFunctionButtons();
  } catch (e) {
    console.error("[SCP] start_openvpn_function failed", e);
    term.writeln(`\r\n\x1b[31m[Start failed: ${String(e)}]\x1b[0m\r\n`);
    syncFunctionButtons();
  }
}

async function stopOpenVpnFunction() {
  await invoke("stop_openvpn_function");
  syncFunctionButtons();
}

async function startCaddyFunction() {
  const folder =
    cfgCaddyFolder.value.trim() || syncSettings?.caddyFolder?.trim() || "";
  if (!folder) {
    caddyTerm.writeln("\r\n[Set Caddy folder in Config first.]\r\n");
    return;
  }
  caddyTerm.clear();
  caddyTerm.writeln("\x1b[32m[Starting Caddy]\x1b[0m");
  try {
    const result = await invoke<StartFunctionResult>("start_caddy_run", {
      cols: caddyTerm.cols,
      rows: caddyTerm.rows,
      folder,
    });
    caddySessionId = result.sessionId;
    syncCaddyButtons();
    void invoke("resize_pty", {
      id: result.sessionId,
      cols: caddyTerm.cols,
      rows: caddyTerm.rows,
    });
  } catch (e) {
    caddyTerm.writeln(`\r\n\x1b[31m[Start failed: ${String(e)}]\x1b[0m\r\n`);
    syncCaddyButtons();
  }
}

async function stopCaddyFunction() {
  await invoke("stop_caddy_run");
  syncCaddyButtons();
}

function writePtyIf(id: string | null, data: string) {
  if (!id) {
    return;
  }
  void invoke("write_pty", { id, data }).catch(() => {});
}

term.onData((data) => writePtyIf(openvpnSessionId, data));
caddyTerm.onData((data) => writePtyIf(caddySessionId, data));
serverTerm.onData((data) =>
  writePtyIf(
    selectedServerId ? (serverSshSessions[selectedServerId] ?? null) : null,
    data,
  ),
);
serverTunnelTerm.onData((data) =>
  writePtyIf(
    selectedServerId ? (serverTunnelSessions[selectedServerId] ?? null) : null,
    data,
  ),
);
boundaryTerm.onData((data) => writePtyIf(boundarySessionId, data));

window.addEventListener("resize", () => {
  fitAddon.fit();
  caddyFitAddon.fit();
  serverFit.fit();
  serverTunnelFit.fit();
  boundaryFit.fit();
  if (openvpnSessionId) {
    void invoke("resize_pty", {
      id: openvpnSessionId,
      cols: term.cols,
      rows: term.rows,
    });
  }
  if (caddySessionId) {
    void invoke("resize_pty", {
      id: caddySessionId,
      cols: caddyTerm.cols,
      rows: caddyTerm.rows,
    });
  }
  for (const sid of Object.values(serverTunnelSessions)) {
    void invoke("resize_pty", {
      id: sid,
      cols: serverTunnelTerm.cols,
      rows: serverTunnelTerm.rows,
    });
  }
  for (const sid of Object.values(serverSshSessions)) {
    void invoke("resize_pty", {
      id: sid,
      cols: serverTerm.cols,
      rows: serverTerm.rows,
    });
  }
  if (boundarySessionId) {
    void invoke("resize_pty", {
      id: boundarySessionId,
      cols: boundaryTerm.cols,
      rows: boundaryTerm.rows,
    });
  }
});

function formatServicesLines(services: ServerService[]): string {
  return services.map((s) => `${s.name} ${s.port}`).join("\n");
}

function matchesServerSearch(server: Server, rawTerm: string): boolean {
  const term = rawTerm.trim().toLowerCase();
  if (!term) {
    return true;
  }
  const haystack = [
    server.name,
    server.host,
    server.os,
    server.connectionMethod,
    server.sshUser,
    server.boundaryTargetId,
    ...server.services.map((service) => `${service.name} ${service.port}`),
  ]
    .join("\n")
    .toLowerCase();
  return haystack.includes(term);
}

function parseServicesText(text: string): ServerService[] {
  const out: ServerService[] = [];
  for (const line of text.split("\n")) {
    const p = line.trim().split(/\s+/);
    if (p.length >= 2) {
      const port = Number(p[p.length - 1]);
      const name = p.slice(0, -1).join(" ");
      if (name && Number.isInteger(port) && port > 0 && port <= 65535) {
        out.push({ name, port });
      }
    }
  }
  return out;
}

function restoreSelectedServerTerminalBuffers() {
  const clearSeq = "\x1b[2J\x1b[3J\x1b[H";
  serverTunnelTerm.write(clearSeq);
  serverTerm.write(clearSeq);
  if (!selectedServerId) {
    return;
  }
  const tunnelBuf = serverTunnelBuffers[selectedServerId];
  if (tunnelBuf) {
    serverTunnelTerm.write(tunnelBuf);
  }
  const sshBuf = serverSshBuffers[selectedServerId];
  if (sshBuf) {
    serverTerm.write(sshBuf);
  }
}

function selectServer(serverId: string | null) {
  selectedServerId = serverId;
  restoreSelectedServerTerminalBuffers();
  renderServersTable();
  syncServerActions();
}

function renderServersTable() {
  const filteredServers = serversList.filter((s) =>
    matchesServerSearch(s, serversSearchTerm),
  );
  serversTbody.innerHTML = filteredServers
    .map((s) => {
      const svc =
        s.services.map((x) => `${x.name}:${x.port}`).join(", ") || "—";
      const sel = s.id === selectedServerId ? ' class="selected"' : "";
      return `<tr data-id="${s.id}"${sel}>
        <td>${escapeHtml(s.name)}</td>
        <td>${escapeHtml(s.host)}</td>
        <td>${s.os}</td>
        <td>${s.connectionMethod}</td>
        <td>${s.sshPort}</td>
        <td>${escapeHtml(s.sshUser)}</td>
        <td class="services-cell">${escapeHtml(svc)}</td>
        <td class="row-actions">
          <button type="button" class="link-btn" data-edit="${s.id}">Edit</button>
          <button type="button" class="link-btn danger" data-del="${s.id}">Delete</button>
        </td>
      </tr>`;
    })
    .join("");
  if (selectedServerId && !serversList.some((s) => s.id === selectedServerId)) {
    selectedServerId = null;
  }
  serversTbody.querySelectorAll("tr[data-id]").forEach((row) => {
    row.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).closest("button")) {
        return;
      }
      const nextId = row.getAttribute("data-id");
      if (nextId) {
        selectServer(nextId);
      }
    });
  });
  serversTbody.querySelectorAll("[data-edit]").forEach((btn) => {
    btn.addEventListener("click", (ev) => {
      ev.stopPropagation();
      const id = (btn as HTMLButtonElement).dataset.edit!;
      openServerEditor(id);
    });
  });
  serversTbody.querySelectorAll("[data-del]").forEach((btn) => {
    btn.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      const id = (btn as HTMLButtonElement).dataset.del!;
      if (!confirm("Delete this server?")) {
        return;
      }
      try {
        serversList = await invoke<Server[]>("delete_server", { id });
        if (selectedServerId === id) {
          selectedServerId = null;
        }
        renderServersTable();
        syncServerActions();
        serversStatus.textContent = "Deleted.";
        updateServersStatus();
      } catch (err) {
        serversStatus.textContent = String(err);
      }
    });
  });
}

function updateServersStatus(message?: string) {
  if (message) {
    serversStatus.textContent = message;
    return;
  }
  const visibleCount = serversList.filter((s) =>
    matchesServerSearch(s, serversSearchTerm),
  ).length;
  const searchPart = serversSearchTerm.trim()
    ? ` ${visibleCount}/${serversList.length} visible.`
    : ` ${serversList.length} server(s).`;
  serversStatus.textContent = `Internal servers:${searchPart}`;
}

function syncServerEditorBoundaryFields() {
  const show = seMethod.value === "boundary";
  for (const el of seBoundaryOnly()) {
    el.style.display = show ? "" : "none";
  }
}

function openServerEditor(id: string | null) {
  editingServerId = id;
  if (id) {
    const s = serversList.find((x) => x.id === id);
    if (!s) {
      return;
    }
    seName.value = s.name;
    seHost.value = s.host;
    seOs.value = s.os ?? "linux";
    seMethod.value = s.connectionMethod;
    sePort.value = String(s.sshPort);
    seUser.value = s.sshUser;
    seBtarget.value = s.boundaryTargetId;
    seListenPort.value = String(s.boundaryListenPort || 6222);
    seServices.value = formatServicesLines(s.services);
  } else {
    seName.value = "";
    seHost.value = "";
    seOs.value = "linux";
    seMethod.value = "ssh";
    sePort.value = "22";
    seUser.value = "";
    seBtarget.value = "";
    seListenPort.value = "6222";
    seServices.value = "";
  }
  syncServerEditorBoundaryFields();
  serverEditorOverlay.classList.remove("hidden");
  serverEditorOverlay.setAttribute("aria-hidden", "false");
}

function closeServerEditor() {
  serverEditorOverlay.classList.add("hidden");
  serverEditorOverlay.setAttribute("aria-hidden", "true");
  editingServerId = null;
}

async function loadServers() {
  try {
    serversList = await invoke<Server[]>("list_servers");
    renderServersTable();
    updateServersStatus();
  } catch (e) {
    serversStatus.textContent = String(e);
  }
}

let boundarySearchResults: BoundaryTargetSummary[] = [];

async function runBoundarySearch() {
  const filter = boundarySearchInput.value.trim() || undefined;
  boundarySearchStatus.textContent = "Searching…";
  boundarySearchBtn.disabled = true;
  try {
    boundarySearchResults = await invoke<BoundaryTargetSummary[]>(
      "search_boundary_targets",
      {
        filter: filter || null,
      },
    );
    renderBoundarySearchResults();
    boundarySearchStatus.textContent =
      boundarySearchResults.length === 0
        ? "No targets found."
        : `${boundarySearchResults.length} target(s). Add any to your server list below.`;
  } catch (e) {
    boundarySearchStatus.textContent = String(e);
    boundarySearchResults = [];
    boundarySearchTbody.innerHTML = "";
  } finally {
    boundarySearchBtn.disabled = false;
  }
}

function renderBoundarySearchResults() {
  boundarySearchTbody.innerHTML = boundarySearchResults
    .map(
      (t) =>
        `<tr data-target-id="${escapeHtml(t.id)}">
          <td>${escapeHtml(t.name)}</td>
          <td class="mono">${escapeHtml(t.id)}</td>
          <td>${escapeHtml(t.targetType ?? "—")}</td>
          <td>${escapeHtml(t.description ?? "—")}</td>
          <td class="mono">${escapeHtml(t.address ?? "—")}</td>
          <td class="row-actions">
            <button type="button" class="link-btn boundary-add-target-btn" data-target-id="${escapeHtml(t.id)}" data-target-name="${escapeHtml(t.name)}" data-target-address="${escapeHtml(t.address ?? "")}">Add to list</button>
          </td>
        </tr>`,
    )
    .join("");
  boundarySearchTbody
    .querySelectorAll(".boundary-add-target-btn")
    .forEach((btn) => {
      btn.addEventListener("click", async (ev) => {
        ev.stopPropagation();
        const el = btn as HTMLButtonElement;
        const id = el.dataset.targetId!;
        const name = el.dataset.targetName!;
        const address = el.dataset.targetAddress ?? "";
        await addBoundaryTargetToList(id, name, address, el);
      });
    });
}

async function addBoundaryTargetToList(
  targetId: string,
  name: string,
  address: string,
  btn: HTMLButtonElement,
) {
  const server: Server = {
    id: "",
    name: name || targetId,
    host: address.trim(),
    os: "linux",
    connectionMethod: "boundary",
    sshPort: 22,
    sshUser: "",
    boundaryTargetId: targetId,
    boundaryListenPort: 6222,
    services: [],
  };
  try {
    btn.disabled = true;
    btn.textContent = "Adding…";
    serversList = await invoke<Server[]>("upsert_server", { server });
    renderServersTable();
    updateServersStatus(`Added "${server.name}" to internal server list.`);
    btn.textContent = "Added";
  } catch (e) {
    serversStatus.textContent = String(e);
    btn.disabled = false;
    btn.textContent = "Add to list";
  }
}

function syncServerActions() {
  const sel = selectedServerId
    ? serversList.find((s) => s.id === selectedServerId)
    : undefined;
  const tunnelRunning = Boolean(sel && serverTunnelSessions[sel.id]);
  const sshRunning = Boolean(sel && serverSshSessions[sel.id]);
  const isBoundary = sel?.connectionMethod === "boundary";
  const isWindows = sel?.os === "windows";
  serverTunnelStartBtn.disabled = !sel || !isBoundary || tunnelRunning;
  serverTunnelStopBtn.disabled = !tunnelRunning;
  serverSshBtn.disabled = !sel || sshRunning;
  serverRdpBtn.disabled = !sel || !isWindows;
  serverSshStopBtn.disabled = !sshRunning;
  serverUploadBtn.disabled = !sel;
  serverDownloadBtn.disabled = !sel;
}

function syncBoundaryButtons() {
  const busy = Boolean(boundarySessionId);
  boundaryStartBtn.disabled = busy;
  boundaryStopBtn.disabled = !busy;
}

navServers.addEventListener("click", () => {
  setActiveView("servers");
  void loadServers();
});

navBoundary.addEventListener("click", () => {
  setActiveView("boundary");
  requestAnimationFrame(() => boundaryFit.fit());
});

serverAddBtn.addEventListener("click", () => openServerEditor(null));

serversSearchInput.addEventListener("input", () => {
  serversSearchTerm = serversSearchInput.value;
  const selectedServer = selectedServerId
    ? serversList.find((s) => s.id === selectedServerId)
    : null;
  if (
    selectedServer &&
    !matchesServerSearch(selectedServer, serversSearchTerm)
  ) {
    selectServer(null);
  } else {
    renderServersTable();
    syncServerActions();
  }
  updateServersStatus();
});

serversSearchClearBtn.addEventListener("click", () => {
  serversSearchTerm = "";
  serversSearchInput.value = "";
  renderServersTable();
  syncServerActions();
  updateServersStatus();
  serversSearchInput.focus();
});

boundarySearchBtn.addEventListener("click", () => void runBoundarySearch());
boundarySearchInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    void runBoundarySearch();
  }
});

seMethod.addEventListener("change", () => syncServerEditorBoundaryFields());

seCancel.addEventListener("click", () => closeServerEditor());

seSave.addEventListener("click", async () => {
  const server: Server = {
    id: editingServerId ?? "",
    name: seName.value.trim(),
    host: seHost.value.trim(),
    os: (seOs.value as "linux" | "windows") || "linux",
    connectionMethod: seMethod.value as "ssh" | "boundary",
    sshPort: Math.min(65535, Math.max(1, Number(sePort.value) || 22)),
    sshUser: seUser.value.trim(),
    boundaryTargetId: seBtarget.value.trim(),
    boundaryListenPort:
      seMethod.value === "boundary"
        ? Math.min(65535, Math.max(1, Number(seListenPort.value) || 6222))
        : 0,
    services: parseServicesText(seServices.value),
  };
  if (!server.name) {
    serversStatus.textContent = "Name is required.";
    return;
  }
  if (server.connectionMethod === "ssh" && (!server.host || !server.sshUser)) {
    serversStatus.textContent =
      "Host and SSH user are required for direct SSH.";
    return;
  }
  if (server.connectionMethod === "boundary") {
    if (!server.boundaryTargetId) {
      serversStatus.textContent = "Boundary target ID is required.";
      return;
    }
    if (!server.sshUser) {
      serversStatus.textContent =
        "SSH username is required for Boundary (used to connect to localhost after tunnel).";
      return;
    }
  }
  try {
    serversList = await invoke<Server[]>("upsert_server", { server });
    closeServerEditor();
    renderServersTable();
    updateServersStatus("Saved.");
    syncServerActions();
  } catch (e) {
    serversStatus.textContent = String(e);
  }
});

serverTunnelStartBtn.addEventListener("click", async () => {
  if (!selectedServerId) {
    return;
  }
  serverTunnelBuffers[selectedServerId] = "";
  serverTunnelTerm.clear();
  serverTunnelTerm.writeln("\x1b[32m[Starting tunnel…]\x1b[0m");
  try {
    const r = await invoke<StartFunctionResult>("start_server_tunnel", {
      serverId: selectedServerId,
      cols: serverTunnelTerm.cols,
      rows: serverTunnelTerm.rows,
    });
    serverTunnelSessions[selectedServerId] = r.sessionId;
    syncServerActions();
    void invoke("resize_pty", {
      id: r.sessionId,
      cols: serverTunnelTerm.cols,
      rows: serverTunnelTerm.rows,
    });
  } catch (e) {
    serverTunnelTerm.writeln(`\x1b[31m${String(e)}\x1b[0m`);
  }
});

serverTunnelStopBtn.addEventListener("click", async () => {
  if (!selectedServerId) {
    return;
  }
  try {
    await invoke("stop_server_tunnel", { serverId: selectedServerId });
    delete serverTunnelSessions[selectedServerId];
    syncServerActions();
  } catch (e) {
    serversStatus.textContent = String(e);
  }
});

serverSshBtn.addEventListener("click", async () => {
  if (!selectedServerId || !appConfig) {
    return;
  }
  serverSshBuffers[selectedServerId] = "";
  serverTerm.clear();
  serverTerm.writeln("\x1b[32m[Connecting…]\x1b[0m");
  try {
    const r = await invoke<StartFunctionResult>("start_server_ssh", {
      serverId: selectedServerId,
      cols: serverTerm.cols,
      rows: serverTerm.rows,
    });
    serverSshSessions[selectedServerId] = r.sessionId;
    syncServerActions();
    void invoke("resize_pty", {
      id: r.sessionId,
      cols: serverTerm.cols,
      rows: serverTerm.rows,
    });
  } catch (e) {
    serverTerm.writeln(`\x1b[31m${String(e)}\x1b[0m`);
  }
});

serverSshStopBtn.addEventListener("click", async () => {
  if (!selectedServerId) {
    return;
  }
  try {
    await invoke("stop_server_ssh", { serverId: selectedServerId });
    delete serverSshSessions[selectedServerId];
    syncServerActions();
  } catch (e) {
    serversStatus.textContent = String(e);
  }
});

serverUploadBtn.addEventListener("click", async () => {
  if (!selectedServerId) {
    return;
  }
  const local = await openDialog({
    multiple: false,
    directory: false,
    title: "File to upload",
  });
  if (typeof local !== "string") {
    return;
  }
  const remote = window.prompt(
    "Remote path (e.g. /home/user/file.txt)",
    "/tmp/upload.bin",
  );
  if (remote === null) {
    return;
  }
  scpResultEl.textContent = "Uploading…";
  scpResultEl.className = "scp-result pending";
  try {
    const msg = await invoke<string>("scp_upload", {
      serverId: selectedServerId,
      localPath: local,
      remotePath: remote,
    });
    scpResultEl.textContent = msg;
    scpResultEl.className = "scp-result success";
  } catch (e) {
    scpResultEl.textContent = String(e);
    scpResultEl.className = "scp-result error";
  }
});

serverDownloadBtn.addEventListener("click", async () => {
  if (!selectedServerId) {
    return;
  }
  const remote = window.prompt(
    "Remote path (e.g. /var/log/syslog)",
    "/etc/hostname",
  );
  if (remote === null || !remote) {
    return;
  }
  const local = await saveDialog({
    title: "Save downloaded file as…",
    defaultPath: remote.split("/").pop() ?? "download",
  });
  if (typeof local !== "string") {
    return;
  }
  scpResultEl.textContent = "Downloading…";
  scpResultEl.className = "scp-result pending";
  try {
    const msg = await invoke<string>("scp_download", {
      serverId: selectedServerId,
      remotePath: remote,
      localPath: local,
    });
    scpResultEl.textContent = msg;
    scpResultEl.className = "scp-result success";
  } catch (e) {
    scpResultEl.textContent = String(e);
    scpResultEl.className = "scp-result error";
  }
});

serverRdpBtn.addEventListener("click", async () => {
  if (!selectedServerId) {
    return;
  }
  serversStatus.textContent = "Opening Remote Desktop…";
  try {
    await invoke("start_server_rdp", { serverId: selectedServerId });
    serversStatus.textContent = "RDP launch requested. Check your RDP client.";
  } catch (e) {
    serversStatus.textContent = String(e);
  }
});

cfgBoundaryAuthKind.addEventListener("change", () =>
  syncBoundaryPasswordFieldsVisibility(),
);
cfgBoundaryOidcLoginMode.addEventListener("change", () =>
  syncBoundaryPasswordFieldsVisibility(),
);

boundaryStartBtn.addEventListener("click", async () => {
  const kind = appConfig?.boundary?.authKind ?? cfgBoundaryAuthKind.value;
  if (
    kind === "password" &&
    !appConfig?.boundary?.password &&
    !cfgBoundaryPassword.value
  ) {
    boundaryTerm.writeln(
      "Password auth: set login and password in Config and Save first.",
    );
    return;
  }
  if (
    kind === "oidc" &&
    (appConfig?.boundary?.oidcLoginMode ?? cfgBoundaryOidcLoginMode.value) ===
      "automated" &&
    (
      !(appConfig?.boundary?.oidcUsername || cfgBoundaryOidcUsername.value) ||
      !(appConfig?.boundary?.oidcPassword || cfgBoundaryOidcPassword.value)
    )
  ) {
    boundaryTerm.writeln(
      "OIDC automation: set OIDC username and password in Config and Save first.",
    );
    return;
  }
  boundaryTerm.clear();
  try {
    const r = await invoke<StartFunctionResult>("start_boundary_auth", {
      cols: boundaryTerm.cols,
      rows: boundaryTerm.rows,
    });
    boundarySessionId = r.sessionId;
    syncBoundaryButtons();
    void invoke("resize_pty", {
      id: r.sessionId,
      cols: boundaryTerm.cols,
      rows: boundaryTerm.rows,
    });
  } catch (e) {
    boundaryTerm.writeln(`\x1b[31m${String(e)}\x1b[0m`);
  }
});

boundaryStopBtn.addEventListener("click", async () => {
  await invoke("stop_boundary_auth");
  syncBoundaryButtons();
});

serverTunnelVisibleToggle.addEventListener("change", () => {
  const tunnelBlock = document.querySelector<HTMLElement>(
    "#server-tunnel-block",
  );
  if (!tunnelBlock) {
    return;
  }
  tunnelBlock.style.display = serverTunnelVisibleToggle.checked ? "" : "none";
  requestAnimationFrame(() => {
    serverFit.fit();
    serverTunnelFit.fit();
  });
});

navStatus.addEventListener("click", () => {
  setActiveView("status");
});

statusList.addEventListener("click", (event) => {
  const card = (event.target as HTMLElement).closest<HTMLElement>(
    ".status-card[data-function-id]",
  );
  const functionId = card?.dataset.functionId;
  if (!functionId) {
    return;
  }
  void navigateFromStatusCard(functionId);
});

navOpenvpn.addEventListener("click", () => {
  setActiveView("openvpn");
});

navCaddy.addEventListener("click", () => {
  setActiveView("caddy");
});

navConfig.addEventListener("click", () => {
  setActiveView("config");
});

serverEditorOverlay.addEventListener("click", (e) => {
  if (e.target === serverEditorOverlay) {
    closeServerEditor();
  }
});

vpnStartBtn.addEventListener("click", () => {
  void startOpenVpnFunction();
});

vpnStopBtn.addEventListener("click", () => {
  void stopOpenVpnFunction();
});

caddyStartBtn.addEventListener("click", () => {
  void startCaddyFunction();
});

caddyStopBtn.addEventListener("click", () => {
  void stopCaddyFunction();
});

cfgSaveBtn.addEventListener("click", () => {
  void saveConfig();
});

cfgReloadBtn.addEventListener("click", () => {
  void loadConfig();
});

cfgOpenvpnBrowseBtn.addEventListener("click", () => {
  void browseOpenvpnExecutable();
});

cfgOpenvpnDetectBtn.addEventListener("click", () => {
  void autoDetectOpenvpnExecutable();
});

cfgOpenvpnOvpnBrowseBtn.addEventListener("click", () => {
  void browseOpenvpnOvpnFile();
});

cfgBoundaryBrowseBtn.addEventListener("click", () => {
  void browseBoundaryCli();
});

cfgCaddyBrowseBtn.addEventListener("click", () => {
  void browseCaddyFolder();
});

cfgSyncPullBtn.addEventListener("click", () => {
  void runConfigSync("pull");
});

cfgSyncPushBtn.addEventListener("click", () => {
  void runConfigSync("push");
});

cfgGitPullBtn.addEventListener("click", () => {
  void runConfigGit("pull");
});

cfgGitPushBtn.addEventListener("click", () => {
  void runConfigGit("push");
});

cfgSyncRepoRoot.addEventListener("change", () => {
  const repoRoot = cfgSyncRepoRoot.value.trim();
  syncSettings = { ...(syncSettings ?? { repoRoot: "" }), repoRoot };
  void invoke("save_sync_settings", { settings: syncSettings }).catch(() => {
    // Non-fatal; user will see error on actual sync if something is wrong.
  });
});

cfgCaddyFolder.addEventListener("change", () => {
  const caddyFolder = cfgCaddyFolder.value.trim();
  const repoRoot = cfgSyncRepoRoot.value.trim() || syncSettings?.repoRoot?.trim() || "";
  syncSettings = {
    ...(syncSettings ?? { repoRoot }),
    repoRoot: repoRoot || (syncSettings?.repoRoot ?? ""),
    caddyFolder,
  };
  void invoke("save_sync_settings", { settings: syncSettings }).catch(() => {});
});

void attachListeners();
void refreshStatuses();
void loadConfig();
void loadSyncSettings();
setActiveView("status");
syncFunctionButtons();
syncCaddyButtons();

window.addEventListener("beforeunload", () => {
  void detachListeners();
});
