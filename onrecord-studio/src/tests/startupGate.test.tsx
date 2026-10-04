// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

// Node 26's flag-gated `localStorage` shadows jsdom's, so replace it before store import.
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

const bridge = vi.hoisted(() => ({
  invoked: [] as { command: string; args: unknown }[],
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
  status: null as (() => Promise<unknown>) | null,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    bridge.handlers.set(name, handler);
    return Promise.resolve(() => bridge.handlers.delete(name));
  },
}));
vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: unknown) => {
    bridge.invoked.push({ command, args });
    if (command === 'get_startup_status' && bridge.status) return bridge.status();
    if (command === 'acknowledge_startup_notices') {
      return Promise.resolve(status({ version: 99, phase: 'ready', notices: [] }));
    }
    return Promise.reject(new Error(`unexpected command ${command}`));
  },
}));

import StartupGate, { STARTUP_STATUS_EVENT } from '../components/startup/StartupGate';
import { useAppStore } from '../store/useAppStore';
import type { StartupStatus } from '../types';

function status(overrides: Partial<StartupStatus>): StartupStatus {
  return {
    version: 1,
    phase: 'preparing',
    file: null,
    phaseStartedAtMs: Date.now(),
    error: null,
    notices: [],
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

function renderGate() {
  return render(
    <StartupGate>
      <div data-testid="workspace" />
    </StartupGate>,
  );
}

describe('StartupGate', () => {
  beforeEach(() => {
    bridge.invoked = [];
    bridge.handlers.clear();
    bridge.status = null;
    useAppStore.setState({ syncLogs: [] });
  });

  afterEach(() => {
    cleanup();
  });

  it('claims a one-time optimization only once the rewrite call has run past the threshold', async () => {
    bridge.status = () =>
      Promise.resolve(status({ version: 3, phase: 'optimizingStorage', file: 'onrecord_studio_core.db' }));
    renderGate();
    await waitFor(() => expect(screen.getByText('Checking storage format…')).toBeTruthy());
    expect(screen.queryByText(/one time/)).toBeNull();
    expect(screen.queryByTestId('workspace')).toBeNull();

    act(() =>
      bridge.handlers.get(STARTUP_STATUS_EVENT)!({
        payload: status({
          version: 4,
          phase: 'optimizingStorage',
          file: 'onrecord_studio_corpus.db',
          phaseStartedAtMs: Date.now() - 41_000,
        }),
      }),
    );
    expect(screen.getByText('Optimizing storage (one time)…')).toBeTruthy();
    expect(screen.getByText('onrecord_studio_corpus.db')).toBeTruthy();
    expect(screen.getByText('Elapsed: 0:41')).toBeTruthy();
  });

  // Identity: a poll begun while optimizing can resolve after the Ready event. Applying it would
  // put the startup screen back over a workspace whose state is already registered, forever,
  // since polling stops at Ready.
  it('never lets a stale poll reply replace a newer status', async () => {
    const stalePoll = deferred<StartupStatus>();
    bridge.status = () => stalePoll.promise;
    renderGate();
    await waitFor(() => expect(bridge.handlers.has(STARTUP_STATUS_EVENT)).toBe(true));

    act(() => bridge.handlers.get(STARTUP_STATUS_EVENT)!({ payload: status({ version: 9, phase: 'ready' }) }));
    expect(screen.getByTestId('workspace')).toBeTruthy();

    await act(async () => {
      stalePoll.resolve(status({ version: 4, phase: 'optimizingStorage', file: 'onrecord_studio_core.db' }));
      await stalePoll.promise;
    });
    expect(screen.getByTestId('workspace')).toBeTruthy();
    expect(screen.queryByText(/storage/i)).toBeNull();
  });

  it('logs each maintenance outcome once, keeps it until dismissed, then acknowledges it', async () => {
    const notice = {
      id: 'notice-refused-1',
      kind: 'storageNotRewritten' as const,
      file: 'onrecord_studio_core.db',
      message: 'Not optimized; Rewriting needs 10.6 GB free and only 2.0 GB is available.',
      recordedAt: '2026-10-03T00:00:00Z',
    };
    bridge.status = () => Promise.resolve(status({ version: 7, phase: 'ready', notices: [notice] }));
    const first = renderGate();
    await waitFor(() => expect(screen.getByText(/only 2\.0 GB is available/)).toBeTruthy());
    first.unmount();
    renderGate();
    await waitFor(() => expect(screen.getByText(/only 2\.0 GB is available/)).toBeTruthy());

    const logged = useAppStore.getState().syncLogs.filter((line) => line.includes('only 2.0 GB'));
    expect(logged).toHaveLength(1);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    });
    expect(bridge.invoked).toContainEqual({
      command: 'acknowledge_startup_notices',
      args: { ids: ['notice-refused-1'] },
    });
    expect(screen.queryByText(/only 2\.0 GB is available/)).toBeNull();
  });

  it('shows why startup failed instead of a screen that never advances', async () => {
    bridge.status = () =>
      Promise.resolve(status({ version: 5, phase: 'failed', error: 'The file is neither a DuckDB nor a SQLite database.' }));
    renderGate();
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/neither a DuckDB/));
    expect(screen.queryByTestId('workspace')).toBeNull();
  });
});
