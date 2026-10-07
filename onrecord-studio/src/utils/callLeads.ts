import type { AnalyticalDatasetSummary, AnalyticalSchema, CityDataset, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import type { ServiceLead } from './serviceAccountability';

/** SDPD calls for service: one table per calendar year, and code lookups published beside them.
 * Tables are recognized by the publisher's column names. */
const CALL_COLUMNS = ['incident_num', 'call_type', 'disposition', 'date_time'];
const COMPANIONS = {
  dispositions: { dataset: 'DISPOSITION_CODES', holds: 'what each disposition code means' },
  beats: { dataset: 'BEAT_CODES', holds: 'the neighborhood each beat covers' },
} as const satisfies Record<string, { dataset: CityDataset; holds: string }>;
type Companion = keyof typeof COMPANIONS;
/** The call types read, by the publisher's exact code. 5150V and other variants are not these. */
const CALL_TYPES = ['SLEEPER', 'CW', '602', '5150', '647F'] as const;
const CALL_NAMES: Record<string, string> = { CW: 'CW (check the welfare)', 602: '602 (trespassing)', 5150: '5150 (listed as mental case)', '647F': '647F (drunk, drugs or alcohol)' };
/** Cancelled before a unit was sent or on scene, or a second call about the same incident. */
const CANCELLED = ['X', 'W', 'CAN', 'DUP', 'V'];
/** A call type is described for a year only with this many calls not cancelled behind it. */
const CALL_MINIMUM = 100;

const has = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary, names: string[]) =>
  names.every((name) => schema.columns.some((column) => column.tableName === dataset.tableName && column.columnName === name));
const columnsOf = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary) => schema.columns.filter((column) => column.tableName === dataset.tableName).map((column) => column.columnName);
const latestFirst = (a: AnalyticalDatasetSummary, b: AnalyticalDatasetSummary) => b.retrievedAt.localeCompare(a.retrievedAt) || a.snapshotId.localeCompare(b.snapshotId);
const isCalls = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary) => has(schema, dataset, CALL_COLUMNS);
// The NIBRS offense files also carry beat and neighborhood, one row per offense; joined as a
// lookup they would multiply the calls.
const isBeatList = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary) => has(schema, dataset, ['beat', 'neighborhood'])
  && !columnsOf(schema, dataset).some((name) => ['incident_num', 'nibrs_uniq', 'case_number'].includes(name));

export interface CallCompanions {
  /** The latest copy of each published year file, latest retrieved first. */
  years: AnalyticalDatasetSummary[];
  dispositions?: AnalyticalDatasetSummary;
  beats?: AnalyticalDatasetSummary;
}

/** The tables the calls checks read, or null unless this is the latest-retrieved calls table. */
export function callCompanions(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): CallCompanions | null {
  const calls = schema.datasets.filter((table) => isCalls(schema, table)).sort(latestFirst);
  if (calls[0]?.snapshotId !== dataset.snapshotId) return null;
  // A re-fetched copy of a year replaces the earlier copy rather than adding to it.
  const years = new Map<string, AnalyticalDatasetSummary>();
  for (const table of calls) if (!years.has(table.sourceUri ?? table.snapshotId)) years.set(table.sourceUri ?? table.snapshotId, table);
  const dispositions = schema.datasets.filter((table) => has(schema, table, ['dispo_code', 'description'])).sort(latestFirst)[0];
  const beats = schema.datasets.filter((table) => isBeatList(schema, table)).sort(latestFirst)[0];
  return { years: [...years.values()], ...(dispositions ? { dispositions } : {}), ...(beats ? { beats } : {}) };
}

/** Lookup tables the latest calls table lacks, each named with the published table that supplies it. */
export function missingCallCompanions(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): { dataset: 'DISPOSITION_CODES' | 'BEAT_CODES'; holds: string }[] {
  const companions = callCompanions(schema, dataset);
  return companions ? (Object.keys(COMPANIONS) as Companion[]).filter((name) => !companions[name]).map((name) => COMPANIONS[name]) : [];
}

const code = (name: string) => `UPPER(TRIM(CAST(${quoteIdentifier(name)} AS VARCHAR)))`;
const list = (codes: readonly string[]) => codes.map((value) => `'${value}'`).join(', ');
/** Every year's rows, then one row per incident: the publisher repeats an incident number when a
 * call is updated, and the latest entry is the one kept. Repeats often share a date_time, so the
 * latest entry cannot always be told apart; an incident whose latest entries disagree on the
 * disposition is marked conflicted rather than resolved by code order, which would favour A. */
