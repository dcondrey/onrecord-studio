// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { AnalyticalColumnSummary, AnalyticalDatasetSummary, AnalyticalSchema } from '../types';

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
  reserved: [] as unknown[],
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
    controller: { reserve: (tables: unknown[]) => { discovery.reserved = tables; return () => { discovery.released += 1; }; }, refresh: () => { discovery.refreshed += 1; } },
  }),
}));
import FirstRunData from '../components/dashboard/FirstRunData';
import { useAppStore } from '../store/useAppStore';
import { firstRunTables, retryTables, useFirstRunDataStore } from '../store/useFirstRunDataStore';
import { capacityCompanions, closedReports, encampmentReportCoverage, OPEN_REPORTS } from '../utils/capacityLeads';

const imported = (rows: number) => Promise.resolve({ importedRows: rows, rejectedRows: 0, malformedRows: 0, duplicateKeyRows: 0, unconvertedCells: 0, tableName: 't', schemaWarnings: [] });
const offer = () => screen.queryByRole('button', { name: 'Get the tables' });
const UNFETCHED_KEY = 'onrecord-first-run-unfetched';
const TODAY = new Date('2026-10-07T12:00:00Z');

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(TODAY);
  window.localStorage.clear();
  useAppStore.setState({ firstRunDataSettled: false, workspaceView: 'overview' });
  useFirstRunDataStore.setState({ running: false, tables: [], statuses: {}, unfetched: [] });
  discovery.state = { schema: { datasets: [], columns: [] } };
  discovery.reserved = [];
  discovery.released = 0;
  discovery.refreshed = 0;
  bridge.respond = () => imported(10);
});
afterEach(() => { cleanup(); bridge.calls.length = 0; vi.useRealTimers(); });

