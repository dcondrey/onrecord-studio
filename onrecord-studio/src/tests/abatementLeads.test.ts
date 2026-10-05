import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/abatement-leads.json';
import service from '../../src-tauri/tests/fixtures/service-accountability.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import { abatementLeads, abatementQueries } from '../utils/abatementLeads';
import { storedLeads } from '../utils/automaticDiscovery';

const schema = fixture.schema as AnalyticalSchema;
const [posted, completed] = schema.datasets;
const query = fixture.queries[0];
const result = (values: Record<string, unknown>): SqlQueryResult => ({ columns: Object.keys(values), rows: [Object.values(values)] as SqlQueryResult['rows'], rowCount: 1, elapsedMs: 1, truncated: false, provenance: [] });
const hours = (paired: number, extra: Record<string, unknown> = {}) => result({ posted_notices: 11090, completed_abatements: 14000, notices_not_matchable: 31, paired_places: paired, completed_before_posted: 0,
  under_3_hours: 22, under_24_hours: 101, from_24_to_72_hours: 1811, over_72_hours: 144, median_hours: 29.3, abatements_in_notice_period: 11104, abatements_without_notice_in_log: 0, first_posted: '2023-08-01', last_posted: '2025-07-28', ...extra });

describe('leads from released abatement logs', () => {
  it('reads a notice log only beside a completion log, and never a service-report table', () => {
    expect(abatementQueries(schema, posted).map((item) => item.id)).toEqual(['abatement_notice_hours']);
    expect(abatementQueries(schema, completed)).toEqual([]);
    expect(abatementQueries({ ...schema, datasets: [posted] }, posted)).toEqual([]);
    const reports = service.schema as AnalyticalSchema;
    expect(abatementQueries(reports, reports.datasets[0])).toEqual([]);
  });

  it('states hours only with 30 paired places, and absent counts as nothing rather than zero', () => {
    expect(abatementLeads([{ query, result: hours(29) }])).toEqual([]);
    const [lead] = abatementLeads([{ query, result: hours(30) }]);
    expect(lead.title).toBe('101 of 30 abatements matched to a single posted notice were completed less than 24 hours after posting');
    expect(lead.detail).toContain('Completed within 3 hours of posting: 22. Within 24 hours: 101. From 24 to 72 hours: 1,811. After 72 hours or more: 144. Median 29.3 hours.');
    expect(lead.detail).not.toContain('before the notice was posted');
    expect(lead.detail).not.toContain('no notice in that log');
    expect(lead.detail).toContain('24 and 72 hours are yardsticks, not the agency\'s stated requirement.');
    const full = abatementLeads([{ query, result: hours(2067, { completed_before_posted: 11, abatements_without_notice_in_log: 214 }) }])[0].detail;
    expect(full).toContain('11 are recorded completed before the notice was posted. 214 of 11,104 abatements dated within the notice log\'s period are at a place with no notice in that log.');
  });

  it('stores the lead under the abatement family', () => {
    const stored = storedLeads(schema, { posted: { fingerprint: 'f', checkedAt: 'now', results: [{ query, result: hours(2067) }] } });
    expect(stored.leads).toEqual([expect.objectContaining({ family: 'ABATEMENT_LOGS', key: 'posted:abatement_notice_hours' })]);
  });
});
