import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/automatic-discovery.json';
import accountability from '../../src-tauri/tests/fixtures/service-accountability.json';
import type { AnalyticalSchema, DataMaintenanceStatus, SqlQueryResult } from '../types';
import { automaticFollowups, automaticProfileQuery, metadataObservations, profileObservations, rankObservations, snapshotFingerprint } from '../utils/automaticDiscovery';
import { DiscoveryController, type DiscoveryTransport } from '../utils/discoveryController';

const schema = fixture.schema as AnalyticalSchema;
const quality = fixture.profile as SqlQueryResult;
const upkeep: DataMaintenanceStatus = { policy: { enabled: true, autoImport: true, autoRefresh: true, refreshHours: 24, retentionMode: 'ORIGINALS', retainedFileBudgetBytes: 10 * 1024 ** 3 }, effectiveAt: '2026-09-14T00:00:00Z', retainedFileBytes: 0, referencedDocuments: 0, reclaimedFileBytes: 0, events: [] };
const response = (columns: string[], rows: SqlQueryResult['rows']): SqlQueryResult => ({ columns, rows, rowCount: rows.length, elapsedMs: 1, truncated: false, provenance: [] });
function transport(catalog: AnalyticalSchema = schema) {
  return { schema: vi.fn(async () => catalog), candidates: vi.fn(async () => []), patterns: vi.fn(async () => []), query: vi.fn<DiscoveryTransport['query']>(async () => quality) } satisfies DiscoveryTransport;
}
const controllers: DiscoveryController[] = [];
function start(bridge: DiscoveryTransport, visible?: () => boolean) { const controller = new DiscoveryController(bridge, visible); controllers.push(controller); controller.start(); return controller; }
beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(new Date('2026-09-14T00:00:00Z')); });
afterEach(() => { controllers.splice(0).forEach((c) => c.stop()); vi.useRealTimers(); });

describe('automatic discovery evidence', () => {
  it('publishes a supported lead before an unrelated table finishes and only clears stale leads after all checks finish', async () => {
    const base = accountability.schema as AnalyticalSchema;
    const first = base.datasets[0];
    const second = { ...first, snapshotId: 'slow', tableName: 'analysis_slow' };
    const catalog = { ...base, datasets: [first, second], columns: [
      ...base.columns.filter((column) => column.tableName === first.tableName),
      ...base.columns.filter((column) => column.tableName === first.tableName).map((column) => ({ ...column, tableName: second.tableName })),
    ] };
    const backlogSql = automaticFollowups(catalog, first, response([], [])).find((query) => query.id === 'service_backlog')!.sql;
    let release!: (result: SqlQueryResult) => void;
    const slow = new Promise<SqlQueryResult>((resolve) => { release = resolve; });
    const record = vi.fn<NonNullable<DiscoveryTransport['record']>>(async () => {});
    const bridge = { ...transport(catalog), record };
    bridge.query.mockImplementation(async (sql: string) => {
      if (sql.includes('"analysis_slow"')) return slow;
      if (sql === backlogSql) return response(
        ['category', 'open_reports', 'median_age_days', 'open_over_one_year', 'oldest_age_days', 'earliest_opened', 'all_open_reports', 'all_open_over_one_year'],
        [['Sidewalk Repair Issue', 14690, 1091, 11929, 3767, '2016-05-22', 83032, 11929]],
      );
      return response([], []);
    });
    start(bridge);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(record).toHaveBeenCalledOnce();
    expect(record.mock.calls[0]).toEqual([expect.arrayContaining([expect.objectContaining({ title: '11,929 of 83,032 open reports have been open more than a year' })]), false]);
    release(response([], []));
    await vi.advanceTimersByTimeAsync(20_000);
    expect(record).toHaveBeenLastCalledWith(expect.any(Array), true);
  });
  it('shares native integration queries and never guesses a sum or a rate', () => {
    const queries = [automaticProfileQuery(schema, schema.datasets[0]), ...automaticFollowups(schema, schema.datasets[0], quality)];
    expect(queries).toEqual(fixture.queries);
    expect(queries.map((q) => q.id)).toEqual(['quality', 'coverage', 'groups', 'changes']);
    expect(queries.filter((q) => q.id !== 'quality').map((q) => q.sql).join('\n')).not.toContain('SUM("amount")');
  });
  it('produces evidence-backed observations, distinguishes missing periods, and handles old diagnostics', () => {
    const dataset = schema.datasets[0];
    const queries = fixture.queries;
    const results = [
      { query: queries[0], result: response(['column_name', 'source_rows', 'missing_rows'], [['amount', 100, 25]]) },
      { query: queries[1], result: response(['first_period', 'last_period', 'unusable_period_rows'], [[22, 25, 1]]) },
      { query: queries[2], result: response(['group_value', 'source_rows', 'record_share_percent'], [['Vista', 75, 75]]) },
      { query: queries[3], result: response(['comparison_status', 'change', 'change_percent'], [['Missing intervening periods', null, null]]) },
    ];
    const findings = profileObservations(schema, dataset, { fingerprint: snapshotFingerprint(schema, dataset), checkedAt: 'now', results });
    expect(findings.some((o) => o.id.includes(':missing:'))).toBe(false);
    expect(findings.some((o) => o.title === 'Latest observed year code: 25')).toBe(true);
    expect(findings.some((o) => o.id.endsWith(':gaps'))).toBe(true);
    expect(findings.some((o) => o.id.endsWith(':change'))).toBe(false);
    for (const validationSummary of ['null', '[]', 'broken']) expect(() => metadataObservations({ ...dataset, validationSummary })).not.toThrow();
    const repaired = metadataObservations({ ...dataset, validationSummary: '{"warnings":["Format repair: UTF-16 decoded"]}' });
    expect(repaired.some((o) => o.title.includes('repaired'))).toBe(true);
    const ranked = rankObservations([...findings, { ...findings[0], id: 'other', snapshotId: 'other', priority: 40 }], 3);
    expect(ranked.some((o) => o.snapshotId === 'other')).toBe(true);
  });
});

