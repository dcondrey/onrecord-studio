import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/charge-leads.json';
import service from '../../src-tauri/tests/fixtures/service-accountability.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import { storedLeads } from '../utils/automaticDiscovery';
import { chargeLeads, chargeQueries } from '../utils/chargeLeads';

const schema = fixture.schema as AnalyticalSchema;
const [counts, races] = fixture.queries;
const table = (columns: string[], rows: SqlQueryResult['rows']): SqlQueryResult => ({ columns, rows, rowCount: rows.length, elapsedMs: 1, truncated: false, provenance: [] });
const counted = (activities: number, cited: number, alone = 259) => table(
  ['activities', 'counted_activities', 'arrests', 'citations', 'arrests_on_these_alone', 'encampment', 'encroachment', 'lodging', 'urination', 'first_date', 'last_date'],
  [[activities, cited, 455, cited - 455, alone, 268, 1704, 194, 0, '2025-01-01', '2026-06-22']]);
const byRace = (allCounted: number, all: number) => table(['race', 'counted_activities', 'activities', 'all_counted_activities', 'all_activities'],
  [['WHITE', 971, 27842, allCounted, all], ['BLACK', 628, 12242, allCounted, all], ['LAOTIAN', 29, 46, allCounted, all]]);

describe('leads from a released arrest and citation log', () => {
  it('reads only a table with the log\'s columns, and compares race only when the log records it', () => {
    expect(chargeQueries(schema, schema.datasets[0]).map((item) => item.id)).toEqual(['charges_living_outdoors', 'charges_living_outdoors_race']);
    const unraced = { ...schema, columns: schema.columns.filter((column) => column.columnName !== 'race') };
    expect(chargeQueries(unraced, schema.datasets[0]).map((item) => item.id)).toEqual(['charges_living_outdoors']);
    const reports = service.schema as AnalyticalSchema;
    expect(chargeQueries(reports, reports.datasets[0])).toEqual([]);
  });

  it('states counts at 30 arrests and citations, not 29, and leaves out a section nobody was charged under', () => {
    expect(chargeLeads([{ query: counts, result: counted(82054, 29) }])).toEqual([]);
    expect(chargeLeads([{ query: counts, result: counted(82054, 30) }])).toHaveLength(1);
    const [lead] = chargeLeads([{ query: counts, result: counted(82054, 2349) }]);
    expect(lead.title).toBe('2,349 arrests and citations cite an encampment, encroachment, lodging or public urination section; 259 were arrests on those sections alone');
    expect(lead.detail).toContain('The file holds 82,054 arrests and citations, dated 2025-01-01 to 2026-06-22. 2,349 cite at least one of these sections.');
    expect(lead.detail).toContain('1,894 were citations and 455 were arrests; in 259 of the arrests no other section was charged.');
    expect(lead.detail).not.toContain('56.55');
    expect(lead.detail).not.toContain('Recorded race');
    expect(chargeLeads([{ query: counts, result: counted(82054, 2349, 0) }])[0].title).toBe('2,349 arrests and citations cite an encampment, encroachment, lodging or public urination section');
  });

  it('names a race only with 30 of the counted behind it, and never when the file holds nothing else to compare with', () => {
    const detail = (allCounted: number, all: number) => chargeLeads([{ query: counts, result: counted(all, allCounted) }, { query: races, result: byRace(allCounted, all) }])[0].detail;
    expect(detail(2349, 81892)).toContain('(blank entries set aside): White 41.3% and 34.0%; Black 26.7% and 14.9%. The file records the charge');
    expect(detail(2349, 81892)).toContain('is not a finding of unequal treatment.');
    expect(detail(1564, 1564)).not.toContain('Recorded race');
    expect(detail(1564, 1564)).not.toContain('unequal treatment');
  });

  it('stores the lead under the charge-log family', () => {
    const stored = storedLeads(schema, { charges: { fingerprint: 'f', checkedAt: 'now', results: [{ query: counts, result: counted(82054, 2349) }] } });
    expect(stored.leads).toEqual([expect.objectContaining({ family: 'CHARGE_LOGS', key: 'charges:charges_living_outdoors' })]);
  });
});
