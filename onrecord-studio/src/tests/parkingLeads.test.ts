import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/parking-leads.json';
import service from '../../src-tauri/tests/fixtures/service-accountability.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import { storedLeads } from '../utils/automaticDiscovery';
import { parkingLeads, parkingQueries } from '../utils/parkingLeads';

const schema = fixture.schema as AnalyticalSchema;
const [citations] = schema.datasets;
const query = (id: string) => fixture.queries.find((item) => item.id === id)!;
const result = (rows: Record<string, unknown>[], truncated = false): SqlQueryResult => {
  const columns = [...new Set(rows.flatMap((row) => Object.keys(row)))];
  return { columns, rowCount: rows.length, elapsedMs: 1, truncated, provenance: [], rows: rows.map((row) => columns.map((name) => row[name] ?? null)) };
};
const section = (key: string, n: number, change: Record<string, unknown> = {}) => ({ section: key, citations: n, by_police: Math.min(n, 20), by_parking: Math.max(n - 20, 0), other_sector: 0, no_sector: 0,
  conflicting: 0, fines: n ? n * 112.5 : null, unread_fines: 0, warnings: 0, sector_texts: n ? 'CSD (I) PARKING ENF; CSD (S) POLICE' : null, all_citations: 120000, first_issue: '2022-07-01', last_issue: '2022-12-31', ...change });
const sections = (oversized: number, unmoved = 12, habitation = 0, change: Record<string, unknown> = {}, truncated = false) => ({ query: query('parking_vehicle_sections'),
  result: result([section('oversized', oversized, change), section('unmoved', unmoved), section('habitation', habitation)], truncated) });
const blocks = (top: number) => ({ query: query('parking_vehicle_blocks'), result: result([
  { block: '4600 PACIFIC HWY', citations: top, oversized: top, unmoved: 0, by_police: top - 1, section_citations: 1628, no_block: 2, first_issue: '2022-07-01', last_issue: '2022-12-31' },
  { block: '7700 SIEMPRE VIVA RD', citations: 20, oversized: 0, unmoved: 20, by_police: 0, section_citations: 1628, no_block: 2, first_issue: '2022-07-01', last_issue: '2022-12-31' }]) });

describe('leads from a parking citation file', () => {
  it('reads only a table with the citation file\'s columns, sector1 among them', () => {
    expect(parkingQueries(schema, citations).map((item) => item.id)).toEqual(['parking_vehicle_sections', 'parking_vehicle_blocks']);
    const unsectored: AnalyticalSchema = { ...schema, columns: schema.columns.filter((column) => column.columnName !== 'sector1') };
    expect(parkingQueries(unsectored, citations)).toEqual([]);
    const reports = service.schema as AnalyticalSchema;
    expect(parkingQueries(reports, reports.datasets[0])).toEqual([]);
  });

  it('describes the oversized-vehicle section at 30 citations, not 29', () => {
    expect(parkingLeads([sections(29)])).toEqual([]);
    const [lead] = parkingLeads([sections(30)]);
    expect(lead.title).toBe('30 citations under SDMC 86.0139(A) (oversized or recreational vehicle) from 2022-07-01 to 2022-12-31, 20 written by police');
    expect(lead.scope).toEqual({ period: '2022-07-01 to 2022-12-31' });
    expect(lead.detail).toContain('SDMC 86.0137(G) (vehicle not moved 1/10 mile in 72 hours): 12 citations; 12 written by police');
    expect(lead.detail).toContain('not whether anyone lived in it');
    expect(parkingLeads([sections(31)])[0].title).toBe('31 citations under SDMC 86.0139(A) (oversized or recreational vehicle) from 2022-07-01 to 2022-12-31, 20 written by police');
  });

  it('heads the lead with the 72-hour section only when it reaches 30 and the oversized section does not', () => {
    expect(parkingLeads([sections(12, 29)])).toEqual([]);
    expect(parkingLeads([sections(12, 30)])[0].title).toBe('30 citations under SDMC 86.0137(G) (vehicle not moved in 72 hours) from 2022-07-01 to 2022-12-31, 20 written by police');
  });

  it('gives no fine figure when no fine is readable, never $0', () => {
    const { detail } = parkingLeads([sections(30, 12, 0, { fines: null, unread_fines: 30 })])[0];
    expect(detail).toContain('SDMC 86.0139(A) (oversized, non-motorized or recreational vehicle): 30 citations; 20 written by police, 10 by parking enforcement, 0 by another City unit and 0 with no issuing unit recorded. Issuing units as written: CSD (I) PARKING ENF; CSD (S) POLICE. Their fines are not recorded in a readable form.');
    expect(detail).not.toContain('$0');
    expect(parkingLeads([sections(30, 12, 0, { unread_fines: 2, fines: 28 * 112.5 })])[0].detail).toContain('The fines written on them total $3,150.00; 2 have no readable fine and are not in that total.');
  });

  it('states the vehicle-habitation count the file holds rather than assuming it holds none', () => {
    expect(parkingLeads([sections(30)])[0].detail).toContain('No citation under SDMC 86.0137(F) (vehicle habitation) is in this file.');
    expect(parkingLeads([sections(30, 12, 36)])[0].detail).toContain('36 citations under SDMC 86.0137(F) (vehicle habitation) are in this file, 20 written by police.');
  });

  it('names the location with most citations at 30, not 29', () => {
    expect(parkingLeads([blocks(29)])).toEqual([]);
    const [lead] = parkingLeads([blocks(30)]);
    expect(lead.title).toBe('30 of 1,628 oversized-vehicle and 72-hour citations from 2022-07-01 to 2022-12-31 were written at 4600 PACIFIC HWY, 29 by police');
    expect(lead.scope).toEqual({ period: '2022-07-01 to 2022-12-31' });
    expect(lead.detail).toContain('7700 SIEMPRE VIVA RD: 20 (0 oversized, 20 72-hour; 0 by police). 2 have no location recorded.');
  });

  it('gives no lead from a result cut short by the row limit', () => {
    const cut = blocks(500);
    expect(parkingLeads([sections(500, 12, 0, {}, true), { ...cut, result: { ...cut.result, truncated: true } }])).toEqual([]);
  });

  it('stores the parking leads under their own family', () => {
    const stored = storedLeads(schema, { parking_2022_2: { fingerprint: 'f', checkedAt: 'now', results: [sections(30), blocks(30)] } });
    expect(stored.leads).toEqual([
      expect.objectContaining({ family: 'PARKING_CITATIONS', key: 'parking_2022_2:parking_vehicle_sections', snapshotIds: ['parking_2022_2'] }),
      expect.objectContaining({ family: 'PARKING_CITATIONS', key: 'parking_2022_2:parking_vehicle_blocks', snapshotIds: ['parking_2022_2'] }),
    ]);
  });
});
