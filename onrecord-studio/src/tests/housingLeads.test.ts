import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/housing-leads.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import { storedLeads } from '../utils/automaticDiscovery';
import { housingLeads, housingQueries, issuedApprovals } from '../utils/housingLeads';

const schema = fixture.schema as AnalyticalSchema;
const table = (id: string) => schema.datasets.find((item) => item.snapshotId === id)!;
const [query] = fixture.queries;
const result = (rows: Record<string, unknown>[], truncated = false): SqlQueryResult => {
  const columns = [...new Set(rows.flatMap((row) => Object.keys(row)))];
  return { columns, rowCount: rows.length, elapsedMs: 1, truncated, provenance: [], rows: rows.map((row) => columns.map((name) => row[name] ?? null)) };
};
/** Most jobs on a year's permits record 0 in every tier; `unit` of them record a unit. */
const year = (issued: number, unit: number, change: Record<string, unknown> = {}) => ({ issued_year: issued, jobs: unit + 105, tiered_jobs: unit + 100, unit_jobs: unit, multi_permit_jobs: 0,
  extremely_low: 1, very_low: 2, low: 7, moderate: 10, above_moderate: 20, bonus: 15, adu: 9, no_job_permits: 0, ...change });
const units = (...rows: Record<string, unknown>[]) => [{ query, result: result(rows) }];

describe('leads from issued development approvals', () => {
  it('reads only the latest-retrieved copy of the issued files, never the created or closed files that share their header', () => {
    expect(table('approvals_issued_2025').sourceUri).toBe(issuedApprovals(2025));
    expect(housingQueries(schema, table('approvals_issued_2025'))).toEqual(fixture.queries);
    for (const id of ['approvals_issued_2024', 'approvals_prior_2025', 'approvals_created_2025', 'approvals_closed_2025']) expect(housingQueries(schema, table(id))).toEqual([]);
  });

  it('describes a year at 30 jobs that record a unit, not 29, however many jobs record 0 in every tier', () => {
    expect(housingLeads(units(year(2025, 29)), 2026)).toEqual([]);
    for (const unit of [30, 31]) {
      const [lead] = housingLeads(units(year(2025, unit)), 2026);
      expect(lead.detail).toContain(`Of ${unit + 105} jobs, ${unit} record a unit in an income tier, 100 record no unit above 0 in any tier, and 5 have no tier recorded.`);
      expect(lead.title).toBe('Building permits issued in 2025 carry 3 units for extremely-low and very-low income households, of 40 units with an income tier');
      expect(lead.key).toBe('housing_units_by_tier:2025');
    }
  });

  it('says when a tier total is net of units a job removes', () => {
    expect(housingLeads(units(year(2025, 30)), 2026)[0].detail).not.toContain('negative');
    expect(housingLeads(units(year(2025, 30, { removal_jobs: 2 })), 2026)[0].detail).toContain('2 jobs record a negative figure in an income tier (units removed); the tier totals are net of them.');
  });

  it('marks the current year as so far', () => {
    expect(housingLeads(units(year(2026, 30)), 2026)[0].title).toBe('Building permits issued in 2026 so far carry 3 units for extremely-low and very-low income households, of 40 units with an income tier');
  });

  it('says a tier no job recorded is not recorded, never 0 units', () => {
    const [lead] = housingLeads(units(year(2025, 30, { extremely_low: null, very_low: null })), 2026);
    expect(lead.title).toBe('Building permits issued in 2025 record no extremely-low or very-low income tier, of 37 units with an income tier');
    expect(lead.detail).toContain('Units by income tier: extremely low not recorded, very low not recorded, low 7, moderate 10, above moderate 20.');
    expect(lead.title).not.toContain('0 units');
    expect(housingLeads(units(year(2025, 30, { extremely_low: null })), 2026)[0].title)
      .toBe('Building permits issued in 2025 carry 2 units for very-low income households (no extremely-low tier recorded), of 39 units with an income tier');
  });

  it('keeps density-bonus units and ADUs out of the tier total', () => {
    const [lead] = housingLeads(units(year(2025, 30, { bonus: 1000, adu: 2000 })), 2026);
    expect(lead.title).toContain('of 40 units with an income tier');
    expect(lead.detail).toContain('density-bonus units 1,000, ADUs 2,000.');
    expect(housingLeads(units(year(2025, 30, { bonus: null, adu: null })), 2026)[0].detail).toContain('density-bonus units not recorded, ADUs not recorded.');
  });

  it('sets two qualifying years side by side without a trend', () => {
    const leads = housingLeads(units(year(2023, 40, { extremely_low: null, very_low: null }), year(2024, 30, { extremely_low: null, very_low: 5 }), year(2025, 30), year(2026, 10)), 2026);
    expect(leads.map((lead) => lead.key)).toEqual(['housing_units_by_tier:2023', 'housing_units_by_tier:2024', 'housing_units_by_tier:2025']);
    expect(leads[2].detail).toContain('Extremely-low and very-low income units on building permits, by issue year: 2023: not recorded; 2024: 5 (no extremely-low tier recorded); 2025: 3.');
  });

  it('gives no lead from a result cut short by the row limit', () => {
    expect(housingLeads([{ query, result: result([year(2025, 30)], true) }], 2026)).toEqual([]);
  });

  it('stores the lead under its own family, citing every issued file read', () => {
    const stored = storedLeads(schema, { approvals_issued_2025: { fingerprint: 'f', checkedAt: 'now', results: units(year(2025, 30)) } });
    expect(stored.leads).toEqual([
      expect.objectContaining({ family: 'HOUSING_APPROVALS', key: 'approvals_issued_2025:housing_units_by_tier:2025', snapshotIds: ['approvals_issued_2025', 'approvals_issued_2024'] }),
    ]);
  });
});
