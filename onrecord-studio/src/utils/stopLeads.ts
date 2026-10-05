import type { AnalyticalDatasetSummary, AnalyticalSchema, CityDataset, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import type { ServiceLead } from './serviceAccountability';

/** Police stop records (RIPA): one table of persons stopped, and companion tables keyed by the
 * same stop and person numbers. Tables are recognized by the publisher's column names. */
const COMPANIONS = {
  results: { column: 'result', dataset: 'RIPA_STOP_RESULTS', holds: 'what each stop resulted in' },
  reasons: { column: 'reasonforstopcodetext', dataset: 'RIPA_STOP_REASONS', holds: 'the recorded reason for each stop' },
  property: { column: 'basisforpropertyseizure', dataset: 'RIPA_PROPERTY_SEIZURE_BASIS', holds: 'why property was seized' },
} as const satisfies Record<string, { column: string; dataset: CityDataset; holds: string }>;
type Companion = keyof typeof COMPANIONS;
const PERCEIVED = 'perceivedunhoused';
/** A rate is stated for a group only with this many persons behind it. */
const GROUP_MINIMUM = 100;
/** A same-reason arrest rate is called higher at this ratio and this many percentage points. */
const HIGHER_RATIO = 1.5;
const HIGHER_POINTS = 3;

const has = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary, ...names: string[]) =>
  names.every((name) => schema.columns.some((column) => column.tableName === dataset.tableName && column.columnName === name));
const isStops = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary) => has(schema, dataset, 'id', 'pid', PERCEIVED);

/** The latest imported companion table of each kind. */
export function stopCompanions(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): Partial<Record<Companion, AnalyticalDatasetSummary>> | null {
  if (!isStops(schema, dataset)) return null;
  const latest = (column: string) => schema.datasets.filter((other) => has(schema, other, 'id', 'pid', column) && !has(schema, other, PERCEIVED))
    .sort((a, b) => b.retrievedAt.localeCompare(a.retrievedAt) || a.snapshotId.localeCompare(b.snapshotId))[0];
  return Object.fromEntries((Object.keys(COMPANIONS) as Companion[]).flatMap((name) => { const table = latest(COMPANIONS[name].column); return table ? [[name, table]] : []; }));
}

/** Companion tables a stops table lacks, each named with the published table that supplies it. */
export function missingStopCompanions(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): { dataset: CityDataset; holds: string }[] {
  const companions = stopCompanions(schema, dataset);
  return companions ? (Object.keys(COMPANIONS) as Companion[]).filter((name) => !companions[name]).map((name) => COMPANIONS[name]) : [];
}

// Stop and person numbers are imported as text or numbers depending on the table; compared as whole numbers.
const key = (name: string) => `CAST(TRY_CAST(TRY_CAST(CAST(${quoteIdentifier(name)} AS VARCHAR) AS DOUBLE) AS BIGINT) AS VARCHAR)`;
const person = `${key('id')} || ':' || ${key('pid')}`;
const lower = (name: string) => `LOWER(TRIM(CAST(${quoteIdentifier(name)} AS VARCHAR)))`;
const perceived = `CASE ${lower(PERCEIVED)} WHEN 'true' THEN 'unhoused' WHEN 'false' THEN 'not unhoused' END`;
const arrested = (table: AnalyticalDatasetSummary) => `SELECT ${person} AS person, BOOL_OR(CONTAINS(${lower('result')}, 'custodial arrest')) AS arrested, BOOL_OR(CONTAINS(${lower('result')}, 'arrest without warrant')) AS warrantless, BOOL_OR(CONTAINS(${lower('result')}, 'outstanding warrant')) AS on_warrant,
  BOOL_OR(CONTAINS(${lower('result')}, 'citation') OR CONTAINS(${lower('result')}, 'cite and release')) AS cited
 FROM ${quoteIdentifier(table.tableName)} GROUP BY 1`;
const basis = 'Perceived unhoused is the officer\'s entry for each person stopped; rows where it is neither true nor false are counted apart.';