describe('first-run core data', () => {
  it('is offered only on a loaded, empty library and is settled by skip or by a finished run, not by starting one', async () => {
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
    let finish!: () => void;
    bridge.respond = () => new Promise((resolve) => { finish = () => resolve(imported(10)); });
    const taken = render(<FirstRunData />);
    fireEvent.click(offer()!);
    // Mid-run the offer is not settled: an app closed now must not lose it silently.
    expect(useAppStore.getState().firstRunDataSettled).toBe(false);
    bridge.respond = () => imported(10);
    await act(async () => { finish(); });
    await waitFor(() => expect(useAppStore.getState().workspaceView).toBe('analysis'));
    expect(useAppStore.getState().firstRunDataSettled).toBe(true);
    expect(window.localStorage.getItem(UNFETCHED_KEY)).toBeNull();
    taken.unmount();
    render(<FirstRunData />);
    expect(offer()).toBeNull();
  });

  it('fetches every closed year the capacity lead needs, reports a failed table and records it as unfetched', async () => {
    bridge.respond = (args) => args.dataset === 'GET_IT_DONE_OPEN' ? Promise.reject(new Error('publisher returned 503')) : imported(10);
    render(<FirstRunData />);
    fireEvent.click(offer()!);
    expect(await screen.findByRole('button', { name: 'Close' })).toBeTruthy();
    expect(bridge.calls).toEqual([
      { dataset: 'OPERATING_BUDGET', year: null },
      { dataset: 'OPERATING_ACTUALS', year: null },
      { dataset: 'GET_IT_DONE_OPEN', year: null },
      { dataset: 'GET_IT_DONE_ENCAMPMENTS', year: 2022 },
      { dataset: 'GET_IT_DONE_ENCAMPMENTS', year: 2023 },
      { dataset: 'GET_IT_DONE_ENCAMPMENTS', year: 2024 },
      { dataset: 'GET_IT_DONE_ENCAMPMENTS', year: 2025 },
      { dataset: 'GET_IT_DONE_ENCAMPMENTS', year: 2026 },
      { dataset: 'CALLS_FOR_SERVICE', year: 2025 },
      { dataset: 'CALLS_FOR_SERVICE', year: 2026 },
    ]);
    expect(screen.getByRole('alert').textContent).toMatch(/publisher returned 503/);
    expect(screen.getAllByText('10 rows imported')).toHaveLength(9);
    expect(screen.getByText(/core public data is incomplete/)).toBeTruthy();
    expect(JSON.parse(window.localStorage.getItem(UNFETCHED_KEY)!)).toEqual([{ dataset: 'GET_IT_DONE_OPEN', year: null }]);
    expect(useAppStore.getState().workspaceView).toBe('overview');
    expect(discovery.released).toBe(1);

    // A retry fetches only what is missing, plus the current-year closed file the new open copy
    // would otherwise postdate, and a full library closes the card.
    bridge.calls.length = 0;
    bridge.respond = () => imported(10);
    fireEvent.click(screen.getByRole('button', { name: 'Get the missing tables' }));
    await waitFor(() => expect(screen.queryByRole('region')).toBeNull());
    expect(bridge.calls).toEqual([{ dataset: 'GET_IT_DONE_OPEN', year: null }, { dataset: 'GET_IT_DONE_ENCAMPMENTS', year: 2026 }]);
    expect(discovery.reserved).toEqual(bridge.calls);
    expect(window.localStorage.getItem(UNFETCHED_KEY)).toBeNull();
  });

  it('keeps fetching after the overview unmounts and does not pull the operator away from where they went', async () => {
    const pending: (() => void)[] = [];
    bridge.respond = () => new Promise((resolve) => { pending.push(() => resolve(imported(10))); });
    const view = render(<FirstRunData />);
    fireEvent.click(offer()!);
    expect(bridge.calls).toHaveLength(1);
    view.unmount();
    useAppStore.getState().setWorkspaceView('evidence');
    const total = firstRunTables(TODAY).length;
    expect(discovery.reserved).toEqual(firstRunTables(TODAY));
    // Discovery rereads the library after each import, so leads from early tables need not wait for the last.
    for (let index = 0; index < total; index += 1) {
      expect(discovery.refreshed).toBe(index);
      await act(async () => { pending.shift()!(); });
    }
    await waitFor(() => expect(useFirstRunDataStore.getState().running).toBe(false));
    expect(bridge.calls).toHaveLength(total);
    expect(useAppStore.getState().workspaceView).toBe('evidence');
    expect(useAppStore.getState().firstRunDataSettled).toBe(true);
    expect(discovery.released).toBe(1);
    expect(discovery.refreshed).toBe(total + 1);
  });

  it('retries a missing open copy together with a current-year closed file no older than it', () => {
    const open = { dataset: 'GET_IT_DONE_OPEN', year: null } as const;
    const closed = (year: number) => ({ dataset: 'GET_IT_DONE_ENCAMPMENTS', year }) as const;
    const budget = { dataset: 'OPERATING_BUDGET', year: null } as const;
    expect(retryTables([budget, closed(2025)], TODAY)).toEqual([budget, closed(2025)]);
    expect(retryTables([budget, open], TODAY)).toEqual([budget, open, closed(2026)]);
    expect(retryTables([closed(2026), budget, open, closed(2025)], TODAY)).toEqual([budget, open, closed(2026), closed(2025)]);
  });

  it('reports a run the app closed during as a partial library on the next launch', async () => {
    let reopen!: () => void;
    const held = new Promise<void>((resolve) => { reopen = resolve; });
    bridge.respond = () => held.then(() => imported(10));
    render(<FirstRunData />);
    fireEvent.click(offer()!);
    expect(bridge.calls).toHaveLength(1);
    const recorded = JSON.parse(window.localStorage.getItem(UNFETCHED_KEY)!) as unknown[];
    expect(recorded).toEqual(firstRunTables(TODAY));
    cleanup();

    // Relaunch: fresh modules read only what storage holds; the library is still empty.
    vi.resetModules();
    const Relaunched = (await import('../components/dashboard/FirstRunData')).default;
    const { useAppStore: relaunchedApp } = await import('../store/useAppStore');
    discovery.state = { schema: { datasets: [], columns: [] } };
    render(<Relaunched />);
    expect(relaunchedApp.getState().firstRunDataSettled).toBe(false);
    expect(screen.getByText(/core public data is incomplete/)).toBeTruthy();
    expect(screen.getAllByText(/Not fetched: the download stopped before reaching it/)).toHaveLength(recorded.length);
    // Let the first launch's run end so it fetches nothing during the next test.
    await act(async () => { reopen(); });
    await waitFor(() => expect(useFirstRunDataStore.getState().running).toBe(false));
  });

  it.each(['2026-01-01', '2026-07-01', '2026-07-02', '2026-12-31', '2027-01-01', '2027-07-02', '2025-07-01'])(
    'on %s fetches enough closed years for the capacity lead to compare two fiscal years with beds',
    (date) => {
      const now = new Date(`${date}T12:00:00Z`);
      const retrievedAt = now.toISOString();
      const columns: AnalyticalColumnSummary[] = [];
      const table = (tableName: string, sourceUri: string | null, names: string[]): AnalyticalDatasetSummary => {
        names.forEach((columnName) => columns.push({ tableName, columnName, logicalType: 'VARCHAR', originalName: null, nullable: true, dictionary: null }));
        return { snapshotId: tableName, datasetKind: 'CSV' as AnalyticalDatasetSummary['datasetKind'], displayName: tableName, tableName, importedRows: 1, rejectedRows: 0, sourceUri, sourceSha256: '', retrievedAt,
          observationPeriodStart: null, observationPeriodEnd: null, partialDataset: false, validationSummary: '{}', unconvertedByColumn: {} };
      };
      const reportColumns = ['service_request_id', 'date_requested', 'service_name'];
      const datasets = firstRunTables(now).flatMap((entry) => {
        if (entry.dataset === 'OPERATING_BUDGET') return [table('budget', null, ['dept_name', 'report_fy', 'budget_cycle', 'amount', 'account_number'])];
        if (entry.dataset === 'GET_IT_DONE_OPEN') return [table('open', OPEN_REPORTS, reportColumns)];
        if (entry.dataset === 'GET_IT_DONE_ENCAMPMENTS') return [table(`closed_${entry.year}`, closedReports(entry.year!), reportColumns)];
        return [];
      });
      const inventory = table('inventory', null, ['year', 'city', 'year_round_beds', 'total_beds']);
      const schema: AnalyticalSchema = { datasets: [...datasets, inventory], columns } as AnalyticalSchema;
      expect(capacityCompanions(schema, inventory)).not.toBeNull();
      // Beds end with the January 2024 count (layer HIC_ES_2014_2024); the budget names the
      // homelessness department from fiscal 2021.
      const comparable = encampmentReportCoverage(schema)!.fiscalYears.filter((year) => year >= 2021 && year <= 2024);
      expect(comparable.length).toBeGreaterThanOrEqual(2);
    },
  );
});
