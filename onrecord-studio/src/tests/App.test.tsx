// @vitest-environment jsdom
//
// The only DOM suite in this project. `vitest.config.ts` keeps the default environment at `node`
// so the pure suites stay dependency-free; this docblock opts just this file into jsdom.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render, waitFor } from '@testing-library/react';

// Node 26 defines its own `localStorage` global, gated behind `--localstorage-file`. Its getter
// returns undefined and it shadows the one jsdom installs, so `window.localStorage` is undefined
// under Vitest's jsdom environment even though the document origin is fine. `useAppStore` reads it
// at module scope, so the replacement has to exist before any import evaluates -- which is exactly
// what `vi.hoisted` provides. Delete this once Node's global stops being flag-gated.
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
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  invoked: [] as string[],
  setTheme: [] as (string | null)[],
  failSavedPortals: false,
  listenerFailures: 0,
  listenerAttempts: 0,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    bridge.listenerAttempts += 1;
    if (bridge.listenerFailures > 0) {
      bridge.listenerFailures -= 1;
      return Promise.reject(new Error('event bridge unavailable'));
    }
    bridge.listeners.set(name, handler);
    return Promise.resolve(() => bridge.listeners.delete(name));
  },
}));
vi.mock('@tauri-apps/api/app', () => ({
  setTheme: (theme: string | null) => {
    bridge.setTheme.push(theme);
    return Promise.resolve();
  },
}));
vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string) => {
    bridge.invoked.push(command);
    if (command === 'list_saved_portals' && bridge.failSavedPortals) {
      return Promise.reject(new Error('source catalog unavailable'));
    }
    if (command === 'get_ocr_capability') {
      return Promise.resolve({
        engine: 'tesseract',
        engineAvailable: true,
        engineVersion: 'tesseract 5.5.1',
        languagePacks: ['eng'],
        languageProbeComplete: true,
        pdfRenderer: 'pdftoppm',
        pdfRendererAvailable: true,
        pdfRendererVersion: 'pdftoppm 25.06.0',
        imageOcrAvailable: true,
        pdfOcrAvailable: true,
        diagnostics: [],
      });
    }
    return Promise.resolve([]);
  },
}));

// The dashboard tree is stubbed deliberately. What is under test here is App's own shell: theme
// hydration, the workspace bootstrap, and the telemetry switch. Mounting the real Sidebar, Header,
// MainLayout, and CommandPalette would pull in the whole component graph and test none of that.
vi.mock('../components/dashboard/Sidebar', () => ({ default: () => <div data-testid="sidebar" /> }));
vi.mock('../components/dashboard/Header', () => ({ default: () => <div data-testid="header" /> }));
vi.mock('../components/dashboard/MainLayout', () => ({ default: () => <div data-testid="main" /> }));
vi.mock('../components/dashboard/CommandPalette', () => ({
  default: () => <div data-testid="palette" />,
}));

import App from '../App';
import { useAppStore } from '../store/useAppStore';

function telemetry(sequence: string, type: string, payload: unknown) {
  return { payload: { protocolVersion: 1, sequence, type, payload } };
}

function deliver(event: { payload: unknown }) {
  const handler = bridge.listeners.get('ingestion-telemetry');
  if (!handler) throw new Error('App never registered an ingestion-telemetry listener');
  act(() => handler(event));
}