describe('automatic discovery lifecycle', () => {
  it('starts profiling without interaction, caches results, and coalesces refreshes', async () => {
    const bridge = transport(); const controller = start(bridge);
    await vi.advanceTimersByTimeAsync(1600);
    expect(controller.snapshot().profiles.fixture.results).toHaveLength(4);
    expect(bridge.query).toHaveBeenCalledTimes(4);
    for (let i = 0; i < 8; i += 1) controller.refresh();
    await vi.advanceTimersByTimeAsync(1600);
    expect(bridge.schema).toHaveBeenCalledTimes(2);
    expect(bridge.query).toHaveBeenCalledTimes(4);
  });
  it('isolates failures, profiles other snapshots, and retries failed checks after backoff', async () => {
    const second = { ...schema.datasets[0], snapshotId: 'second', tableName: 'analysis_second' };
    const bridge = transport({ ...schema, datasets: [...schema.datasets, second], columns: [...schema.columns, ...schema.columns.map((c) => ({ ...c, tableName: 'analysis_second' }))] });
    bridge.candidates.mockRejectedValue(new Error('Catalog unavailable'));
    bridge.query.mockRejectedValueOnce(new Error('Temporary query failure'));
    const controller = start(bridge);
    await vi.advanceTimersByTimeAsync(2500);
    expect(controller.snapshot().profiles.fixture.results[0].error).toContain('Temporary');
    expect(controller.snapshot().profiles.second.results).toHaveLength(4);
    expect(controller.snapshot().errors.candidates).toContain('Catalog unavailable');
    await vi.advanceTimersByTimeAsync(310_000);
    expect(controller.snapshot().profiles.fixture.results).toHaveLength(4);
    expect(controller.snapshot().profiles.fixture.results.every((entry) => !entry.error)).toBe(true);
  });
  it('profiles tables while a slow document-pattern request is still pending', async () => {
    const bridge = transport();
    bridge.patterns.mockImplementation(() => new Promise(() => {}));
    const controller = start(bridge); await vi.advanceTimersByTimeAsync(1600);
    expect(controller.snapshot().profiles.fixture.results).toHaveLength(4);
    controller.refresh(); await vi.advanceTimersByTimeAsync(1000);
    expect(bridge.patterns).toHaveBeenCalledOnce();
  });
  it('does not let a table without usable columns starve later snapshots', async () => {
    const broken = { ...schema.datasets[0], snapshotId: 'broken', tableName: 'analysis_broken' };
    const controller = start(transport({ ...schema, datasets: [broken, ...schema.datasets] }));
    await vi.advanceTimersByTimeAsync(2000);
    expect(controller.snapshot().errors['profile:broken']).toBeTruthy();
    expect(controller.snapshot().profiles.fixture.results).toHaveLength(4);
  });
  it('invalidates modified snapshots and removes observations when their snapshot disappears', async () => {
    const bridge = transport(); const controller = start(bridge);
    await vi.advanceTimersByTimeAsync(1600);
    bridge.schema.mockResolvedValue({ ...schema, datasets: [{ ...schema.datasets[0], importedRows: 99 }] });
    controller.refresh(); await vi.advanceTimersByTimeAsync(1600);
    expect(bridge.query).toHaveBeenCalledTimes(8);
    bridge.schema.mockResolvedValue({ datasets: [], columns: [], templates: [] });
    controller.refresh(); await vi.advanceTimersByTimeAsync(600);
    expect(controller.snapshot().profiles).toEqual({});
  });
  it('pauses profiling when hidden or held, but allows background upkeep', async () => {
    let visible = false;
    const bridge = { ...transport(), maintain: vi.fn(async () => upkeep) };
    const controller = start(bridge, () => visible);
    await vi.advanceTimersByTimeAsync(7000);
    expect(bridge.query).not.toHaveBeenCalled(); expect(bridge.maintain).toHaveBeenCalledOnce();
    visible = true;
    const release = controller.hold(); controller.refresh();
    await vi.advanceTimersByTimeAsync(2000); expect(bridge.query).not.toHaveBeenCalled();
    release(); release(); await vi.advanceTimersByTimeAsync(1600);
    expect(bridge.query).toHaveBeenCalledTimes(4);
  });
  it('leaves a reserved table to its reserver while fetching every other missing table', async () => {
    // An open-reports copy with encampment reports lacks the stop records and the shelter inventory.
    const base = accountability.schema as AnalyticalSchema;
    const open = { ...base.datasets[0], snapshotId: 'latest', tableName: 'analysis_latest', sourceUri: 'https://seshat.datasd.org/get_it_done_reports/get_it_done_requests_open_datasd.csv', retrievedAt: '2026-09-14T00:00:00Z' };
    const catalog = { ...base, datasets: [open], columns: base.columns.filter((c) => c.tableName === 'analysis_fixture').map((c) => ({ ...c, tableName: 'analysis_latest' })) };
    const fetch = vi.fn(async () => ({}));
    const bridge = { ...transport(catalog), query: vi.fn(async () => response(['column_name', 'category'], [['status', 'Encampment']])), fetch };
    const controller = start(bridge);
    const stops = { dataset: 'RIPA_STOPS' as const, year: null };
    const first = controller.reserve([stops]);
    const second = controller.reserve([stops]);
    controller.setMaintenance(upkeep);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetch).toHaveBeenCalledWith({ dataset: 'SHELTER_INVENTORY', year: null });
    first(); first();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetch).not.toHaveBeenCalledWith(stops);
    second();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetch).toHaveBeenCalledWith(stops);
  });
  it('discards an in-flight result after teardown and completes safely after restart', async () => {
    let resolve!: (result: SqlQueryResult) => void;
    const bridge = transport(); bridge.query.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    const controller = start(bridge);
    await vi.advanceTimersByTimeAsync(300); controller.stop(); controller.start();
    resolve(quality); await vi.advanceTimersByTimeAsync(2000);
    expect(controller.snapshot().profiles.fixture.results).toHaveLength(4);
    expect(bridge.query).toHaveBeenCalledTimes(5);
  });
  it('recovers upkeep errors without repeatedly importing on every refresh', async () => {
    const bridge = { ...transport(), maintain: vi.fn(async () => upkeep) };
    bridge.maintain.mockRejectedValueOnce(new Error('Temporary upkeep error'));
    const controller = start(bridge); await vi.advanceTimersByTimeAsync(7000);
    expect(controller.snapshot().errors.maintenance).toContain('Temporary');
    controller.refresh(); await vi.advanceTimersByTimeAsync(2000);
    expect(bridge.maintain).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(65_000);
    expect(controller.snapshot().errors.maintenance).toBeUndefined();
    expect(controller.snapshot().maintenance?.policy.retentionMode).toBe('ORIGINALS');
  });
});
