import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/service-accountability.json';
import type { AnalyticalSchema, DataMaintenanceStatus, SqlQueryResult } from '../types';
import type { AutomaticProfile } from '../utils/automaticDiscovery';
import { dataGaps } from '../utils/dataGaps';
import { DiscoveryController, type DiscoveryTransport } from '../utils/discoveryController';

const open = 'https://seshat.datasd.org/get_it_done_reports/get_it_done_requests_open_datasd.csv';
const closed2026 = 'https://seshat.datasd.org/get_it_done_reports/get_it_done_requests_closed_2026_datasd.csv';
const base = fixture.schema as AnalyticalSchema;
const response = (columns: string[], rows: SqlQueryResult['rows']): SqlQueryResult => ({ columns, rows, rowCount: rows.length, elapsedMs: 1, truncated: false, provenance: [] });
const table = (snapshotId: string, sourceUri: string, retrievedAt: string, validationSummary = '{}') => ({ ...base.datasets[0], snapshotId, tableName: `analysis_${snapshotId}`, sourceUri, retrievedAt, validationSummary });
const catalog = (...datasets: ReturnType<typeof table>[]): AnalyticalSchema => ({ ...base, datasets,
  columns: datasets.flatMap((dataset) => base.columns.filter((column) => column.tableName === 'analysis_fixture').map((column) => ({ ...column, tableName: dataset.tableName }))) });
const query = (id: string) => fixture.queries.find((item) => item.id === id)!;
const profile = (unrecorded: number | null): AutomaticProfile => ({ fingerprint: 'f', checkedAt: 'now', results: [
  { query: query('service_backlog'), result: response(['category'], [['Pothole']]) },
  ...(unrecorded === null ? [] : [{ query: query('service_movement'), result: response(['category', 'all_no_outcome_record'], [['Pothole', unrecorded]]) }]),
] });

describe('missing tables the app fetches for itself', () => {
  it('asks for a newer copy of the open file at seven days, not six', () => {
    const schema = catalog(table('first', open, '2026-09-14T17:00:00Z'));
    expect(dataGaps(schema, { first: profile(null) }, '2026-09-20T09:00:00Z')).toEqual([]);
    expect(dataGaps(schema, { first: profile(null) }, '2026-09-21T09:00:00Z').map((gap) => gap.fetch)).toEqual([{ dataset: 'GET_IT_DONE_OPEN', year: null }]);
    // No backlog was counted in this table, so a second copy has nothing to compare.
    expect(dataGaps(schema, {}, '2026-09-21T09:00:00Z')).toEqual([]);
  });

  it('asks for the closed-reports year only while departed reports have no record and no table covers them', () => {
    const copies = [table('first', open, '2025-12-20T17:00:00Z'), table('second', open, '2026-01-04T07:00:00Z')];
    const gaps = (unrecorded: number, ...held: ReturnType<typeof table>[]) => dataGaps(catalog(...copies, ...held), { second: profile(unrecorded) }, '2026-01-04T09:00:00Z').map((gap) => gap.fetch);
    expect(gaps(0)).toEqual([]);
    // The two copies straddle a year end, so a report may have been closed in either year's file.
    expect(gaps(12)).toEqual([{ dataset: 'GET_IT_DONE_OUTCOMES', year: 2025 }, { dataset: 'GET_IT_DONE_OUTCOMES', year: 2026 }]);
    const subset = (filter: string) => table('closed', closed2026, '2026-01-04T08:00:00Z', JSON.stringify({ derivedSubset: { filter } }));
    expect(gaps(12, subset("service_name = 'Encampment'"))).toHaveLength(2);
    expect(gaps(12, subset('service_request_id is one of 90000 values in imported copies'))).toEqual([{ dataset: 'GET_IT_DONE_OUTCOMES', year: 2025 }]);
    // A closed file retrieved before the later open copy cannot record what closed since.
    expect(gaps(12, { ...subset('service_request_id is one of 9 values'), retrievedAt: '2026-01-03T08:00:00Z' })).toHaveLength(2);
  });
});

describe('the budget behind a backlog', () => {
  it('asks for the operating budget only when a staff group is read as a department and no budget table is imported', () => {
    const latest = table('latest', open, '2026-10-04T00:00:00Z');
    const backlog = (group: string | null): Record<string, AutomaticProfile> => ({ latest: { fingerprint: 'f', checkedAt: 'now', results: [{ query: query('service_backlog'), result: response(['category', 'staff_group'], [['Pothole', group]]) }] } });
    const keys = (schema: AnalyticalSchema, group: string | null) => dataGaps(schema, backlog(group), '2026-10-04T12:00:00Z').map((gap) => gap.key);
    expect(keys(catalog(latest), 'TSW')).toEqual(['budget:latest']);
    expect(keys(catalog(latest), 'Parking')).toEqual([]);
    expect(keys(catalog(latest), null)).toEqual([]);
    const withBudget = catalog(latest);
    const budget = base.datasets.find((dataset) => dataset.snapshotId === 'budget')!;
    expect(keys({ ...withBudget, datasets: [...withBudget.datasets, budget], columns: [...withBudget.columns, ...base.columns.filter((column) => column.tableName === budget.tableName)] }, 'TSW')).toEqual([]);
  });
});

