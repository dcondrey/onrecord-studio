import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/call-leads.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import { storedLeads } from '../utils/automaticDiscovery';
import { callLeads, callQueries } from '../utils/callLeads';

const schema = fixture.schema as AnalyticalSchema;
const table = (snapshotId: string) => schema.datasets.find((dataset) => dataset.snapshotId === snapshotId)!;
const query = (id: string) => fixture.queries.find((item) => item.id === id)!;
const result = (rows: Record<string, unknown>[], truncated = false): SqlQueryResult => {
  const columns = [...new Set(rows.flatMap((row) => Object.keys(row)))];
  return { columns, rowCount: rows.length, elapsedMs: 1, truncated, provenance: [], rows: rows.map((row) => columns.map((name) => row[name] ?? null)) };
};
const ended = (type: string, sent: number, change: Record<string, unknown> = {}) => ({ call_year: 2025, call_type: type, calls: sent + 20, cancelled_duplicate: 20, blank: 5,
  arrest: 12, report: 8, no_report: 40, unfounded: 3, other: sent - 5 - 12 - 8 - 40 - 3, crisis_team: 5, other_codes: 'O (OTHER)', last_call: '2025-12-31 23:00:00', ...change });
const outcomes = (rows: Record<string, unknown>[], truncated = false) => ({ query: query('calls_outcomes'), result: result(rows, truncated) });
const beats = (citywide: number, neighborhood: string | null = 'Core-Columbia') => ({ query: query('calls_sleeper_beats'), result: result([
  { beat: '524', neighborhood, calls: 40, citywide_calls: citywide, no_beat: 2, first_year: 2025, last_year: 2026 },
  { beat: '521', neighborhood: 'East Village', calls: 30, citywide_calls: citywide, no_beat: 2, first_year: 2025, last_year: 2026 }]) });

describe('leads from police calls for service', () => {
  it('reads every year from the latest calls table only, the latest copy of each year, and needs a disposition', () => {
    expect(callQueries(schema, table('cfs_2026')).map((item) => item.id)).toEqual(['calls_outcomes', 'calls_sleeper_beats']);
    expect(callQueries(schema, table('cfs_2025'))).toEqual([]);
    expect(callQueries(schema, table('cfs_prior_2026'))).toEqual([]);
    expect(callQueries(schema, table('cfs_types'))).toEqual([]);
    const [{ sql }] = callQueries(schema, table('cfs_2026'));
    expect(sql).toContain('"analysis_cfs_2025"');
    expect(sql).not.toContain('analysis_cfs_prior_2026');
    const undisposed: AnalyticalSchema = { ...schema, columns: schema.columns.filter((column) => column.columnName !== 'disposition') };
    expect(callQueries(undisposed, table('cfs_2026'))).toEqual([]);
  });

  it('describes a year only with 100 SLEEPER calls not cancelled, and keeps blank dispositions apart from no report', () => {
    expect(callLeads([outcomes([ended('SLEEPER', 99)])], 2026)).toEqual([]);
    const [lead] = callLeads([outcomes([ended('SLEEPER', 100), ended('SLEEPER', 400, { call_year: 2024 })])], 2026);
    expect(lead.title).toBe('100 calls dispatched as SLEEPER in 2025: 12 ended in arrest, 5 with the Crisis Response Team, 40 with no report required');
    // Cancelled and duplicate calls are out of the count, and blanks out of the shares.
    expect(lead.detail).toContain('120 calls, 100 calls not cancelled; of the 95 with a disposition, 12 (12.6%) ended in arrest');
    expect(lead.detail).toContain('5 have no disposition recorded and 20 were cancelled or duplicates');
    expect(lead.detail).toContain('not that no service was given');
    expect(callLeads([outcomes([ended('SLEEPER', 101)])], 2026)[0].title).toMatch(/^101 calls dispatched as SLEEPER in 2025: /);
    expect(callLeads([outcomes([ended('SLEEPER', 100)])], 2025)[0].title).toMatch(/^100 calls dispatched as SLEEPER in 2025 so far: /);
  });

  it('describes another call type beside SLEEPER only at 100 calls not cancelled', () => {
    const detail = (welfare: number) => callLeads([outcomes([ended('SLEEPER', 100), ended('CW', welfare)])], 2026)[0].detail;
    expect(detail(99)).not.toContain('CW (check the welfare)');
    expect(detail(100)).toContain('CW (check the welfare) in 2025: 120 calls, 100 calls not cancelled');
  });

  it('gives no lead from a result cut short by the row limit', () => {
    expect(callLeads([outcomes([ended('SLEEPER', 500)], true), { ...beats(500), result: { ...beats(500).result, truncated: true } }], 2026)).toEqual([]);
  });

  it('names the beat with most SLEEPER calls, and only the beat code when the beat list does not name it', () => {
    expect(callLeads([beats(99)], 2026)).toEqual([]);
    expect(callLeads([beats(100)], 2026)[0].title).toBe('40 of 100 SLEEPER calls from 2025 to 2026 were made in beat 524 (Core-Columbia)');
    const [unnamed] = callLeads([beats(100, null)], 2026);
    expect(unnamed.title).toBe('40 of 100 SLEEPER calls from 2025 to 2026 were made in beat 524');
    expect(unnamed.detail).toContain('not in the imported beat list');
  });

  it('stores the calls leads under their own family, citing every year and lookup read', () => {
    const stored = storedLeads(schema, { cfs_2026: { fingerprint: 'f', checkedAt: 'now', results: [outcomes([ended('SLEEPER', 100)]), beats(100)] } });
    expect(stored.leads).toEqual([
      expect.objectContaining({ family: 'POLICE_CALLS', key: 'cfs_2026:calls_outcomes', snapshotIds: ['cfs_2026', 'cfs_2025', 'cfs_dispositions'] }),
      expect.objectContaining({ family: 'POLICE_CALLS', key: 'cfs_2026:calls_sleeper_beats', snapshotIds: ['cfs_2026', 'cfs_2025', 'cfs_beats'] }),
    ]);
  });
});