const deduplicated = (years: AnalyticalDatasetSummary[], withBeat: boolean) => `call_rows AS (
${years.map((table) => ` SELECT TRIM(CAST(${quoteIdentifier('incident_num')} AS VARCHAR)) AS incident, ${code('call_type')} AS call_type, NULLIF(${code('disposition')}, '') AS dispo,
  TRY_CAST(CAST(${quoteIdentifier('date_time')} AS VARCHAR) AS TIMESTAMP) AS called_at${withBeat ? `, NULLIF(TRIM(CAST(${quoteIdentifier('beat')} AS VARCHAR)), '') AS beat` : ''}
 FROM ${quoteIdentifier(table.tableName)}`).join('\n UNION ALL\n')}
), incidents AS (
 SELECT *, incident IS NOT NULL AND MIN(COALESCE(dispo, '')) OVER (PARTITION BY incident, called_at) <> MAX(COALESCE(dispo, '')) OVER (PARTITION BY incident, called_at) AS conflicted
 FROM call_rows
 QUALIFY incident IS NULL OR ROW_NUMBER() OVER (PARTITION BY incident ORDER BY called_at DESC NULLS LAST, dispo NULLS LAST) = 1
)`;
const CAVEAT = 'A call type is the dispatcher\'s label when the call was entered, not a finding that anyone was unhoused. K means no report was required, not that no service was given. The file records no arrival or clear time. The publisher leaves out sensitive call types such as domestic violence and child abuse.';

export function callQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const companions = callCompanions(schema, dataset);
  if (!companions) return [];
  const { years, dispositions, beats } = companions;
  const names = years.map((table) => table.displayName).join(', ');
  const outcome = `CASE WHEN conflicted THEN 'conflicting' WHEN dispo IS NULL THEN 'blank' WHEN dispo IN (${list(CANCELLED)}) THEN 'cancelled_duplicate' WHEN STARTS_WITH(dispo, 'A') THEN 'arrest'
  WHEN STARTS_WITH(dispo, 'R') THEN 'report' WHEN STARTS_WITH(dispo, 'K') THEN 'no_report' WHEN dispo = 'U' THEN 'unfounded' ELSE 'other' END`;
  const label = dispositions ? `dispo || COALESCE(' (' || codes.description || ')', '')` : 'dispo';
  const queries: DiscoveryQuery[] = [{
    id: 'calls_outcomes', title: 'How SLEEPER, welfare, trespass, 5150 and 647F calls ended, by year',
    method: `Calls in ${names}, one row per incident number (the latest entry when the publisher repeats one; where the latest entries share a time but not a disposition, the incident is counted as conflicting and under no outcome), whose call type is exactly ${CALL_TYPES.join(', ')}, counted per calendar year of date_time by disposition code. A code beginning A is an arrest, R a report, K no report required; U is unfounded; ${CANCELLED.join(', ')} are calls cancelled before a unit was sent or on scene, or duplicate calls, and are counted apart. A code ending HR is a call the Crisis Response Team was involved in and is also counted under its first letter. A blank disposition is counted apart, not as K. Other codes are listed${dispositions ? ` with their description from ${dispositions.displayName}` : ' as written'}. Where a year's file was retrieved more than once, only the latest copy is read.`,
    sql: `WITH ${deduplicated(years, false)}, typed AS (
 SELECT YEAR(called_at) AS call_year, call_type, dispo, called_at, ${outcome} AS outcome FROM incidents WHERE call_type IN (${list(CALL_TYPES)})
)${dispositions ? `, codes AS (
 SELECT ${code('dispo_code')} AS code, MIN(TRIM(CAST(${quoteIdentifier('description')} AS VARCHAR))) AS description FROM ${quoteIdentifier(dispositions.tableName)} GROUP BY 1
)` : ''}
SELECT call_year, call_type, COUNT(*) AS calls, COUNT(*) FILTER (WHERE outcome = 'cancelled_duplicate') AS cancelled_duplicate, COUNT(*) FILTER (WHERE outcome = 'blank') AS blank, COUNT(*) FILTER (WHERE outcome = 'conflicting') AS conflicting,
 COUNT(*) FILTER (WHERE outcome = 'arrest') AS arrest, COUNT(*) FILTER (WHERE outcome = 'report') AS report, COUNT(*) FILTER (WHERE outcome = 'no_report') AS no_report,
 COUNT(*) FILTER (WHERE outcome = 'unfounded') AS unfounded, COUNT(*) FILTER (WHERE outcome = 'other') AS other, COUNT(*) FILTER (WHERE outcome <> 'conflicting' AND ENDS_WITH(dispo, 'HR')) AS crisis_team,
 STRING_AGG(DISTINCT ${label}, '; ') FILTER (WHERE outcome = 'other') AS other_codes, CAST(MAX(called_at) AS VARCHAR) AS last_call
FROM typed${dispositions ? ' LEFT JOIN codes ON codes.code = typed.dispo' : ''}
GROUP BY call_year, call_type ORDER BY call_year DESC NULLS LAST, call_type`,
  }];
  if (years.every((table) => has(schema, table, ['beat']))) queries.push({
    id: 'calls_sleeper_beats', title: 'Where SLEEPER calls were sent, by police beat',
    method: `SLEEPER calls in ${names}, one row per incident number as above, leaving out the cancelled and duplicate codes ${CANCELLED.join(', ')} unless the incident's latest entries disagree on the disposition, counted by the beat of the call's address${beats ? ` with the neighborhood from ${beats.displayName}` : ''}. The ten beats with most calls are read, with the citywide total and the calls with no beat recorded.`,
    sql: `WITH ${deduplicated(years, true)}, sleeper AS (
 SELECT beat, called_at FROM incidents WHERE call_type = 'SLEEPER' AND (conflicted OR dispo IS NULL OR dispo NOT IN (${list(CANCELLED)}))
)${beats ? `, places AS (
 SELECT TRIM(CAST(${quoteIdentifier('beat')} AS VARCHAR)) AS beat, MIN(TRIM(CAST(${quoteIdentifier('neighborhood')} AS VARCHAR))) AS neighborhood FROM ${quoteIdentifier(beats.tableName)} GROUP BY 1
)` : ''}
, beat_calls AS (
 SELECT beat, COUNT(*) AS calls FROM sleeper WHERE beat IS NOT NULL GROUP BY beat
), sleeper_totals AS (
 SELECT COUNT(*) AS citywide_calls, COUNT(*) FILTER (WHERE beat IS NULL) AS no_beat, MIN(YEAR(called_at)) AS first_year, MAX(YEAR(called_at)) AS last_year FROM sleeper
)
SELECT beat_calls.beat, ${beats ? 'places.neighborhood' : 'CAST(NULL AS VARCHAR)'} AS neighborhood, beat_calls.calls, citywide_calls, no_beat, first_year, last_year
FROM beat_calls CROSS JOIN sleeper_totals${beats ? ' LEFT JOIN places ON places.beat = beat_calls.beat' : ''}
ORDER BY beat_calls.calls DESC, beat_calls.beat LIMIT 10`,
  });
  return queries;
}

