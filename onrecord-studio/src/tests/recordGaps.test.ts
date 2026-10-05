import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/abatement-leads.json';
import type { AnalyticalSchema, RecordGapRequestStatus, RequestSearchResult, SqlQueryResult } from '../types';
import { profileObservations, type AutomaticProfile } from '../utils/automaticDiscovery';
import { belongingsLeads, belongingsQueries } from '../utils/belongingsLeads';
import type { DiscoveryQuery } from '../utils/dataDiscovery';
import { gapRequestInput, recordGaps, recordGapState } from '../utils/recordGaps';

const schema = fixture.schema as AnalyticalSchema;
const completed = schema.datasets[1];
const table = (columns: string[], rows: unknown[][]): SqlQueryResult => ({ columns, rows: rows as SqlQueryResult['rows'], rowCount: rows.length, elapsedMs: 1, truncated: false, provenance: [] });
const repeatQuery: DiscoveryQuery = { id: 'abatement_repeat_places', title: 't', method: 'm', sql: 'SELECT 1' };
const repeat = table(['address', 'days', 'first_day', 'last_day', 'places', 'place_days', 'places_abated_once', 'repeat_places', 'repeat_place_days', 'rows_not_placed'],
  [['1400 Imperial Ave', 31, '2023-08-02', '2025-07-01', 900, 2000, 400, 2, 55, 0], ['900 B St', 24, '2023-09-10', '2025-06-12', 900, 2000, 400, 2, 55, 0], ['12 Elm St', 3, '2024-01-01', '2024-02-01', 900, 2000, 400, 2, 55, 0]]);
const profile = (results: AutomaticProfile['results']): AutomaticProfile => ({ fingerprint: 'f', checkedAt: 'now', results });
const leadsOf = (profiles: Record<string, AutomaticProfile>, from = schema) => from.datasets.flatMap((dataset) => profiles[dataset.snapshotId] ? profileObservations(from, dataset, profiles[dataset.snapshotId]) : []).filter((item) => item.kind === 'lead');

const storage: AnalyticalSchema = { ...schema, datasets: [...schema.datasets, { ...completed, snapshotId: 'storage', displayName: 'Storage log', tableName: 'analysis_storage', sourceUri: 'https://example.test/storage.csv' }],
  columns: [...schema.columns, ...['date_stored', 'disposition', 'date_released'].map((columnName) => ({ ...schema.columns[0], tableName: 'analysis_storage', columnName, originalName: columnName }))] };
const storageLog = storage.datasets.at(-1)!;
const dispositions = (released: number | null) => table(['disposition', 'entries', 'all_entries', 'dispositions', 'released_entries', 'first_taken', 'last_taken'],
  [['Discarded', 70, 100, 3, released, '2024-01-02', '2024-12-30'], ['Stored', 25, 100, 3, released, '2024-01-02', '2024-12-30'], [null, 5, 100, 3, released, '2024-01-02', '2024-12-30']]);

describe('record gaps behind a lead', () => {
  const profiles = { completed: profile([{ query: repeatQuery, result: repeat }]) };

  it('names the records a lead leaves open, scoped by the lead\'s own places and dates', () => {
    const gaps = recordGaps(schema, leadsOf(profiles), profiles);
    expect(gaps.map((gap) => gap.definition.id)).toEqual(['stored_property_log', 'abatement_work_orders', 'abatement_invoices']);
    const [gap] = gaps;
    expect(gap).toMatchObject({ period: '2023-08-02 to 2025-07-01', identifiers: ['1400 Imperial Ave', '900 B St'], sourceOrigin: 'https://example.test', answered: false });
    const input = gapRequestInput(gap, '');
    expect(input.draft).toContain('To: Records custodian');
    expect(input.draft).toContain('- 900 B St');
    expect(input.draft).toContain(`SHA-256 ${completed.sourceSha256}`);
    expect(input.period).toBe('2023-08-02 to 2025-07-01');
  });

  it('keeps one gap across two copies of the same source', () => {
    const copy = { ...schema, datasets: [...schema.datasets, { ...completed, snapshotId: 'completed-2', retrievedAt: '2026-10-05T00:00:00Z' }] };
    const both = { ...profiles, 'completed-2': profile([{ query: repeatQuery, result: repeat }]) };
    expect(recordGaps(copy, leadsOf(both, copy), both)).toHaveLength(3);
  });

  it('is answered only when the check on the imported record returned rows', () => {
    const answered = (results: AutomaticProfile['results']) => { const all = { ...profiles, storage: profile(results) }; return recordGaps(storage, leadsOf(profiles), all)[0].answered; };
    const [query] = belongingsQueries(storage, storageLog);
    expect(answered([{ query, error: 'failed' }])).toBe(false);
    expect(answered([{ query, result: table(['disposition'], []) }])).toBe(false);
    expect(answered([{ query, result: dispositions(12) }])).toBe(true);
  });
});

