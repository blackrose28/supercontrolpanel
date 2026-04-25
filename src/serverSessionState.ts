export type ServerFunctionStatus = {
  functionId: string;
  state?: string | null;
  sessionId?: string | null;
};

const ACTIVE_SERVER_PTY_STATES = new Set(['running', 'starting']);

export function hasActiveServerPtySession(status: ServerFunctionStatus): boolean {
  const state = String(status.state || '').trim().toLowerCase();
  const sessionId = String(status.sessionId || '').trim();
  return Boolean(sessionId) && ACTIVE_SERVER_PTY_STATES.has(state);
}

export function syncServerSessionFromStatus(
  sessions: Record<string, string>,
  functionPrefix: string,
  status: ServerFunctionStatus,
): string | null {
  if (!status.functionId.startsWith(functionPrefix)) {
    return null;
  }

  const serverId = status.functionId.slice(functionPrefix.length);
  if (!serverId) {
    return null;
  }

  const sessionId = String(status.sessionId || '').trim();
  if (hasActiveServerPtySession(status)) {
    sessions[serverId] = sessionId;
  } else {
    delete sessions[serverId];
  }

  return serverId;
}
