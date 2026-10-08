import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/capacity-leads.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import { storedLeads } from '../utils/automaticDiscovery';
import { capacityLeads, capacityQueries, closedReports, encampmentReportCoverage } from '../utils/capacityLeads';

const schema = fixture.schema as AnalyticalSchema;
const [inventory] = schema.datasets;
const [query] = fixture.queries;
const columns = ['fiscal_year', 'adopted_rows', 'department_rows', 'departments', 'adopted_expense', 'unreadable_amounts', 'encampment_reports', 'undated_reports',
  'shelter_projects', 'year_round_beds', 'total_beds', 'projects_without_year_round', 'projects_without_total'];
type Row = { year: number; expense?: number | null; departmentRows?: number | null; reports?: number | null; beds?: number };
const result = (rows: Row[]): SqlQueryResult => ({ columns, rowCount: rows.length, elapsedMs: 1, truncated: false, provenance: [],
  rows: rows.map(({ year, expense = 100, departmentRows = 5, reports = 1000, beds = 500 }) =>
    [year, departmentRows === null ? null : 900, departmentRows, expense === null ? null : 'Homelessness Strategies & Solutions', expense, departmentRows === null ? null : 0, reports, 0, 4, beds, beds + 10, 0, 0]) });
const leads = (rows: Row[]) => capacityLeads([{ query, result: result(rows) }]);
const without = (...snapshotIds: string[]): AnalyticalSchema => ({ ...schema, datasets: schema.datasets.filter((dataset) => !snapshotIds.includes(dataset.snapshotId)) });
const replaced = (snapshotId: string, change: Partial<AnalyticalSchema['datasets'][number]>): AnalyticalSchema =>
  ({ ...schema, datasets: schema.datasets.map((dataset) => dataset.snapshotId === snapshotId ? { ...dataset, ...change } : dataset) });

