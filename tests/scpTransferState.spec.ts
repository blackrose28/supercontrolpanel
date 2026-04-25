import { expect, test } from '@playwright/test';
import { applyScpTransferStatusUpdate, type ScpTransferUiState } from '../src/scpTransferState';

function emptyState(): ScpTransferUiState {
  return {
    counter: 0,
    history: [],
    sessionIds: {},
    sessionOutput: {},
  };
}

test('creates visible app history and binds session for CC-started SCP transfer', () => {
  const next = applyScpTransferStatusUpdate(
    emptyState(),
    {
      transferId: 'tr-1',
      sessionId: 'pty-1',
      state: 'running',
      direction: 'upload',
      localPath: '/tmp/report.csv',
      remotePath: '/srv/report.csv',
      message: 'start_scp_upload',
    },
    1_700_000_000_000,
  );

  expect(next.history).toHaveLength(1);
  expect(next.history[0]).toMatchObject({
    id: 'cc-scp-tr-1',
    title: 'CC upload',
    detail: 'start_scp_upload',
    state: 'pending',
    createdAtMs: 1_700_000_000_000,
    progressPercent: 0,
    source: '/tmp/report.csv',
    destination: '/srv/report.csv',
  });
  expect(next.sessionIds['pty-1']).toBe('cc-scp-tr-1');
  expect(next.sessionOutput['pty-1']).toBe('');
});

test('updates existing CC transfer history with progress and terminal status', () => {
  const running = applyScpTransferStatusUpdate(
    emptyState(),
    {
      transferId: 'tr-1',
      sessionId: 'pty-1',
      state: 'running',
      direction: 'download',
      localPath: '/tmp/report.csv',
      remotePath: '/srv/report.csv',
      percent: 42,
    },
    1_700_000_000_000,
  );

  const finished = applyScpTransferStatusUpdate(
    running,
    {
      transferId: 'tr-1',
      sessionId: 'pty-1',
      state: 'succeeded',
      direction: 'download',
      localPath: '/tmp/report.csv',
      remotePath: '/srv/report.csv',
      message: 'Transfer finished.',
    },
    1_700_000_010_000,
  );

  expect(finished.history).toHaveLength(1);
  expect(finished.history[0]).toMatchObject({
    id: 'cc-scp-tr-1',
    title: 'CC download',
    detail: 'Transfer finished.',
    state: 'success',
    createdAtMs: 1_700_000_000_000,
    progressPercent: 100,
    source: '/srv/report.csv',
    destination: '/tmp/report.csv',
  });
});
