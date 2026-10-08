import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/abatement-leads.json';
import service from '../../src-tauri/tests/fixtures/service-accountability.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import nearby from '../../src-tauri/tests/fixtures/abatement-reports-nearby.json';
import { abatementLeads, abatementQueries, reportsAroundAbatements } from '../utils/abatementLeads';
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
    // A completion log alone is still read for places abated on many days.
    expect(abatementQueries(schema, completed).map((item) => item.id)).toEqual(['abatement_repeat_places', 'abatement_daily']);
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

  it('bins the hours with under 3 as part of under 24, completions before posting outside the axis, and no figure from a cut-short result', () => {
    const [lead] = abatementLeads([{ query, result: hours(2067, { completed_before_posted: 11 }) }]);
    if (lead.figure?.kind !== 'bins') throw new Error('the notice lead draws bins');
    expect(lead.figure.bins.map((bin) => [bin.from, bin.to, bin.count])).toEqual([[0, 3, 22], [3, 24, 79], [24, 72, 1811], [72, null, 144]]);
    expect(lead.figure.before).toEqual({ key: 'before', label: 'completed before the notice was logged', count: 11 });
    const [cut] = abatementLeads([{ query, result: { ...hours(2067), truncated: true } }]);
    expect(cut.figure).toBeUndefined();
  });

  it('stores the lead under the abatement family, with its figure', () => {
    const stored = storedLeads(schema, { posted: { fingerprint: 'f', checkedAt: 'now', results: [{ query, result: hours(2067) }] } });
    expect(stored.leads).toEqual([expect.objectContaining({ family: 'ABATEMENT_LOGS', key: 'posted:abatement_notice_hours', figure: expect.objectContaining({ kind: 'bins' }) })]);
  });
});

