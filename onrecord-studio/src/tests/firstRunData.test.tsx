// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

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
    },
  });
});
const bridge = vi.hoisted(() => ({
  calls: [] as { dataset: string; year: number | null }[],
  respond: null as unknown as (args: { dataset: string; year: number | null }) => Promise<unknown>,
}));
const discovery = vi.hoisted(() => ({
  state: { schema: null as { datasets: unknown[]; columns: unknown[] } | null },
  released: 0,
  refreshed: 0,
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: (name: string, args: { dataset: string; year: number | null }) => {
  if (name !== 'fetch_city_dataset') return Promise.resolve(null);
  bridge.calls.push(args);
  return bridge.respond(args);
} }));
vi.mock('../components/dashboard/AutomaticDiscoveryProvider', () => ({
  useAutomaticDiscovery: () => ({
    state: discovery.state,
    controller: { hold: () => () => { discovery.released += 1; }, refresh: () => { discovery.refreshed += 1; } },
  }),
}));
import FirstRunData from '../components/dashboard/FirstRunData';
import { useAppStore } from '../store/useAppStore';

const imported = (rows: number) => Promise.resolve({ importedRows: rows, rejectedRows: 0, malformedRows: 0, duplicateKeyRows: 0, unconvertedCells: 0, tableName: 't', schemaWarnings: [] });
const offer = () => screen.queryByRole('button', { name: 'Get the tables' });

beforeEach(() => {
  window.localStorage.clear();
  useAppStore.setState({ firstRunDataSettled: false, workspaceView: 'overview' });
  discovery.state = { schema: { datasets: [], columns: [] } };
  discovery.released = 0;
  discovery.refreshed = 0;
  bridge.respond = () => imported(10);
});
afterEach(() => { cleanup(); bridge.calls.length = 0; });

describe('first-run core data', () => {
  it('is offered only on a loaded, empty library and never again once skipped or taken', async () => {
    discovery.state = { schema: null };
    const unloaded = render(<FirstRunData />);
    expect(offer()).toBeNull();
    unloaded.unmount();
    discovery.state = { schema: { datasets: [{ snapshotId: 's' }], columns: [] } };
    const populated = render(<FirstRunData />);
    expect(offer()).toBeNull();
    populated.unmount();

    discovery.state = { schema: { datasets: [], columns: [] } };
    const skipped = render(<FirstRunData />);
    fireEvent.click(screen.getByRole('button', { name: 'Skip' }));
    expect(offer()).toBeNull();
    skipped.unmount();
    render(<FirstRunData />);
    expect(offer()).toBeNull();
    expect(window.localStorage.getItem('onrecord-first-run-data')).toBe('settled');
    cleanup();

    window.localStorage.clear();
    useAppStore.setState({ firstRunDataSettled: false });
    const taken = render(<FirstRunData />);
    fireEvent.click(offer()!);
    await waitFor(() => expect(useAppStore.getState().workspaceView).toBe('analysis'));
    taken.unmount();
    render(<FirstRunData />);
    expect(offer()).toBeNull();
  });

  it('reports a failed table and still fetches every other one', async () => {
    const year = new Date().getUTCFullYear();
    bridge.respond = (args) => args.dataset === 'GET_IT_DONE_OPEN' ? Promise.reject(new Error('publisher returned 503')) : imported(10);
    render(<FirstRunData />);
    fireEvent.click(offer()!);
    expect(await screen.findByRole('button', { name: 'Close' })).toBeTruthy();
    expect(bridge.calls).toEqual([
      { dataset: 'OPERATING_BUDGET', year: null },
      { dataset: 'OPERATING_ACTUALS', year: null },
      { dataset: 'GET_IT_DONE_OPEN', year: null },
      { dataset: 'GET_IT_DONE_ENCAMPMENTS', year: year - 1 },
      { dataset: 'GET_IT_DONE_ENCAMPMENTS', year },
      { dataset: 'CALLS_FOR_SERVICE', year: year - 1 },
      { dataset: 'CALLS_FOR_SERVICE', year },
    ]);
    expect(screen.getByRole('alert').textContent).toMatch(/publisher returned 503/);
    expect(screen.getAllByText('10 rows imported')).toHaveLength(6);
    expect(useAppStore.getState().workspaceView).toBe('overview');
    expect(discovery.released).toBe(1);
  });

  it('starts nothing further and navigates nowhere once unmounted mid-fetch', async () => {
    let finish!: (value: unknown) => void;
    bridge.respond = () => new Promise((resolve) => { finish = resolve; });
    const view = render(<FirstRunData />);
    fireEvent.click(offer()!);
    expect(bridge.calls).toHaveLength(1);
    view.unmount();
    await act(async () => { finish(await imported(10)); });
    expect(bridge.calls).toHaveLength(1);
    expect(useAppStore.getState().workspaceView).toBe('overview');
    // The discovery hold must still be released, or automatic checks stay paused for good.
    expect(discovery.released).toBe(1);
  });
});