describe('App', () => {
  beforeEach(() => {
    cleanup();
    bridge.listeners.clear();
    bridge.invoked.length = 0;
    bridge.setTheme.length = 0;
    bridge.failSavedPortals = false;
    bridge.listenerFailures = 0;
    bridge.listenerAttempts = 0;
    window.localStorage.clear();
    useAppStore.setState({
      syncLogs: [],
      portals: [],
      workspaceInitializationStatus: 'loading',
      workspaceInitializationError: null,
      ocrCapability: null,
    });
    window.matchMedia = vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    }));
  });

  it('hydrates the resolved theme onto the document and persists the preference', async () => {
    render(<App />);
    await waitFor(() => expect(bridge.listeners.has('ingestion-telemetry')).toBe(true));

    // `matches: false` above means the system prefers light, and the default preference is `auto`,
    // so the resolved theme must be light rather than the raw preference string.
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(document.documentElement.dataset.themePreference).toBe(
      useAppStore.getState().theme,
    );
    expect(window.localStorage.getItem('onrecord-theme')).toBe(useAppStore.getState().theme);
    // `isTauri()` is false in a browser test, so the native theme call must be skipped.
    expect(bridge.setTheme).toEqual([]);
  });

  it('bootstraps the workspace over the native bridge', async () => {
    render(<App />);
    await waitFor(() => expect(bridge.invoked).toContain('list_saved_portals'));
    expect(bridge.invoked).toContain('list_portal_runtimes');
    expect(bridge.invoked).toContain('get_agency_performance_profiles');
    expect(bridge.invoked).toContain('get_ocr_capability');
    await waitFor(() => expect(useAppStore.getState().workspaceInitializationStatus).toBe('ready'));
    await waitFor(() => expect(useAppStore.getState().ocrCapability?.pdfOcrAvailable).toBe(true));
  });

  it('reconnects the telemetry listener after a transient registration failure', async () => {
    vi.useFakeTimers();
    bridge.listenerFailures = 1;
    try {
      render(<App />);
      await act(async () => Promise.resolve());
      expect(bridge.listenerAttempts).toBe(1);
      expect(useAppStore.getState().syncLogs.join('\n')).toContain(
        'Telemetry listener failed; retrying in 1s',
      );

      await act(async () => vi.advanceTimersByTimeAsync(1_000));
      expect(bridge.listenerAttempts).toBe(2);
      expect(bridge.listeners.has('ingestion-telemetry')).toBe(true);
    } finally {
      cleanup();
      vi.useRealTimers();
    }
  });

  it('records a failed source-catalog load as unavailable instead of an empty workspace', async () => {
    bridge.failSavedPortals = true;
    render(<App />);

    await waitFor(() => expect(useAppStore.getState().workspaceInitializationStatus).toBe('error'));
    expect(useAppStore.getState().workspaceInitializationError).toContain('source catalog unavailable');
    expect(useAppStore.getState().syncLogs.join('\n')).toContain('Workspace initialization failed');
  });

  // REGRESSION(ipc): this variant was absent from App's switch, so a legacy-database migration --
  // the one event that reports evidence being copied between database files -- was dropped without
  // a trace. The `default: assertNever(payload)` arm added alongside it makes any future backend
  // variant a `tsc` error rather than another silent drop.
  it('reports the legacy core database migration it used to drop', async () => {
    render(<App />);
    await waitFor(() => expect(bridge.listeners.has('ingestion-telemetry')).toBe(true));

    deliver(
      telemetry('1', 'legacyCoreDatabaseMigrated', {
        backupPath: '/evidence/core.sqlite.backup',
        copiedTables: 12,
        copiedRows: 4096,
        tablesAbsentFromLegacy: ['corpus_segments'],
        tablesDroppedFromLegacy: ['scratch_notes'],
      }),
    );

    const log = useAppStore.getState().syncLogs.join('\n');
    expect(log).toContain('/evidence/core.sqlite.backup');
    expect(log).toContain('4,096');
    expect(log).toContain('corpus_segments');
    expect(log).toContain('scratch_notes');
  });

  // The exhaustiveness guard is compile-time. At runtime it must stay inert: this callback runs
  // outside the `.catch()` on the `listen()` promise, so throwing on an unknown variant would kill
  // telemetry for every event after it.
  it('survives a variant it does not recognize and keeps handling later events', async () => {
    render(<App />);
    await waitFor(() => expect(bridge.listeners.has('ingestion-telemetry')).toBe(true));

    expect(() => deliver(telemetry('7', 'somethingTheBackendAddedLater', {}))).not.toThrow();
    expect(useAppStore.getState().syncLogs.join('\n')).toContain(
      'somethingTheBackendAddedLater',
    );

    deliver(telemetry('8', 'corpusReconciliationStarted', {}));
    expect(useAppStore.getState().syncLogs.join('\n')).toContain('derived corpus index');
  });
});