describe('Encampment reports around abatements', () => {
  const around = nearby.schema as AnalyticalSchema;
  const [log, closed, open] = around.datasets;
  const nearbyQuery = nearby.queries.find((item) => item.id === 'abatement_reports_nearby')!;
  const repeatQuery = nearby.queries.find((item) => item.id === 'abatement_repeat_places')!;
  const counts = (placeDays: number, extra: Record<string, unknown> = {}) => result({ abatement_place_days: 13406, counted_place_days: placeDays, reports_in_period: 41222, reports_not_placed: 0,
    near_before: 500, near_after: 480, near_same_day: 40, ring_before: 5000, ring_after: 5100, reports_since: '2023-01-01', reports_until: '2026-10-04', ...extra });

  it('reads a completion log only beside report tables that are complete and carry coordinates', () => {
    expect(abatementQueries(around, log).map((item) => item.id)).toEqual(['abatement_repeat_places', 'abatement_daily', 'abatement_reports_nearby']);
    expect(reportsAroundAbatements(around, log)).toMatchObject({ since: '2025-01-01', until: '2025-09-30', coordinates: ['lat', 'lng'] });
    // Without the closed-year file, reports closed before the open copy are in no table.
    expect(abatementQueries({ ...around, datasets: [log, open] }, log).map((item) => item.id)).toEqual(['abatement_repeat_places', 'abatement_daily']);
    expect(abatementQueries({ ...around, columns: around.columns.filter((column) => !(column.tableName === closed.tableName && column.columnName === 'lng')) }, log).map((item) => item.id)).toEqual(['abatement_repeat_places', 'abatement_daily']);
    expect(abatementQueries(around, closed)).toEqual([]);
  });

  it('ends the reports at the last Pacific day the open copy holds whole, and keeps the year it was retrieved in', () => {
    // A 2026 retrieval needs the 2026 closed file retrieved no earlier than the open copy.
    const retrieved = (retrievedAt: string, held: AnalyticalSchema['datasets'] = []): AnalyticalSchema => ({ ...around, datasets: [log, ...held,
      { ...closed, sourceUri: closed.sourceUri!.replace('closed_2025', 'closed_2026'), retrievedAt }, { ...open, retrievedAt }] });
    // 02:00 UTC on 10 March is 18:00 on 9 March in Pacific standard time: 9 March is not over.
    expect(reportsAroundAbatements(retrieved('2026-03-10T02:00:00Z'), log)).toMatchObject({ since: '2026-01-01', until: '2026-03-08' });
    expect(abatementQueries(retrieved('2026-03-10T02:00:00Z'), log).find((item) => item.id === 'abatement_reports_nearby')!.sql).toContain(`<= DATE '2026-03-08'`);
    expect(reportsAroundAbatements(retrieved('2026-03-10T09:00:00Z'), log)).toMatchObject({ since: '2026-01-01', until: '2026-03-09' });
    // Pacific standard time ends a day at 08:00 UTC.
    expect(reportsAroundAbatements(retrieved('2026-01-10T07:59:00Z'), log)).toMatchObject({ until: '2026-01-08' });
    expect(reportsAroundAbatements(retrieved('2026-01-10T08:00:00Z'), log)).toMatchObject({ until: '2026-01-09' });
    // 00:30 on 10 July in daylight time: 9 July is over, but the fixed offset stops a day early.
    expect(reportsAroundAbatements(retrieved('2026-07-10T07:30:00Z'), log)).toMatchObject({ until: '2026-07-08' });
    // Before 1 January ends in Pacific time, the 2026 file alone holds no whole day.
    expect(reportsAroundAbatements(retrieved('2026-01-01T03:00:00Z'), log)).toBeNull();
    const earlierYear = { ...closed, snapshotId: `${closed.snapshotId}-2025`, retrievedAt: '2026-01-01T03:00:00Z' };
    expect(reportsAroundAbatements(retrieved('2026-01-01T03:00:00Z', [earlierYear]), log)).toMatchObject({ since: '2025-01-01', until: '2025-12-30' });
  });

  it('states the counts at 30 abatement place-days, not 29, and says what they are counts of', () => {
    expect(abatementLeads([{ query: nearbyQuery, result: counts(29) }])).toEqual([]);
    const [lead] = abatementLeads([{ query: nearbyQuery, result: counts(30) }]);
    expect(lead.title).toBe('Around 30 abatements, 480 Encampment reports were filed within 150 metres in the 14 days after and 500 in the 14 days before');
    expect(lead.detail).toContain('Within 150 metres: 500 reports in the 14 days before, 40 on the day, 480 in the 14 days after. From 150 to 800 metres: 5,000 before and 5,100 after.');
    expect(lead.detail).toContain('a report near several abatements is counted once for each');
    expect(lead.detail).toContain('The counts do not show where anyone went.');
    expect(lead.detail).not.toContain('no coordinates');
    expect(lead.detail).not.toContain('in that ring beforehand');
    expect(abatementLeads([{ query: nearbyQuery, result: counts(11208, { ring_compared: 11208, ring_more_after: 5151, ring_fewer_after: 5884, ring_rose_by_half: 809, ring_fell_by_third: 1114 }) }])[0].detail)
      .toContain('Of 11,208 abatements with at least 10 reports in that ring beforehand, 5,151 had more afterwards and 5,884 fewer; in 809 the count rose by half or more and in 1,114 it fell by a third or more.');
    expect(abatementLeads([{ query: nearbyQuery, result: counts(30, { reports_not_placed: 7 }) }])[0].detail).toContain('7 reports have no coordinates or date and are in none of these counts.');
  });
  it('gives the count taken 28 days earlier only with 30 abatements behind it', () => {
    const earlier = { earlier_near_before: 296235, earlier_near_after: 304996, earlier_ring_before: 2722122, earlier_ring_after: 2707555 };
    expect(abatementLeads([{ query: nearbyQuery, result: counts(40, { earlier_place_days: 29, ...earlier }) }])[0].detail).not.toContain('days earlier');
    expect(abatementLeads([{ query: nearbyQuery, result: counts(40, { earlier_place_days: 30, ...earlier }) }])[0].detail)
      .toContain("The same count taken 28 days earlier at 30 of these abatements' places: within 150 metres 296,235 then 304,996; in the ring 2,722,122 then 2,707,555.");
  });

  it('names places abated on 20 or more separate days, not 19, and says what the log does not record', () => {
    const places = (top: number, repeatPlaces: number) => ({ columns: ['address', 'days', 'first_day', 'last_day', 'places', 'place_days', 'places_abated_once', 'repeat_places', 'repeat_place_days', 'rows_not_placed'],
      rows: [['1600 K Street', top, '2023-07-31', '2026-01-22', 3013, 13387, 2129, repeatPlaces, 7814, 34], ['9 Elm Street', 19, '2024-01-01', '2025-01-01', 3013, 13387, 2129, repeatPlaces, 7814, 34]],
      rowCount: 2, elapsedMs: 1, truncated: false, provenance: [] }) as SqlQueryResult;
    expect(abatementLeads([{ query: repeatQuery, result: places(19, 0) }])).toEqual([]);
    const [lead] = abatementLeads([{ query: repeatQuery, result: places(331, 131) }]);
    expect(lead.title).toBe('7,814 of 13,387 abatements were at 131 places abated on 20 or more separate days');
    expect(lead.detail).toContain('The most abated: 1600 K Street, 331 days from 2023-07-31 to 2026-01-22.');
    expect(lead.detail).not.toContain('9 Elm Street');
    expect(lead.detail).toContain('34 rows have no coordinates or readable time');
    expect(lead.detail).toContain('whether the same people returned');
  });
});
