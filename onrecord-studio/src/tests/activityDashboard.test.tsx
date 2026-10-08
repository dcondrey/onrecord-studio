// @vitest-environment jsdom
//
// Before a runtime snapshot is measured, the throughput tiles have nothing to report. A pending or
// failed first read must never render as a measured zero or as "no runtimes".
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, within } from '@testing-library/react';

// Node 26's flag-gated `localStorage` shadows jsdom's, so replace it before the store import.
vi.hoisted(() => {
  const entries = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => entries.get(key) ?? null,
      setItem: (key: string, value: string) => void entries.set(key, String(value)),
      removeItem: (key: string) => void entries.delete(key),
      clear: () => entries.clear(),
      key: (index: number) => [...entries.keys()][index] ?? null,
      get length() {
        return entries.size;
      },
    },
  });
});

import { useAppStore } from '../store/useAppStore';
import ActivityDashboard from '../components/dashboard/ActivityDashboard';

afterEach(cleanup);

describe('ActivityDashboard', () => {
  it.each(['pending', 'failed'] as const)('shows no measured zero and no empty-runtime claim while the first snapshot is %s', (status) => {
    useAppStore.setState({ portalRuntimes: [], portalRuntimesStatus: status, portalRuntimesError: status === 'failed' ? 'DATABASE_UNAVAILABLE: busy' : null });
    render(<ActivityDashboard />);
    const tiles = screen.getByRole('region', { name: 'Session throughput' });
    expect(within(tiles).queryByText('0')).toBeNull();
    expect(screen.queryByText('No source runtimes')).toBeNull();
    expect(screen.queryByText('Nothing collecting')).toBeNull();
  });

  it('keeps the last snapshot when a later refresh fails', () => {
    useAppStore.setState({ portalRuntimes: [], portalRuntimesStatus: 'ready', portalRuntimesError: null });
    useAppStore.getState().setPortalRuntimesError('DATABASE_UNAVAILABLE: busy');
    expect(useAppStore.getState().portalRuntimesStatus).toBe('ready');
    render(<ActivityDashboard />);
    expect(screen.getByText(/the figures below are from the last snapshot read/)).toBeTruthy();
  });
});
