import { describe, expect, it } from 'vitest';
import generic from '../../src-tauri/tests/fixtures/automatic-discovery.json';
import fixture from '../../src-tauri/tests/fixtures/service-accountability.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import { automaticFollowups, profileObservations, recurringObservations, snapshotFingerprint, type AutomaticProfile } from '../utils/automaticDiscovery';
import { serviceAccountabilityLeads, serviceAccountabilityQueries } from '../utils/serviceAccountability';

const schema = fixture.schema as AnalyticalSchema;
const dataset = schema.datasets[0];
const response = (columns: string[], rows: SqlQueryResult['rows']): SqlQueryResult => ({ columns, rows, rowCount: rows.length, elapsedMs: 1, truncated: false, provenance: [] });
const query = (id: string) => fixture.queries.find((item) => item.id === id)!;
const wait = response(['category', 'area', 'closed_reports', 'median_days', 'over_one_year', 'all_median_days', 'times_all_areas'], [['Pothole', '3', 30, 40, 0, 5, 8]]);

describe('service-report leads', () => {
  it('generates the queries the native test runs, and none for a table that is not service reports', () => {
    expect(serviceAccountabilityQueries(schema, dataset)).toEqual(fixture.queries);
    const profile = response(['column_name', 'distinct_values'], []);
    expect(automaticFollowups(schema, dataset, profile).map((item) => item.id)).toEqual(expect.arrayContaining(fixture.queries.map((item) => item.id)));
    const other = generic.schema as AnalyticalSchema;
    expect(serviceAccountabilityQueries(other, other.datasets[0])).toEqual([]);
  });

  it('states a backlog only from counted reports and keeps small counts off the feed', () => {
    const backlog = (stale: number) => ({ query: query('service_backlog'), result: response(
      ['category', 'open_reports', 'median_age_days', 'open_over_one_year', 'oldest_age_days', 'earliest_opened', 'all_open_reports', 'all_open_over_one_year'],
      [['Sidewalk Repair Issue', 14690, 1091, stale, 3767, '2016-05-22', 83032, stale]]) });
    const profile: AutomaticProfile = { fingerprint: snapshotFingerprint(schema, dataset), checkedAt: 'now', results: [backlog(11929)] };
    const [lead] = profileObservations(schema, dataset, profile);
    expect(lead.kind).toBe('lead');
    expect(lead.title).toBe('11,929 of 83,032 open reports have been open more than a year');
    expect(lead.detail).toContain('Sidewalk Repair Issue: 11,929 of 14,690 open reports are older than a year, the earliest filed 2016-05-22.');
    expect(serviceAccountabilityLeads([backlog(24)])).toEqual([]);
  });

  it('reports referrals only where they are a large share of a service', () => {
    const referred = (reports: number) => ({ query: query('service_referred'), result: response(
      ['category', 'reports', 'referred_reports', 'referred_percent', 'all_reports', 'all_referred_reports'], [['Other', reports, 150, 0, 5000, 400]]) });
    expect(serviceAccountabilityLeads([referred(600)])[0].title).toBe('400 of 5,000 reports were referred elsewhere, with no outcome recorded here');
    expect(serviceAccountabilityLeads([referred(601)])).toEqual([]);
  });

  it('compares the backlog of one source across retrieval dates and never across sources', () => {
    const copy = (snapshotId: string, retrievedAt: string, sourceUri: string, stale: number) => ({ dataset: { ...dataset, snapshotId, retrievedAt, sourceUri }, profile: { fingerprint: snapshotId, checkedAt: 'now', results: [{ query: query('service_backlog'), result: response(
      ['category', 'open_reports', 'open_over_one_year', 'all_open_reports', 'all_open_over_one_year'], [['Sidewalk Repair Issue', 14000 + stale, stale, 80000 + stale, stale + 30000]]) }] } });
    const leads = (copies: ReturnType<typeof copy>[]) => recurringObservations({ ...schema, datasets: copies.map((item) => item.dataset) }, Object.fromEntries(copies.map((item) => [item.dataset.snapshotId, item.profile])));
    const [lead] = leads([copy('october', '2026-10-04T07:00:00Z', 'https://example.test/open.csv', 12500), copy('september', '2026-09-14T07:00:00Z', 'https://example.test/open.csv', 11929)]);
    expect(lead.title).toBe('Open reports older than a year rose from 41,929 to 42,500 between 2026-09-14 and 2026-10-04');
    expect(lead.detail).toContain('Sidewalk Repair Issue: 11,929 to 12,500.');
    expect(lead.snapshotId).toBe('october');
    expect(leads([copy('october', '2026-10-04T07:00:00Z', 'https://example.test/open.csv', 12500), copy('september', '2026-09-14T07:00:00Z', 'https://example.test/other.csv', 11929)])).toEqual([]);
  });

  it('says what became of reports that left the open file only when a table could record it', () => {
    const columns = ['category', 'left_reports', 'left_over_one_year', 'closed_on_record', 'referred_on_record', 'no_outcome_record', 'arrived_reports',
      'all_left_reports', 'all_left_over_one_year', 'all_closed_on_record', 'all_referred_on_record', 'all_no_outcome_record', 'all_arrived_reports', 'outcome_tables'];
    const backlog = { query: query('service_backlog'), result: response(['category', 'open_reports', 'open_over_one_year', 'all_open_reports', 'all_open_over_one_year'], [['ROW Maintenance', 10564, 6166, 83032, 42322]]) };
    const leads = (movement: SqlQueryResult['rows'][number]) => {
      const copies = [{ ...dataset, snapshotId: 'september', retrievedAt: '2026-09-14T07:00:00Z' }, { ...dataset, snapshotId: 'october', retrievedAt: '2026-10-04T07:00:00Z' }];
      return recurringObservations({ ...schema, datasets: copies }, {
        september: { fingerprint: 'september', checkedAt: 'now', results: [backlog] },
        october: { fingerprint: 'october', checkedAt: 'now', results: [backlog, { query: query('service_movement'), result: response(columns, [movement]) }] },
      });
    };
    const [searched] = leads(['ROW Maintenance', 385, 170, 374, 11, 0, 376, 8016, 657, 7832, 184, 0, 8939, 1]);
    expect(searched.title).toBe('8,016 reports left the open file and 8,939 new ones arrived between 2026-09-14 and 2026-10-04');
    expect(searched.detail).toContain('7,832 have a close date and 184 were referred in the imported tables; 0 have no close or referral record in any of them.');
    expect(searched.detail).toContain('ROW Maintenance: 385 left (170 open more than a year), 376 arrived.');
    const [unsearched] = leads(['ROW Maintenance', 385, 170, 0, 0, 385, 376, 8016, 657, 0, 0, 8016, 8939, 0]);
    expect(unsearched.detail).toContain('whether they were closed or referred is not in it');
    expect(unsearched.detail).not.toContain('have no close or referral record');
    const [missing] = leads(['ROW Maintenance', 385, 170, 370, 11, 4, 376, 8016, 657, 7828, 184, 4, 8939, 1]);
    expect(missing.question).toBe('What became of the 4 reports that left the open file with no close or referral record?');
  });

  it('calls a slower area recurring at three imported tables, not two', () => {
    const tables = ['2016', '2017', '2018'].map((year) => ({ ...dataset, snapshotId: year, displayName: `Closed ${year}`, tableName: `analysis_${year}` }));
    const catalog = { ...schema, datasets: tables };
    const profiles = Object.fromEntries(tables.map((table) => [table.snapshotId, { fingerprint: table.snapshotId, checkedAt: 'now', results: [{ query: query('service_wait_area:council_district'), result: wait }] }]));
    const [lead] = recurringObservations(catalog, profiles);
    expect(lead.title).toBe('Pothole reports in council district 3 closed slower than the all-area median in 3 imported tables');
    expect(lead.detail).toContain('Closed 2017: median 40 days against 5 (30 reports).');
    delete profiles['2018'];
    expect(recurringObservations(catalog, profiles)).toEqual([]);
  });
});
