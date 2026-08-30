import { describe, expect, it } from 'vitest';
import { autoMapColumns, mapEvents, parseCsv, toRfc3339 } from '../components/investigation/AuxiliaryEventImport';

const HEADER = 'Unique Key,Created Date,Incident Address,Complaint Type,Descriptor,Latitude,Longitude';

describe('auxiliary event import', () => {
  it('maps a 311 export by header name and keeps the whole row as the source payload', () => {
    const parsed = parseCsv(
      `${HEADER}\n55123,2026-07-04T09:15:00Z,"1200 BLOCK, MAIN ST",Illegal Dumping,Mattress,37.77,-122.41\n`,
      'sr.csv',
    );
    const mapping = autoMapColumns(parsed.columns);
    expect(mapping.externalId).toBe('Unique Key');
    expect(mapping.occurredAt).toBe('Created Date');
    expect(mapping.neighborhoodBlock).toBe('Incident Address');

    const { events, rejected } = mapEvents(parsed.rows, mapping, '');
    expect(rejected).toHaveLength(0);
    expect(events[0]).toMatchObject({
      externalId: '55123',
      occurredAt: '2026-07-04T09:15:00.000Z',
      neighborhoodBlock: '1200 BLOCK, MAIN ST',
      eventType: 'Illegal Dumping',
      latitude: 37.77,
      longitude: -122.41,
    });
    // The unmapped columns have to survive the import, or a later question can only be answered
    // from the operator's memory of the file.
    expect(events[0].sourcePayload).toMatchObject({ Descriptor: 'Mattress' });
  });

  // The backend counts a bad row only as `rejectedCount`. Without these the operator sees an
  // unexplained shortfall, which reads as "the data was examined and found thin" rather than
  // "these rows never landed".
  it('refuses a row the backend would silently drop, and says which and why', () => {
    const parsed = parseCsv(
      [
        HEADER,
        '55124,not a date,BLOCK A,Noise,,,',
        '55125,2026-07-04T10:00:00Z,,Noise,,,',
        '55129,2026-07-04T10:00:00Z,BLOCK B,Noise,,999,0',
        '55126,2026-07-04T11:00:00Z,BLOCK B,Noise,,,',
        '55126,2026-07-04T11:30:00Z,BLOCK B,Noise,,,',
        '55127,2026-07-04T12:00:00Z,BLOCK C,Noise,,,',
        '55128,2026-07-04T13:00:00Z,BLOCK D',
      ].join('\n'),
      'sr.csv',
    );
    // The short final record is not a row with blank tail columns: its columns do not line up.
    expect(parsed.malformedRowCount).toBe(1);

    const { events, rejected } = mapEvents(parsed.rows, autoMapColumns(parsed.columns), '');
    expect(events.map((event) => event.externalId)).toEqual(['55126', '55127']);
    expect(rejected.map((rejection) => rejection.reason)).toEqual([
      'timestamp is not a recognizable date',
      'no neighborhood or block',
      'coordinates are outside valid bounds',
      'duplicate identifier 55126',
    ]);
  });

  // REGRESSION(import): `new Date('2026-07-04 09:15:00').toISOString()` reads the value as this
  // machine's local time and then labels the result UTC, which is the defect 60300c9 removed from
  // the storage layer. Proven by restoring the bare `new Date(value)`: the first case below then
  // yields an instant that shifts with the runner's zone.
  it('refuses a timestamp with no offset until the export offset is stated', () => {
    expect(toRfc3339('2026-07-04 09:15:00', '')).toEqual({
      reason: 'timestamp carries no time zone; state the export offset above',
    });
    expect(toRfc3339('2026-07-04 09:15:00', '-07:00')).toEqual({ instant: '2026-07-04T16:15:00.000Z' });
    // An offset the value already carries is authoritative; the stated one does not override it.
    expect(toRfc3339('2026-07-04T09:15:00Z', '-07:00')).toEqual({ instant: '2026-07-04T09:15:00.000Z' });
    // The other shape municipal CSV exports use.
    expect(toRfc3339('7/4/2026 9:15:00 PM', '+00:00')).toEqual({ instant: '2026-07-04T21:15:00.000Z' });
    expect(toRfc3339('last Tuesday', '-07:00')).toEqual({ reason: 'timestamp is not a recognizable date' });
  });
});
