// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DimensionResult, WatchChange, WatchSummary } from '../types';
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
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../ipc', () => ({ invokeCommand: bridge.invoke }));
import WatchlistPanel, { describeDimension } from '../components/dashboard/WatchlistPanel';

function dimension(overrides: Partial<DimensionResult>): DimensionResult {
  return { dimension: 'DOCUMENTS', outcome: 'UNCHANGED', changeCount: 0, baselineAsOf: '2026-10-01T00:00:00Z', observedAsOf: '2026-10-02T00:00:00Z', reason: null, ...overrides };
}
function summary(id: string, label: string, lastDimensions: DimensionResult[] = []): WatchSummary {
  return { watch: { id, kind: 'AGENCY', target: label, label, createdAt: '2026-10-01T00:00:00Z', enabled: true }, lastCheckedAt: lastDimensions.length ? '2026-10-02T00:00:00Z' : null, lastDimensions };
}
function change(watchId: string, subject: string): WatchChange {
  return { id: `${watchId}:${subject}`, watchId, dimension: 'DOCUMENTS', changeKind: 'DOCUMENT_ADDED', subject, oldValue: null, newValue: `${subject}.pdf`, oldAsOf: '2026-10-01T00:00:00Z', newAsOf: '2026-10-02T00:00:00Z', basis: 'Present now and absent from the baseline.', detectedAt: '2026-10-02T00:00:00Z', portalKey: 'city' };
}
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }

beforeEach(() => { bridge.invoke.mockReset(); useAppStore.setState({ findingsRefreshRequests: 0 }); });
afterEach(cleanup);

describe('watchlist change detection', () => {
  it('discards a change list that arrives after another watch was selected', async () => {
    const slow = deferred<WatchChange[]>();
    bridge.invoke.mockImplementation((command: string, args: { watchId?: string | null }) => {
      if (command === 'list_watches') return Promise.resolve([summary('a', 'Alpha'), summary('b', 'Bravo')]);
      if (command === 'list_watch_changes') {
        if (args.watchId === 'a') return slow.promise;
        if (args.watchId === 'b') return Promise.resolve([change('b', 'bravo-record')]);
        return Promise.resolve([]);
      }
      return Promise.resolve(null);
    });
    render(<WatchlistPanel />);
    fireEvent.click(await screen.findByRole('button', { name: /^Alpha/ }));
    fireEvent.click(await screen.findByRole('button', { name: /^Bravo/ }));
    await screen.findByText('bravo-record');
    await act(async () => slow.resolve([change('a', 'alpha-record')]));
    expect(screen.queryByText('alpha-record')).toBeNull();
    expect(screen.getByText('bravo-record')).toBeTruthy();
  });

  it('reloads for the watch selected when a delete resolves, not the one selected when it began', async () => {
    const deletion = deferred<null>();
    bridge.invoke.mockImplementation((command: string, args: { watchId?: string | null }) => {
      if (command === 'list_watches') return Promise.resolve([summary('a', 'Alpha'), summary('b', 'Bravo'), summary('c', 'Charlie')]);
      if (command === 'delete_watch') return deletion.promise;
      if (command === 'list_watch_changes') return Promise.resolve(args.watchId ? [change(args.watchId, `${args.watchId}-record`)] : []);
      return Promise.resolve(null);
    });
    render(<WatchlistPanel />);
    fireEvent.click(await screen.findByRole('button', { name: /^Bravo/ }));
    await screen.findByText('b-record');
    fireEvent.click(screen.getByRole('button', { name: 'Stop watching Alpha' }));
    fireEvent.click(screen.getByRole('button', { name: /^Charlie/ }));
    await screen.findByText('c-record');
    await act(async () => deletion.resolve(null));
    await screen.findByText('Changes for Charlie');
    expect(screen.getByText('c-record')).toBeTruthy();
    expect(screen.queryByText('b-record')).toBeNull();
  });

  it('never words a dimension with no data as no change', async () => {
    const missing = dimension({ dimension: 'SOURCE_HEALTH', outcome: 'NOT_MEASURED', baselineAsOf: null, observedAsOf: null, reason: 'No sync run has completed since the baseline.' });
    const same = dimension({ dimension: 'DOCUMENTS', outcome: 'UNCHANGED' });
    bridge.invoke.mockImplementation((command: string) => Promise.resolve(command === 'list_watches' ? [summary('a', 'Alpha', [same, missing])] : []));
    render(<WatchlistPanel />);
    await screen.findByText(/No data this run: No sync run has completed since the baseline\./);
    expect(screen.getByText(/Observed, no change since 2026-10-01T00:00:00Z/)).toBeTruthy();
    expect(describeDimension(missing).text).not.toMatch(/no change/i);
    expect(describeDimension(dimension({ outcome: 'BASELINE_RECORDED', baselineAsOf: null })).text).not.toMatch(/no change/i);
  });

  it('states both values and both times, and says when the baseline lacked the subject', async () => {
    bridge.invoke.mockImplementation((command: string) => Promise.resolve(command === 'list_watches' ? [summary('a', 'Alpha')] : command === 'list_watch_changes' ? [change('a', 'city:2')] : null));
    render(<WatchlistPanel />);
    await screen.findByText('city:2.pdf');
    expect(screen.getByText('Not present at the baseline')).toBeTruthy();
    expect(screen.getByText(/Before, as of 2026-10-01T00:00:00Z/)).toBeTruthy();
    expect(screen.getByText(/Now, as of 2026-10-02T00:00:00Z/)).toBeTruthy();
    expect(screen.getByText(/Basis: Present now and absent from the baseline\./)).toBeTruthy();
  });

  it('checks every watch when a refresh completes, once per refresh', async () => {
    bridge.invoke.mockImplementation((command: string) => Promise.resolve(command === 'list_watches' ? [summary('a', 'Alpha')] : command === 'list_watch_changes' ? [] : []));
    render(<WatchlistPanel />);
    await screen.findByText('Alpha');
    expect(bridge.invoke.mock.calls.filter(([command]) => command === 'check_watches')).toHaveLength(0);
    await act(async () => { useAppStore.setState({ findingsRefreshRequests: 1 }); });
    await screen.findByText('Alpha');
    expect(bridge.invoke.mock.calls.filter(([command]) => command === 'check_watches')).toEqual([['check_watches', { watchId: null }]]);
  });
});
