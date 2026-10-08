// @vitest-environment jsdom
//
// Before a runtime snapshot is measured, the throughput tiles have nothing to report. A pending or
// failed first read must never render as a measured zero or as "no runtimes".
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';

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
    useAppStore.getState().setPortalRuntimesFailed('DATABASE_UNAVAILABLE: busy');
    expect(useAppStore.getState().portalRuntimesStatus).toBe('ready');
    render(<ActivityDashboard />);
    expect(screen.getByText(/the figures below are from the last snapshot read/)).toBeTruthy();
  });

  const runtimeFor = (portalKey: string, displayName: string, failedTasks: number) => ({
    portal: { portalKey, displayName, canonicalUrl: `https://${portalKey}.example`, originUrl: `https://${portalKey}.example`, apiLayout: { requestsEndpoint: '', requestsPointer: '', metadataPointer: null, pageParameter: '', pageSizeParameter: '' }, verifiedAt: '2026-01-01T00:00:00Z', sourceKind: 'NEXT_REQUEST' as const },
    syncRunId: 'run', queuedTasks: 0, activeTasks: 0, completedTasks: 10, failedTasks, pagesProcessed: 0, requestsProcessed: 0, documentsProcessed: 0, isRunning: false, rateLimitTokens: 0,
    frontierExclusions: [], stoppedHosts: [],
  });

  it('opens a source\'s failures as that source\'s problems only', () => {
    useAppStore.setState({
      portalRuntimesStatus: 'ready',
      portalRuntimesError: null,
      portalRuntimes: [runtimeFor('city', 'City', 2), runtimeFor('city-west', 'City West', 1)],
      syncLogs: ['city-west: doc-9 failed — HTTP 500', 'city: doc-1 failed — HTTP 503', 'city: page 2 committed 20 requests.'],
    });
    render(<ActivityDashboard />);
    fireEvent.keyDown(screen.getByRole('img', { name: /^City:/ }), { key: 'Enter' });
    const stream = screen.getByRole('region', { name: 'Event stream' });
    expect(within(stream).getByText('city: doc-1 failed — HTTP 503')).toBeTruthy();
    expect(within(stream).queryByText('city-west: doc-9 failed — HTTP 500')).toBeNull();
    expect(within(stream).queryByText('city: page 2 committed 20 requests.')).toBeNull();
  });

  it('draws failed tasks as their own ink segment and states what was not collected', () => {
    useAppStore.setState({
      portalRuntimesStatus: 'ready',
      portalRuntimesError: null,
      portalRuntimes: [{
        portal: { portalKey: 'sd', displayName: 'City of San Diego', canonicalUrl: 'https://sd.example', originUrl: 'https://sd.example', apiLayout: { requestsEndpoint: '', requestsPointer: '', metadataPointer: null, pageParameter: '', pageSizeParameter: '' }, verifiedAt: '2026-01-01T00:00:00Z', sourceKind: 'NEXT_REQUEST' },
        syncRunId: 'run', queuedTasks: 41, activeTasks: 3, completedTasks: 1810, failedTasks: 4, pagesProcessed: 0, requestsProcessed: 0, documentsProcessed: 0, isRunning: true, rateLimitTokens: 0,
        frontierExclusions: [{ rule: 'off-scope-host', excludedLinks: 37 }],
        stoppedHosts: [],
      }],
    });
    render(<ActivityDashboard />);
    const bar = screen.getByRole('img', { name: /^City of San Diego:/ });
    expect(bar.getAttribute('aria-label')).toContain('1,810 done · 4 failed · 3 active · 41 queued of 1,858 tasks');
    expect(bar.textContent).toContain('Not collected: Links to hosts outside the source: 37 links refused');
    expect([...bar.querySelectorAll('*')].some((node) => (node.getAttribute('style') ?? '').includes('--viz-counted'))).toBe(false);
    expect(screen.getByText(/4 failed, all in City of San Diego|1,810 of 1,858 collection tasks done/)).toBeTruthy();
  });
});

