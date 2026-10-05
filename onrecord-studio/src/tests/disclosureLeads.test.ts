import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/disclosure-leads.json';
import service from '../../src-tauri/tests/fixtures/service-accountability.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import { storedLeads } from '../utils/automaticDiscovery';
import { disclosureLeads, disclosureQueries } from '../utils/disclosureLeads';

const schema = fixture.schema as AnalyticalSchema;
const [forms, officials] = fixture.queries;
const table = (columns: string[], rows: SqlQueryResult['rows']): SqlQueryResult => ({ columns, rows, rowCount: rows.length, elapsedMs: 1, truncated: false, provenance: [] });
const byOfficial = (all: number, unnamed: number | null, rows: [string | null, number][]) => table(
  ['official', 'reports', 'first_filed', 'last_filed', 'all_reports', 'officials_named', 'unnamed_reports'],
  rows.map(([official, reports]) => [official, reports, '2008-08-18', '2026-08-28', all, rows.filter(([name]) => name).length, unnamed]));
const byForm = table(['form', 'reports', 'undated', 'first_filed', 'last_filed'], [['FPPC 802', 960, 0, '2008-08-18', '2024-12-01'], ['FPPC 803', 278, 0, '2008-08-18', '2026-08-28'], ['OTHER', 6, 0, null, null]]);

describe('leads from a disclosure portal agency-report export', () => {
  it('reads only a table with the export\'s columns', () => {
    expect(disclosureQueries(schema, schema.datasets[0]).map((item) => item.id)).toEqual(['disclosure_reports_by_form', 'disclosure_behested_by_official']);
    const reports = service.schema as AnalyticalSchema;
    expect(disclosureQueries(reports, reports.datasets[0])).toEqual([]);
  });

  it('describes behested payments at 10 reports, not 9, and names an official at 3 reports, not 2', () => {
    expect(disclosureLeads([{ query: officials, result: byOfficial(9, null, [['Official A', 9]]) }])).toEqual([]);
    const [lead] = disclosureLeads([{ query: officials, result: byOfficial(10, null, [['Official A', 5], ['Official B', 3], ['Official C', 2]]) }]);
    expect(lead.title).toBe('10 behested payment reports filed 2008-08-18 to 2026-08-28 name 3 officials');
    expect(lead.detail).toContain('Reports by the official named: Official A 5; Official B 3.');
    expect(lead.detail).not.toContain('Official C');
  });

  it('counts reports naming no official apart, and says nothing about them when none was counted', () => {
    const detail = (unnamed: number | null) => disclosureLeads([{ query: forms, result: byForm }, { query: officials, result: byOfficial(278, unnamed, [['Official A', 200], [null, 78]]) }])[0].detail;
    expect(detail(78)).toContain('78 name no official.');
    expect(detail(null)).not.toContain('name no official');
    expect(detail(78)).toContain('The same export also holds 960 ticket distributions (Form 802).');
    expect(detail(78)).toContain('A count of reports says nothing about any decision the official made');
  });

  it('stores the lead under the disclosure family', () => {
    const stored = storedLeads(schema, { reports: { fingerprint: 'f', checkedAt: 'now', results: [{ query: officials, result: byOfficial(10, null, [['Official A', 10]]) }] } });
    expect(stored.leads).toEqual([expect.objectContaining({ family: 'DISCLOSURE_FILINGS', key: 'reports:disclosure_behested_by_official' })]);
  });
});
