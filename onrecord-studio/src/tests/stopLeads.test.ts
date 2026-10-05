import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/stop-leads.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import { storedLeads, type AutomaticProfile } from '../utils/automaticDiscovery';
import { dataGaps } from '../utils/dataGaps';
import { stopLeads, stopQueries } from '../utils/stopLeads';

const schema = fixture.schema as AnalyticalSchema;
const stops = schema.datasets[0];
const response = (columns: string[], rows: SqlQueryResult['rows']): SqlQueryResult => ({ columns, rows, rowCount: rows.length, elapsedMs: 1, truncated: false, provenance: [] });
const query = (id: string) => fixture.queries.find((item) => item.id === id)!;
const without = (snapshotId: string): AnalyticalSchema => ({ ...schema, datasets: schema.datasets.filter((dataset) => dataset.snapshotId !== snapshotId) });

describe('leads from police stop records', () => {
  it('needs the results table, and asks for each companion table that is missing', () => {
    expect(stopQueries(schema, stops).map((item) => item.id)).toEqual(['stops_unhoused_outcomes', 'stops_unhoused_reasons', 'stops_unhoused_property']);
    expect(stopQueries(schema, schema.datasets[1])).toEqual([]);
    expect(stopQueries(without('results'), stops)).toEqual([]);
    expect(stopQueries(without('property'), stops).find((item) => item.id === 'stops_unhoused_outcomes')!.sql).toContain('NULL AS property_seized');
    const gaps = (catalog: AnalyticalSchema) => dataGaps(catalog, {}, '2026-10-04T00:00:00Z').map((gap) => gap.fetch.dataset);
    expect(gaps(schema)).toEqual([]);
    expect(gaps(without('results'))).toEqual(['RIPA_STOP_RESULTS']);
    expect(gaps({ ...schema, datasets: [stops] })).toEqual(['RIPA_STOP_RESULTS', 'RIPA_STOP_REASONS', 'RIPA_PROPERTY_SEIZURE_BASIS']);
  });

  it('states a rate only with 100 persons in each group, and an absent property table as no figure', () => {
    const outcomes = (unhoused: number, seized: number | null) => ({ query: query('stops_unhoused_outcomes'), result: response(
      ['perceived', 'persons', 'arrested', 'arrested_on_warrant', 'arrested_without_warrant', 'cited', 'no_result_row', 'property_seized', 'first_stop', 'last_stop'],
      [['not unhoused', 1000, 100, 30, 70, 400, 0, seized, '2024-01-01', '2026-09-28'], ['unhoused', unhoused, 30, 15, 15, 10, 0, seized, '2024-01-02', '2026-09-28'], [null, 7, 0, 0, 0, 0, 0, seized, null, null]]) });
    expect(stopLeads([outcomes(99, 5)])).toEqual([]);
    const [lead] = stopLeads([outcomes(100, 5)]);
    expect(lead.title).toBe('30.0% of people perceived as unhoused were arrested when stopped, against 10.0% of others');
    expect(lead.detail).toContain('100 of 1,100 persons in the stop records, 2024-01-02 to 2026-09-28 were perceived as unhoused.');
    expect(lead.detail).toContain('Arrest on an outstanding warrant: 15 of 100 (15.0%) against 30 of 1,000 (3.0%).');
    expect(lead.detail).toContain('7 persons have no perception recorded and are left out.');
    expect(lead.detail).toContain('not by itself a finding of unequal treatment');
    expect(stopLeads([outcomes(100, null)])[0].detail).not.toContain('Property seized');
  });

  it('calls a same-reason arrest rate higher only at 1.5 times and three points, among reasons with 100 persons each', () => {
    const reasons = (unhousedArrested: number, others: number) => ({ query: query('stops_unhoused_reasons'), result: response(
      ['reason', 'unhoused_persons', 'unhoused_arrested', 'other_persons', 'other_arrested'],
      [['LODGING', 1000, unhousedArrested, others, others / 10], ['TRESPASS', 500, 50, 500, 50]]) });
    const same = (unhousedArrested: number, others = 1000) => stopLeads([reasons(unhousedArrested, others)]).find((lead) => lead.key === 'stops_same_reason');
    expect(same(149)).toBeUndefined();
    expect(same(150)!.title).toBe('Stopped for the same recorded reason, people perceived as unhoused were arrested without a warrant more often in 1 of 2 reasons compared');
    expect(same(150)!.detail).toContain('LODGING: 150 of 1,000 (15.0%) against 100 of 1,000 (10.0%).');
    expect(same(300, 90)).toBeUndefined();
  });

  it('stores stop leads under their own family', () => {
    const profile: AutomaticProfile = { fingerprint: 'f', checkedAt: 'now', results: [{ query: query('stops_unhoused_property'), result: response(['basis', 'persons'], [['Evidence', 90], ['Safekeeping as allowed by law/statute', 10]]) }] };
    const stored = storedLeads(schema, { stops: profile });
    expect(stored.leads).toEqual([expect.objectContaining({ family: 'POLICE_STOPS', key: 'stops:stops_unhoused_property', snapshotId: 'stops', title: 'Property was seized from people perceived as unhoused on 100 recorded bases, 90 as evidence' })]);
    expect(stored.complete).toBe(false);
  });
});
