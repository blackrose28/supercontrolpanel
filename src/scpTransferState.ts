export type ScpTransferState = "pending" | "success" | "error";

export type ScpTransferEntry = {
  id: string;
  title: string;
  detail: string;
  state: ScpTransferState;
  createdAtMs: number;
  progressPercent: number | null;
  source?: string;
  destination?: string;
};

export type ScpTransferStatusPayload = {
  transferId?: string | null;
  sessionId?: string | null;
  state?: string | null;
  direction?: string | null;
  localPath?: string | null;
  remotePath?: string | null;
  message?: string | null;
  reasonDetail?: string | null;
  phase?: string | null;
  percent?: number | null;
};

export type ScpTransferUiState = {
  counter: number;
  history: ScpTransferEntry[];
  sessionIds: Record<string, string>;
  sessionOutput: Record<string, string>;
};

const DEFAULT_HISTORY_LIMIT = 8;

function normalizeText(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function normalizeDirection(value: unknown): "upload" | "download" | "transfer" {
  const direction = normalizeText(value).toLowerCase();
  if (direction === "upload" || direction === "download") {
    return direction;
  }
  return "transfer";
}

function normalizeStatusState(value: unknown): string {
  return normalizeText(value).toLowerCase() || "running";
}

function clampPercent(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return null;
  }
  return Math.max(0, Math.min(100, Math.round(value)));
}

function entryStateFromStatus(statusState: string): ScpTransferState {
  if (statusState === "succeeded") {
    return "success";
  }
  if (statusState === "failed" || statusState === "canceled") {
    return "error";
  }
  return "pending";
}

function detailFromStatus(payload: ScpTransferStatusPayload, statusState: string): string {
  const message = normalizeText(payload.message);
  if (message) {
    return message;
  }

  const reasonDetail = normalizeText(payload.reasonDetail);
  if (reasonDetail) {
    return reasonDetail;
  }

  if (statusState === "succeeded") {
    return "Transfer finished.";
  }
  if (statusState === "failed") {
    return "Transfer failed.";
  }
  if (statusState === "canceled") {
    return "Transfer canceled.";
  }
  if (statusState === "starting" || statusState === "queued") {
    return "Starting transfer…";
  }
  return "Transferring…";
}

function progressFromStatus(
  payload: ScpTransferStatusPayload,
  entryState: ScpTransferState,
  existing?: ScpTransferEntry,
): number | null {
  const percent = clampPercent(payload.percent);
  if (percent !== null) {
    return percent;
  }
  if (entryState === "success") {
    return 100;
  }
  if (entryState === "pending") {
    return existing?.progressPercent ?? 0;
  }
  return null;
}

function sourceDestinationForDirection(payload: ScpTransferStatusPayload): {
  source?: string;
  destination?: string;
} {
  const direction = normalizeDirection(payload.direction);
  const localPath = normalizeText(payload.localPath);
  const remotePath = normalizeText(payload.remotePath);

  if (direction === "download") {
    return {
      source: remotePath || undefined,
      destination: localPath || undefined,
    };
  }

  return {
    source: localPath || undefined,
    destination: remotePath || undefined,
  };
}

export function scpTransferEntryIdFromTransferId(transferId: string): string {
  return `cc-scp-${transferId}`;
}

export function applyScpTransferStatusUpdate(
  current: ScpTransferUiState,
  payload: ScpTransferStatusPayload,
  nowMs: number,
  historyLimit = DEFAULT_HISTORY_LIMIT,
): ScpTransferUiState {
  const transferId = normalizeText(payload.transferId);
  if (!transferId) {
    return current;
  }

  const entryId = scpTransferEntryIdFromTransferId(transferId);
  const existing = current.history.find((entry) => entry.id === entryId);
  const statusState = normalizeStatusState(payload.state);
  const entryState = entryStateFromStatus(statusState);
  const direction = normalizeDirection(payload.direction);
  const { source, destination } = sourceDestinationForDirection(payload);
  const nextEntry: ScpTransferEntry = {
    id: entryId,
    title: `CC ${direction}`,
    detail: detailFromStatus(payload, statusState),
    state: entryState,
    createdAtMs: existing?.createdAtMs ?? nowMs,
    progressPercent: progressFromStatus(payload, entryState, existing),
    source: source ?? existing?.source,
    destination: destination ?? existing?.destination,
  };

  const history = [nextEntry, ...current.history.filter((entry) => entry.id !== entryId)].slice(
    0,
    historyLimit,
  );
  const sessionIds = { ...current.sessionIds };
  const sessionOutput = { ...current.sessionOutput };
  const sessionId = normalizeText(payload.sessionId);

  if (sessionId) {
    if (entryState === "pending") {
      sessionIds[sessionId] = entryId;
      sessionOutput[sessionId] = sessionOutput[sessionId] ?? "";
    } else {
      delete sessionIds[sessionId];
      delete sessionOutput[sessionId];
    }
  }

  return {
    counter: current.counter,
    history,
    sessionIds,
    sessionOutput,
  };
}