describe('fetching a missing table', () => {
  const upkeep = (autoRefresh: boolean): DataMaintenanceStatus => ({ policy: { enabled: true, autoImport: true, autoRefresh, refreshHours: 24, retentionMode: 'ORIGINALS', retainedFileBudgetBytes: 10 * 1024 ** 3 }, effectiveAt: '2026-09-14T00:00:00Z', retainedFileBytes: 0, referencedDocuments: 0, reclaimedFileBytes: 0, events: [] });
  const schema = catalog(table('first', open, '2026-09-01T17:00:00Z'));
  const controllers: DiscoveryController[] = [];
  function start(fetch: DiscoveryTransport['fetch']) {
    const controller = new DiscoveryController({ schema: async () => schema, candidates: async () => [], patterns: async () => [],
      query: async () => response(['column_name', 'category'], [['status', 'Pothole']]), fetch });
    controllers.push(controller); controller.start();
    return controller;
  }
  beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(new Date('2026-09-14T00:00:00Z')); });
  afterEach(() => { controllers.splice(0).forEach((controller) => controller.stop()); vi.useRealTimers(); });

  it('waits for the owner unless upkeep rechecks sources, and fetches a gap once', async () => {
    const fetch = vi.fn(async () => ({}));
    const controller = start(fetch);
    await vi.advanceTimersByTimeAsync(20_000);
    const [gap] = controller.gaps();
    expect(gap.fetch).toEqual({ dataset: 'GET_IT_DONE_OPEN', year: null });
    controller.setMaintenance(upkeep(false));
    await vi.advanceTimersByTimeAsync(20_000);
    expect(fetch).not.toHaveBeenCalled();
    controller.fill(gap.key);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(controller.snapshot().gapStatus[gap.key]).toBe('fetched');
    controller.setMaintenance(upkeep(true));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it('fetches automatically under upkeep and does not hammer a failing publisher', async () => {
    const fetch = vi.fn(async () => { throw new Error('offline'); });
    const controller = start(fetch);
    controller.setMaintenance(upkeep(true));
    await vi.advanceTimersByTimeAsync(120_000);
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(controller.snapshot().gapStatus[controller.gaps()[0].key]).toBe('Error: offline');
    await vi.advanceTimersByTimeAsync(21_600_000);
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it('records a fetch that finishes after a restart instead of downloading the file twice', async () => {
    let finish: () => void = () => {};
    const fetch = vi.fn(() => new Promise<void>((resolve) => { finish = resolve; }));
    const controller = start(fetch);
    controller.setMaintenance(upkeep(true));
    await vi.advanceTimersByTimeAsync(20_000);
    const key = controller.gaps()[0].key;
    expect(controller.snapshot().gapStatus[key]).toBe('fetching');
    controller.stop(); controller.start();
    await vi.advanceTimersByTimeAsync(20_000);
    finish();
    await vi.advanceTimersByTimeAsync(20_000);
    expect(controller.snapshot().gapStatus[key]).toBe('fetched');
    expect(fetch).toHaveBeenCalledTimes(1);
  });
});

describe('storing leads as findings', () => {
  const controllers: DiscoveryController[] = [];
  beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(new Date('2026-09-14T00:00:00Z')); });
  afterEach(() => { controllers.splice(0).forEach((controller) => controller.stop()); vi.useRealTimers(); });

  it('sends a changed set once, and calls it complete only when every table was checked without error', async () => {
    const backlog = response(['category', 'open_reports', 'open_over_one_year', 'earliest_opened', 'all_open_reports', 'all_open_over_one_year'], [['Pothole', 40, 30, '2024-01-01', 40, 30]]);
    let failed = false;
    const record = vi.fn(async () => ({}));
    const controller = new DiscoveryController({ schema: async () => catalog(table('first', open, '2026-09-13T00:00:00Z'), table('second', closed2026, '2026-09-13T00:00:00Z')),
      candidates: async () => [], patterns: async () => [], record,
      query: async (sql) => {
        if (!sql.includes('service_backlog AS (')) return response(['column_name'], []);
        if (sql.includes('analysis_second') && !failed) { failed = true; throw new Error('Temporary query failure'); }
        return backlog;
      } });
    controllers.push(controller); controller.start();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(record).toHaveBeenCalledTimes(1);
    expect(record).toHaveBeenLastCalledWith([expect.objectContaining({ key: 'first:service_backlog', snapshotId: 'first', title: '30 of 40 open reports have been open more than a year' })], false);
    await vi.advanceTimersByTimeAsync(310_000);
    expect(record).toHaveBeenCalledTimes(2);
    expect(record).toHaveBeenLastCalledWith([expect.objectContaining({ snapshotId: 'first' }), expect.objectContaining({ snapshotId: 'second' })], true);
  });
});