describe('the state of a record gap', () => {
  const [gap] = recordGaps(schema, leadsOf({ completed: profile([{ query: repeatQuery, result: repeat }]) }), {});
  const tracked = (patch: Partial<RecordGapRequestStatus>): RecordGapRequestStatus => ({ gapKey: gap.key, caseId: 'c', reportId: 'lead:x', requestId: 'gap-x', state: 'DRAFT', submittedAt: null, determinationDueOn: null, deadlinePassedWithoutRecordedResponse: false, productions: 0, remainingOmissions: 0, ...patch });
  const match = (documentCount: number) => ({ requestId: documentCount, portalKey: 'p', documentCount } as RequestSearchResult);
  const today = '2026-10-05T12:00:00Z';

  it('treats an unsearched library, an empty search and matches without files as not asked', () => {
    expect(recordGapState(gap, undefined, undefined, today).kind).toBe('not_asked');
    expect(recordGapState(gap, undefined, [], today).kind).toBe('not_asked');
    expect(recordGapState(gap, undefined, [match(0)], today).kind).toBe('not_asked');
    expect(recordGapState(gap, undefined, [match(0), match(2)], today)).toEqual({ kind: 'found', requests: 2, withFiles: 1 });
  });

  it('has no deadline until a submission is recorded, then counts to and past it', () => {
    expect(recordGapState(gap, tracked({}), undefined, today)).toEqual({ kind: 'drafted' });
    const filed = { state: 'SUBMITTED', submittedAt: '2026-09-28', determinationDueOn: '2026-10-08' };
    expect(recordGapState(gap, tracked(filed), undefined, today)).toEqual({ kind: 'requested', dueOn: '2026-10-08', daysLeft: 3 });
    expect(recordGapState(gap, tracked({ ...filed, determinationDueOn: '2026-10-04', deadlinePassedWithoutRecordedResponse: true }), undefined, today)).toEqual({ kind: 'overdue', dueOn: '2026-10-04', daysPast: 1 });
  });

  it('stays open after a production until the imported record is checked', () => {
    expect(recordGapState(gap, tracked({ state: 'PRODUCED', submittedAt: '2026-09-01', productions: 1 }), undefined, today).kind).toBe('produced');
    expect(recordGapState({ ...gap, answered: true }, tracked({ state: 'PRODUCED', productions: 1 }), undefined, today).kind).toBe('answered');
  });
});

describe('a stored-property log', () => {
  it('is read only with an intake date and a disposition, and counts no releases without a release column', () => {
    expect(belongingsQueries(schema, completed)).toEqual([]);
    expect(belongingsQueries(storage, storageLog)[0].sql).toContain('"date_released"');
    const bare = { ...storage, columns: storage.columns.filter((column) => column.columnName !== 'date_released') };
    const [query] = belongingsQueries(bare, storageLog);
    expect(query.sql).toContain('CAST(NULL AS BIGINT) AS released_entries');
    const [unreleased] = belongingsLeads([{ query, result: dispositions(null) }]);
    expect(unreleased.detail).toContain('has no release or retrieval date column');
    expect(unreleased.detail).not.toContain('have a release or retrieval date');
  });

  it('states the dispositions as written, blank ones apart, and zero releases as a count', () => {
    const [query] = belongingsQueries(storage, storageLog);
    const [lead] = belongingsLeads([{ query, result: dispositions(0) }]);
    expect(lead.title).toBe('70 of 100 stored-property entries have recorded as "Discarded"');
    expect(lead.detail).toContain('70 recorded as "Discarded"; 25 recorded as "Stored"; 5 no disposition recorded.');
    expect(lead.detail).toContain('0 entries have a release or retrieval date.');
    expect(lead.scope).toEqual({ period: '2024-01-02 to 2024-12-30' });
  });
});