export function stopQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const companions = stopCompanions(schema, dataset);
  if (!companions?.results) return [];
  const stops = quoteIdentifier(dataset.tableName);
  const dated = has(schema, dataset, 'stopdate');
  const persons = `stop_persons AS (
 SELECT DISTINCT ${person} AS person, ${perceived} AS perceived${dated ? `, TRY_CAST(CAST(${quoteIdentifier('stopdate')} AS VARCHAR) AS DATE) AS stopped_on` : ''}
 FROM ${stops}
), stop_results AS (
 ${arrested(companions.results)}
)`;
  const queries: DiscoveryQuery[] = [{
    id: 'stops_unhoused_outcomes', title: 'Stops of people perceived as unhoused, and what they resulted in',
    method: `Persons in ${dataset.displayName} by the officer's perceived-unhoused entry, joined on stop and person number to ${companions.results.displayName}${companions.property ? ` and ${companions.property.displayName}` : ''}. Arrested means a result naming a custodial arrest; arrests on an outstanding warrant and arrests without a warrant are also counted apart, since a warrant arrest follows from the warrant rather than the stop. Cited means a citation or an in-field cite and release. A person can have several results. ${basis} Rates are not adjusted for the reason for the stop.`,
    sql: `WITH ${persons}${companions.property ? `, stop_seized AS (SELECT DISTINCT ${person} AS person FROM ${quoteIdentifier(companions.property.tableName)})` : ''}
SELECT perceived, COUNT(*) AS persons, COUNT(*) FILTER (WHERE arrested) AS arrested, COUNT(*) FILTER (WHERE on_warrant) AS arrested_on_warrant, COUNT(*) FILTER (WHERE warrantless) AS arrested_without_warrant, COUNT(*) FILTER (WHERE cited) AS cited,
 COUNT(*) FILTER (WHERE stop_results.person IS NULL) AS no_result_row,
 ${companions.property ? 'COUNT(*) FILTER (WHERE stop_seized.person IS NOT NULL)' : 'NULL'} AS property_seized,
 ${dated ? `CAST(MIN(stopped_on) FILTER (WHERE stopped_on >= DATE '2000-01-01') AS VARCHAR)` : 'NULL'} AS first_stop, ${dated ? 'CAST(MAX(stopped_on) AS VARCHAR)' : 'NULL'} AS last_stop
FROM stop_persons LEFT JOIN stop_results ON stop_results.person = stop_persons.person${companions.property ? ' LEFT JOIN stop_seized ON stop_seized.person = stop_persons.person' : ''}
GROUP BY perceived ORDER BY perceived`,
  }];
  if (companions.reasons) queries.push({
    id: 'stops_unhoused_reasons', title: 'Why people perceived as unhoused were stopped, and arrests for the same reason',
    method: `The recorded reason code text from ${companions.reasons.displayName} for each person stopped, with how many persons perceived as unhoused and how many others were stopped for it and how many of each were arrested without a warrant (a result naming a custodial arrest without warrant), so an outstanding warrant does not explain the arrest. The forty reasons with the most persons perceived as unhoused are read. A person with two reasons is counted under each. ${basis}`,
    sql: `WITH ${persons}, stop_reasons AS (
 SELECT DISTINCT ${person} AS person, NULLIF(TRIM(CAST(${quoteIdentifier(COMPANIONS.reasons.column)} AS VARCHAR)), '') AS reason FROM ${quoteIdentifier(companions.reasons.tableName)}
)
SELECT reason, COUNT(*) FILTER (WHERE perceived = 'unhoused') AS unhoused_persons, COUNT(*) FILTER (WHERE perceived = 'unhoused' AND warrantless) AS unhoused_arrested,
 COUNT(*) FILTER (WHERE perceived = 'not unhoused') AS other_persons, COUNT(*) FILTER (WHERE perceived = 'not unhoused' AND warrantless) AS other_arrested
FROM stop_persons JOIN stop_reasons ON stop_reasons.person = stop_persons.person LEFT JOIN stop_results ON stop_results.person = stop_persons.person
WHERE reason IS NOT NULL GROUP BY reason ORDER BY unhoused_persons DESC, reason LIMIT 40`,
  });
  if (companions.property) queries.push({
    id: 'stops_unhoused_property', title: 'Why property was seized from people perceived as unhoused',
    method: `Persons perceived as unhoused with a row in ${companions.property.displayName}, by the recorded basis for the seizure. A person with two bases is counted under each. The table records that property was seized, not where it was kept or whether it was returned.`,
    sql: `WITH ${persons}, stop_basis AS (
 SELECT DISTINCT ${person} AS person, NULLIF(TRIM(CAST(${quoteIdentifier(COMPANIONS.property.column)} AS VARCHAR)), '') AS basis FROM ${quoteIdentifier(companions.property.tableName)}
)
SELECT basis, COUNT(*) AS persons FROM stop_persons JOIN stop_basis ON stop_basis.person = stop_persons.person
WHERE perceived = 'unhoused' AND basis IS NOT NULL GROUP BY basis ORDER BY persons DESC, basis`,
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
const UNADJUSTED = 'Perception is the officer\'s entry, a person can have several results, and a difference in rates is not by itself a finding of unequal treatment.';

export function stopLeads(results: { query: DiscoveryQuery; result?: SqlQueryResult }[]): ServiceLead[] {
  const leads: ServiceLead[] = [];
  for (const { query, result } of results) {
    if (!result?.rows.length) continue;
    const rows = records(result);
    if (query.id === 'stops_unhoused_outcomes') {
      const group = (name: string) => rows.find((row) => row.perceived === name);
      const unhoused = group('unhoused'); const others = group('not unhoused');
      const u = count(unhoused?.persons) ?? 0; const o = count(others?.persons) ?? 0;
      if (!unhoused || !others || u < GROUP_MINIMUM || o < GROUP_MINIMUM) continue;
      const unrecorded = rows.filter((row) => row.perceived == null).reduce((sum, row) => sum + (count(row.persons) ?? 0), 0);
      const rate = (label: string, column: string) => count(unhoused[column]) === null || count(others[column]) === null ? ''
        : ` ${label}: ${shown(count(unhoused[column])!)} of ${shown(u)} (${percent(count(unhoused[column])!, u)}) against ${shown(count(others[column])!)} of ${shown(o)} (${percent(count(others[column])!, o)}).`;
      const period = unhoused.first_stop && unhoused.last_stop ? `, ${String(unhoused.first_stop).slice(0, 10)} to ${String(unhoused.last_stop).slice(0, 10)}` : '';
      leads.push({ key: 'stops_unhoused_outcomes', priority: 100, query,
        title: `${percent(count(unhoused.arrested) ?? 0, u)} of people perceived as unhoused were arrested when stopped, against ${percent(count(others.arrested) ?? 0, o)} of others`,
        detail: `${shown(u)} of ${shown(u + o)} persons in the stop records${period} were perceived as unhoused.${rate('Custodial arrest', 'arrested')}${rate('Arrest on an outstanding warrant', 'arrested_on_warrant')}${rate('Arrest without a warrant', 'arrested_without_warrant')}${rate('Citation or cite and release', 'cited')}${rate('Property seized', 'property_seized')}${unrecorded ? ` ${shown(unrecorded)} persons have no perception recorded and are left out.` : ''} These rates are not adjusted for the reason for the stop or where it was made. ${UNADJUSTED}`,
        question: 'What were the outstanding warrants for? A records request for the warrant offences would show how many began as unpaid citations or missed court dates.',
      });
    } else if (query.id === 'stops_unhoused_reasons') {
      const reasons = rows.map((row) => ({ reason: String(row.reason), u: count(row.unhoused_persons) ?? 0, ua: count(row.unhoused_arrested) ?? 0, o: count(row.other_persons) ?? 0, oa: count(row.other_arrested) ?? 0 }));
      const compared = reasons.filter((row) => row.u >= GROUP_MINIMUM && row.o >= GROUP_MINIMUM);
      // Cross-multiplied so a rate exactly at the limit is not lost to division.
      const higher = compared.filter((row) => row.ua * row.o >= HIGHER_RATIO * row.oa * row.u && 100 * (row.ua * row.o - row.oa * row.u) >= HIGHER_POINTS * row.u * row.o);
      if (reasons[0]?.u >= GROUP_MINIMUM) leads.push({ key: 'stops_unhoused_reasons', priority: 97, query,
        title: `Most recorded reason for stopping people perceived as unhoused: ${reasons[0].reason} (${shown(reasons[0].u)} persons)`,
        detail: `${reasons.slice(0, 5).map((row) => `${row.reason}: ${shown(row.u)}.`).join(' ')} The reason is the publisher's code text; a person with two reasons is counted under each.`,
        question: 'Which ordinances do the local-ordinance codes stand for, and how many of these stops followed an offer of shelter?',
      });
      if (higher.length) leads.push({ key: 'stops_same_reason', priority: 99, query,
        title: `Stopped for the same recorded reason, people perceived as unhoused were arrested without a warrant more often in ${higher.length} of ${compared.length} reasons compared`,
        detail: `${higher.slice(0, 3).map((row) => `${row.reason}: ${shown(row.ua)} of ${shown(row.u)} (${percent(row.ua, row.u)}) against ${shown(row.oa)} of ${shown(row.o)} (${percent(row.oa, row.o)}).`).join(' ')} A reason is compared when at least ${GROUP_MINIMUM} persons of each group were stopped for it, and counted as higher at ${HIGHER_RATIO} times the others' rate and at least ${HIGHER_POINTS} percentage points. In the other ${compared.length - higher.length} the rate was not higher by that measure. Arrests on an outstanding warrant are left out. ${UNADJUSTED}`,
        question: 'For these reasons, what offence was each person arrested for, and where were the stops made?',
      });
    } else if (query.id === 'stops_unhoused_property') {
      const total = rows.reduce((sum, row) => sum + (count(row.persons) ?? 0), 0);
      if (total >= GROUP_MINIMUM) leads.push({ key: 'stops_unhoused_property', priority: 96, query,
        title: `Property was seized from people perceived as unhoused on ${shown(total)} recorded bases, ${shown(count(rows[0].persons) ?? 0)} as ${String(rows[0].basis).toLowerCase()}`,
        detail: `${rows.map((row) => `${String(row.basis)}: ${shown(count(row.persons) ?? 0)}.`).join(' ')} A person with two bases is counted under each. The table records that property was seized, not where it was kept, whether it was returned or whether it was discarded.`,
        question: 'Where was the property taken for safekeeping or as abandoned stored, and how much was returned? A records request for the property log would show it.',
      });
    }
  }
  return leads;
}
