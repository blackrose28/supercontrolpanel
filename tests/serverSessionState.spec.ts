import { expect, test } from '@playwright/test';
import { syncServerSessionFromStatus } from '../src/serverSessionState';

test('clears active server session when status is terminal but still carries sessionId for logs', () => {
  const sessions: Record<string, string> = { 'srv-1': 'pty-previous' };

  syncServerSessionFromStatus(sessions, 'server-tunnel-', {
    functionId: 'server-tunnel-srv-1',
    state: 'error',
    sessionId: 'pty-previous',
  });

  expect(sessions['srv-1']).toBeUndefined();
});

test('tracks active server session only while status is running', () => {
  const sessions: Record<string, string> = {};

  syncServerSessionFromStatus(sessions, 'server-tunnel-', {
    functionId: 'server-tunnel-srv-1',
    state: 'running',
    sessionId: 'pty-active',
  });

  expect(sessions['srv-1']).toBe('pty-active');
});