const count = (value: unknown): number | null => {
  if (value == null || value === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};
const shown = (value: number) => Math.round(value).toLocaleString('en-US');
const percent = (part: number, whole: number) => `${(100 * part / whole).toFixed(1)}%`;
const records = (result: SqlQueryResult) => result.rows.map((row) => Object.fromEntries(result.columns.map((name, i) => [name, row[i]])));
type Entry = { query: DiscoveryQuery; result?: SqlQueryResult };
/** A result cut short by the row limit would give short counts, so it gives none. */
const rowsOf = (results: Entry[], id: string) => {
  const entry = results.find((item) => item.query.id === id);
  return entry?.result?.rows.length && !entry.result.truncated ? { query: entry.query, rows: records(entry.result) } : null;
};

interface TypeYear { year: number | null; type: string; calls: number; cancelled: number; blank: number; conflicting: number; arrest: number; report: number; noReport: number; unfounded: number; other: number; crisis: number; otherCodes: string | null }
const typeYear = (row: Record<string, unknown>): TypeYear => ({ year: count(row.call_year), type: String(row.call_type), calls: count(row.calls) ?? 0, cancelled: count(row.cancelled_duplicate) ?? 0,
  blank: count(row.blank) ?? 0, conflicting: count(row.conflicting) ?? 0, arrest: count(row.arrest) ?? 0, report: count(row.report) ?? 0, noReport: count(row.no_report) ?? 0, unfounded: count(row.unfounded) ?? 0, other: count(row.other) ?? 0,
  crisis: count(row.crisis_team) ?? 0, otherCodes: typeof row.other_codes === 'string' && row.other_codes ? row.other_codes : null });
/** Calls not cancelled or duplicated: the ones a unit was sent to or that were otherwise closed. */
const sent = (item: TypeYear) => item.calls - item.cancelled;
const split = (item: TypeYear) => {
  const handled = sent(item) - item.blank - item.conflicting;
  const share = (part: number) => handled > 0 ? `${shown(part)} (${percent(part, handled)})` : shown(part);
  return `${shown(sent(item))} calls not cancelled; of the ${shown(handled)} with a disposition, ${share(item.arrest)} ended in arrest, ${share(item.crisis)} with the Crisis Response Team, ${share(item.noReport)} with no report required, ${share(item.report)} with a report, ${share(item.unfounded)} unfounded and ${share(item.other)} with another code${item.otherCodes ? ` (${item.otherCodes})` : ''}; ${shown(item.blank)} have no disposition recorded${item.conflicting ? `, ${shown(item.conflicting)} were entered with different dispositions at the same time and are counted under none,` : ''} and ${shown(item.cancelled)} were cancelled or duplicates.`;
};

export function callLeads(results: Entry[], currentYear = new Date().getUTCFullYear()): ServiceLead[] {
  const leads: ServiceLead[] = [];
  const outcomes = rowsOf(results, 'calls_outcomes');
  if (outcomes) {
    const rows = outcomes.rows.map(typeYear);
    const sleeper = rows.filter((item) => item.type === 'SLEEPER' && item.year !== null).sort((a, b) => b.year! - a.year!);
    const latest = sleeper.find((item) => sent(item) >= CALL_MINIMUM);
    if (latest) {
      const year = latest.year!;
      const soFar = year >= currentYear ? ' so far' : '';
      const others = rows.filter((item) => item.year === year && item.type !== 'SLEEPER' && sent(item) >= CALL_MINIMUM)
        .sort((a, b) => CALL_TYPES.indexOf(a.type as typeof CALL_TYPES[number]) - CALL_TYPES.indexOf(b.type as typeof CALL_TYPES[number]));
      const earlier = sleeper.filter((item) => item.year !== year);
      const undated = rows.filter((item) => item.type === 'SLEEPER' && item.year === null).reduce((sum, item) => sum + sent(item), 0);
      leads.push({ key: 'calls_outcomes', priority: 98, query: outcomes.query,
        title: `${shown(sent(latest))} calls dispatched as SLEEPER in ${year}${soFar}: ${shown(latest.arrest)} ended in arrest, ${shown(latest.crisis)} with the Crisis Response Team, ${shown(latest.noReport)} with no report required`,
        detail: `SLEEPER in ${year}${soFar}: ${shown(latest.calls)} calls, ${split(latest)} Cancelled and duplicate calls (${CANCELLED.join(', ')}) are left out of the calls counted, and calls with no disposition are not counted as no report required. A code ending HR is counted both under the Crisis Response Team and under its first letter, so the figures overlap.${others.map((item) => ` ${CALL_NAMES[item.type] ?? item.type} in ${year}: ${shown(item.calls)} calls, ${split(item)}`).join('')}${earlier.length ? ` SLEEPER in other years: ${earlier.map((item) => `${item.year}${item.year! >= currentYear ? ' so far' : ''}: ${shown(sent(item))} calls not cancelled, ${shown(item.arrest)} ended in arrest`).join('; ')}.` : ''}${undated ? ` ${shown(undated)} SLEEPER calls have no readable date and are in no year.` : ''} ${CAVEAT}`,
        question: 'What did the officers record on the SLEEPER calls that ended in arrest, and what was each arrest for? The CAD event narratives and arrest reports for these incidents would show it.',
      });
    }
  }
  const places = rowsOf(results, 'calls_sleeper_beats');
  const citywide = places ? count(places.rows[0].citywide_calls) ?? 0 : 0;
  if (places && citywide >= CALL_MINIMUM) {
    const [top] = places.rows;
    const place = (row: Record<string, unknown>, word = 'beat') => `${word} ${String(row.beat)}${typeof row.neighborhood === 'string' && row.neighborhood ? ` (${row.neighborhood})` : ''}`;
    const first = count(top.first_year); const last = count(top.last_year);
    const span = first === null || last === null ? '' : first === last ? ` in ${first}` : ` from ${first} to ${last}`;
    const spanSoFar = span && last! >= currentYear ? `${span} so far` : span;
    const noBeat = count(top.no_beat) ?? 0;
    const unnamed = places.rows.some((row) => row.neighborhood == null);
    const listed = places.query.sql.includes('LEFT JOIN places');
    leads.push({ key: 'calls_sleeper_beats', priority: 95, query: places.query,
      title: `${shown(count(top.calls) ?? 0)} of ${shown(citywide)} SLEEPER calls${spanSoFar} were to addresses in ${place(top)}`,
      detail: `${places.rows.map((row) => `${place(row, 'Beat')}: ${shown(count(row.calls) ?? 0)} (${percent(count(row.calls) ?? 0, citywide)}).`).join(' ')}${noBeat ? ` ${shown(noBeat)} calls have no beat recorded.` : ''}${unnamed ? listed ? ' A beat with no neighborhood named is not in the imported beat list.' : ' No beat list is imported, so beats are named by code.' : ''} Cancelled and duplicate calls are left out. A beat's count is of calls to addresses in it, not a count of people. ${CAVEAT}`,
      question: 'Which units answered the SLEEPER calls in these beats, and what did they record? The CAD event narratives and incident reports for the beats named would show it.',
    });
  }
  return leads;
}