describe('budget, Encampment reports and shelter beds side by side', () => {
  it('counts a fiscal year of reports only when the open file and every closed file since it began are imported', () => {
    // Closed 2023 to 2026 and an open copy from October 2026: reports filed from 1 July 2023 on.
    expect(encampmentReportCoverage(schema)).toMatchObject({ fiscalYears: [2024, 2025, 2026], nextClosedYear: 2022 });
    expect(query.sql).toContain('IN (2024, 2025, 2026) THEN COALESCE(encampment_reports, 0) END');
    // Without the 2025 file a report filed in fiscal 2024, 2025 or 2026 and closed in 2025 is in no table.
    expect(encampmentReportCoverage(without('closed_2025'))).toMatchObject({ fiscalYears: [], nextClosedYear: 2025 });
    expect(capacityQueries(without('closed_2025'), inventory)).toEqual([]);
    // A current-year closed file older than the open copy misses what closed between the two.
    expect(encampmentReportCoverage(replaced('closed_2026', { retrievedAt: '2026-10-03T12:00:00Z' }))).toMatchObject({ fiscalYears: [], nextClosedYear: 2026 });
    // A past year's file was complete once that year ended.
    expect(encampmentReportCoverage(replaced('closed_2025', { retrievedAt: '2026-01-02T00:00:00Z' }))?.fiscalYears).toEqual([2024, 2025, 2026]);
    expect(encampmentReportCoverage(replaced('closed_2025', { retrievedAt: '2026-01-01T23:00:00Z' }))?.fiscalYears).toEqual([]);
    // The subset kept for reports seen open does not hold every Encampment report closed that year.
    const subset = (filter: string) => replaced('closed_2024', { validationSummary: JSON.stringify({ derivedSubset: { filter } }) });
    expect(encampmentReportCoverage(subset('service_request_id is one of 9 values'))?.fiscalYears).toEqual([2026]);
    expect(encampmentReportCoverage(subset("service_name = 'Encampment'"))?.fiscalYears).toEqual([2024, 2025, 2026]);
    // Fiscal 2026 ends 30 June 2026 local time; an open copy from 1 July UTC may not hold its last reports.
    const early = (retrievedAt: string) => ({ ...schema, datasets: schema.datasets.map((dataset) => dataset.snapshotId === 'open' || dataset.snapshotId === 'closed_2026' ? { ...dataset, retrievedAt } : dataset) });
    expect(encampmentReportCoverage(early('2026-07-01T23:00:00Z'))?.fiscalYears).toEqual([2024, 2025]);
    expect(encampmentReportCoverage(early('2026-07-02T00:00:00Z'))?.fiscalYears).toEqual([2024, 2025, 2026]);
    expect(closedReports(2025)).toBe(schema.datasets.find((dataset) => dataset.snapshotId === 'closed_2025')!.sourceUri);
  });

  it('needs the budget table and is raised once, from the latest inventory table', () => {
    expect(capacityQueries(without('budget'), inventory)).toEqual([]);
    expect(capacityQueries(schema, schema.datasets[1])).toEqual([]);
    const older = { ...inventory, snapshotId: 'older', tableName: 'analysis_older', retrievedAt: '2026-09-01T00:00:00Z' };
    const both = { ...schema, datasets: [...schema.datasets, older], columns: [...schema.columns, ...schema.columns.filter((column) => column.tableName === inventory.tableName).map((column) => ({ ...column, tableName: older.tableName }))] };
    expect(capacityQueries(both, older)).toEqual([]);
    expect(capacityQueries(both, inventory)).toHaveLength(1);
  });

  it('compares a year with 30 Encampment reports, not 29, and needs two such years', () => {
    const pair = (reports: number) => leads([{ year: 2024, reports }, { year: 2025, reports: 5000 }]);
    expect(pair(29)).toEqual([]);
    expect(pair(30)).toHaveLength(1);
    expect(pair(31)).toHaveLength(1);
    expect(leads([{ year: 2025, reports: 5000 }])).toEqual([]);
  });

  it('is raised when one series differs by a tenth between the first and latest year, whichever series it is', () => {
    const budget = (expense: number) => leads([{ year: 2024 }, { year: 2025, expense }]);
    expect(budget(109)).toEqual([]);
    expect(budget(110)).toHaveLength(1);
    expect(budget(111)).toHaveLength(1);
    expect(budget(91)).toEqual([]);
    expect(budget(90)).toHaveLength(1);
    expect(leads([{ year: 2024 }, { year: 2025, reports: 1099 }])).toEqual([]);
    expect(leads([{ year: 2024 }, { year: 2025, reports: 1100 }])).toHaveLength(1);
    expect(leads([{ year: 2024 }, { year: 2025, beds: 451 }])).toEqual([]);
    expect(leads([{ year: 2024 }, { year: 2025, beds: 450 }])).toHaveLength(1);
  });

  it('says a year has no such department, no adopted rows or no complete count, and never shows it as zero', () => {
    const [lead] = leads([{ year: 2023, departmentRows: 0, expense: null }, { year: 2024, expense: 44_007_454, reports: 69_987, beds: 2476 },
      { year: 2025, expense: 53_916_571, reports: 48_135, beds: 1970 }, { year: 2026, departmentRows: null, expense: null }, { year: 2027, reports: null }]);
    expect(lead.title).toBe('Adopted homelessness budget $44.0 million to $53.9 million; Encampment reports 69,987 to 48,135; year-round emergency shelter beds 2,476 to 1,970, fiscal 2024 to 2025');
    expect(lead.detail).toContain('Between FY 2024 and FY 2025: adopted operating expense of Homelessness Strategies & Solutions $44.0 million to $53.9 million (up 22.5%); Encampment reports filed 69,987 to 48,135 (down 31.2%); year-round emergency shelter beds 2,476 to 1,970 (down 20.4%).');
    expect(lead.detail).toContain('Not compared: FY 2023 (the budget table lists no department with "homeless" in its name, which is not an amount of zero); FY 2026 (the budget table has no rows marked adopted); FY 2027 (the imported files cannot hold every Encampment report filed, so none is counted).');
    expect(lead.detail).not.toMatch(/FY 2023: |\$0|FY 2027: /);
    expect(lead.detail).toContain('Nothing here shows why any series moved.');
    // The department's first year is the first compared, not a rise from nothing.
    expect(leads([{ year: 2023, departmentRows: 0, expense: null }, { year: 2024 }])).toEqual([]);
    // The figure draws the same years with each absence as a gap carrying its reason, never a 0.
    if (lead.figure?.kind !== 'series') throw new Error('the capacity lead draws a series');
    const budget = lead.figure.panels.find((panel) => panel.key === 'budget')!.values;
    expect(lead.figure.points.map((point) => point.label)).toEqual(['FY 2023', 'FY 2024', 'FY 2025', 'FY 2026', 'FY 2027']);
    expect(budget[0]).toEqual({ state: 'not-recorded', reason: 'no department with "homeless" in its name; not an amount of zero' });
    expect(budget[3]).toEqual({ state: 'not-recorded', reason: 'the budget table has no rows marked adopted' });
    expect(lead.figure.panels.find((panel) => panel.key === 'reports')!.values[4]).toEqual({ state: 'not-covered', reason: 'the imported files cannot hold every Encampment report filed' });
  });

  it('stores the lead with every table its SQL read, the inventory first', () => {
    const stored = storedLeads(schema, { shelter: { fingerprint: 'f', checkedAt: 'now', results: [{ query, result: result([{ year: 2024 }, { year: 2025, expense: 200 }]) }] } });
    expect(stored.leads).toEqual([expect.objectContaining({ family: 'CAPACITY_COMPARISON', key: 'shelter:capacity_comparison', snapshotId: 'shelter', sql: query.sql,
      snapshotIds: ['shelter', 'budget', 'closed_2023', 'closed_2024', 'closed_2025', 'closed_2026', 'open'] })]);
  });
});
