import type { AnalyticalDatasetSummary, AnalyticalSchema, LeadFigure, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import { figureValue, type ServiceLead } from './serviceAccountability';
import { numberOrNull, wholeNumber as shown, percent } from './leadRows';

/** Arrest and citation logs as police departments release them to records requests: one row per
 * charge, several rows per arrest or citation. Found by the publisher's column names. */
const REQUIRED = ['activitynumber', 'activitytype', 'violationsection'];
/** A count is stated only with this many arrests and citations behind it. */
const ACTIVITY_MINIMUM = 30;
/** A race is named only with this many of the counted arrests and citations. */
const RACE_MINIMUM = 30;

/** Sections charged for sleeping, camping or keeping belongings in public, and for having no
 * restroom. Spreadsheets drop a trailing zero, so 54.0110 is also released as 54.011. */
const SECTIONS = [
  { name: 'encampment', label: 'San Diego Municipal Code 63.0404 (unauthorized encampment)', match: "section LIKE '63.0404%'" },
  { name: 'encroachment', label: 'Municipal Code 54.0110 (unauthorized encroachment)', match: "(section IN ('54.011', '54.0110') OR section LIKE '54.011(%' OR section LIKE '54.0110(%')" },
  { name: 'lodging', label: 'Penal Code 647(e) (lodging without consent)', match: "(section = '647E' OR section LIKE '647(E)%')" },
  { name: 'urination', label: 'Municipal Code 56.55 (urinating or defecating in public)', match: "section LIKE '56.55%'" },
] as const;

const has = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary, name: string) =>
  schema.columns.some((column) => column.tableName === dataset.tableName && column.columnName === name);
export const isChargeLog = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary) => REQUIRED.every((name) => has(schema, dataset, name));

const text = (name: string) => `UPPER(TRIM(CAST(${quoteIdentifier(name)} AS VARCHAR)))`;

export function chargeQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  if (!isChargeLog(schema, dataset)) return [];
  const dated = has(schema, dataset, 'activitydate');
  const raced = has(schema, dataset, 'race');
  const activities = `charges AS (
 SELECT ${text('activitynumber')} AS activity, ${text('activitytype')} AS activity_type, REPLACE(${text('violationsection')}, ' ', '') AS section${dated ? `,
  CAST(TRY_CAST(CAST(${quoteIdentifier('activitydate')} AS VARCHAR) AS TIMESTAMP) AS DATE) AS activity_date` : ''}${raced ? `, NULLIF(NULLIF(${text('race')}, ''), 'NULL') AS race` : ''}
 FROM ${quoteIdentifier(dataset.tableName)} WHERE ${quoteIdentifier('activitynumber')} IS NOT NULL
), activities AS (
 SELECT activity, MIN(activity_type) AS activity_type, COUNT(*) AS charges,
${SECTIONS.map((item) => `  COUNT(*) FILTER (WHERE ${item.match}) AS ${item.name}`).join(',\n')}${dated ? ',\n  MIN(activity_date) AS activity_date' : ''}${raced ? ',\n  MIN(race) AS race' : ''}
 FROM charges GROUP BY activity
), counted AS (
 SELECT *, ${SECTIONS.map((item) => item.name).join(' + ')} AS these FROM activities
)`;
  const method = `Rows in ${dataset.displayName} are charges; rows sharing an activity number are one arrest or citation. An arrest or citation is counted when any of its rows cites ${SECTIONS.map((item) => item.label).join(', ')}. Sections are compared in upper case with spaces removed; 54.011 is read as 54.0110 because spreadsheets drop a trailing zero. The file records the charge, not whether the person had housing.`;
  const queries: DiscoveryQuery[] = [{
    id: 'charges_living_outdoors', title: 'Arrests and citations under encampment, encroachment, lodging and public urination sections', method,
    sql: `WITH ${activities}
SELECT COUNT(*) AS activities, COUNT(*) FILTER (WHERE these > 0) AS counted_activities,
 COUNT(*) FILTER (WHERE these > 0 AND activity_type = 'ARREST') AS arrests, COUNT(*) FILTER (WHERE these > 0 AND activity_type = 'CITATION') AS citations,
 COUNT(*) FILTER (WHERE these > 0 AND these = charges AND activity_type = 'ARREST') AS arrests_on_these_alone,
${SECTIONS.map((item) => ` COUNT(*) FILTER (WHERE ${item.name} > 0) AS ${item.name}`).join(',\n')}${dated ? `,
 CAST(MIN(activity_date) AS VARCHAR) AS first_date, CAST(MAX(activity_date) AS VARCHAR) AS last_date` : ''}
FROM counted`,
  }];
  if (raced) queries.push({
    id: 'charges_living_outdoors_race', title: 'Recorded race in those arrests and citations, beside every arrest and citation in the file',
    method: `${method} Race is the department's entry for each arrest or citation; entries that are blank are left out of both columns. The comparison is with the other arrests and citations in the same file, not with the population.`,
    sql: `WITH ${activities}
SELECT race, COUNT(*) FILTER (WHERE these > 0) AS counted_activities, COUNT(*) AS activities,
 SUM(COUNT(*) FILTER (WHERE these > 0)) OVER () AS all_counted_activities, SUM(COUNT(*)) OVER () AS all_activities
FROM counted WHERE race IS NOT NULL GROUP BY race ORDER BY counted_activities DESC, race LIMIT 12`,
  });
  return queries;
}

const count = (value: unknown): number => numberOrNull(value) ?? 0;
const titled = (value: unknown) => String(value).toLowerCase().replace(/(^|[\s-])[a-z]/g, (letter) => letter.toUpperCase());

export function chargeLeads(results: { query: DiscoveryQuery; result?: SqlQueryResult }[]): ServiceLead[] {
  const entry = results.find((item) => item.query.id === 'charges_living_outdoors');
  if (!entry?.result?.rows.length) return [];
  const row = Object.fromEntries(entry.result.columns.map((name, index) => [name, entry.result!.rows[0][index]]));
  const counted = count(row.counted_activities);
  if (counted < ACTIVITY_MINIMUM) return [];
  const all = count(row.activities);
  const arrests = count(row.arrests); const alone = count(row.arrests_on_these_alone);
  const period = row.first_date && row.last_date ? `, dated ${String(row.first_date)} to ${String(row.last_date)}` : '';
  const sections = SECTIONS.filter((item) => count(row[item.name]) > 0).map((item) => `${item.label}: ${shown(count(row[item.name]))}`).join('. ');
  const races = results.find((item) => item.query.id === 'charges_living_outdoors_race')?.result;
  const raceRows = (races?.rows ?? []).map((values) => Object.fromEntries(races!.columns.map((name, index) => [name, values[index]])));
  // A file holding only these sections has nothing else to compare the recorded races with.
  const compared = raceRows.filter((item) => count(item.counted_activities) >= RACE_MINIMUM && count(item.all_activities) > count(item.all_counted_activities)).slice(0, 3)
    .map((item) => `${titled(item.race)} ${percent(count(item.counted_activities), count(item.all_counted_activities))} and ${percent(count(item.activities), count(item.all_activities))}`);
  const comparedRows = raceRows.filter((item) => count(item.counted_activities) >= RACE_MINIMUM && count(item.all_activities) > count(item.all_counted_activities));
  const figure: LeadFigure | undefined = entry.result.truncated ? undefined : comparedRows.length >= 2 && !races?.truncated
    ? { kind: 'paired', display: 'dots', measure: 'Recorded race: share of these arrests and citations against every one in the file', claim: 'statistical', unit: 'percent',
      subjectLabel: 'These sections', comparisonLabel: 'Every arrest and citation', band: null,
      caption: 'Shares of recorded race, blank entries set aside; a difference in share is not a finding about any arrest.', notes: [],
      rows: comparedRows.map((item) => ({ key: String(item.race), label: titled(item.race),
        subject: figureValue(count(item.counted_activities) / count(item.all_counted_activities), ''), comparison: figureValue(count(item.activities) / count(item.all_activities), ''),
        subjectBasis: `${shown(count(item.counted_activities))} of ${shown(count(item.all_counted_activities))}`, comparisonBasis: `${shown(count(item.activities))} of ${shown(count(item.all_activities))}` })) }
    : { kind: 'ranked', measure: 'Arrests and citations citing each section', claim: 'descriptive', unit: 'count', excluded: [],
      caption: 'One arrest or citation can cite more than one section.', notes: [],
      rows: SECTIONS.filter((item) => count(row[item.name]) > 0).map((item, index) => ({ key: item.name, label: item.label, value: figureValue(count(row[item.name]), ''), subject: index === 0, detail: null })) };
  return [{ key: 'charges_living_outdoors', priority: 96, query: entry.query, ...(figure ? { figure } : {}),
    title: alone > 0
      ? `${shown(counted)} arrests and citations cite an encampment, encroachment, lodging or public urination section; ${shown(alone)} were arrests on those sections alone`
      : `${shown(counted)} arrests and citations cite an encampment, encroachment, lodging or public urination section`,
    detail: `The file holds ${shown(all)} arrests and citations${period}. ${shown(counted)} cite at least one of these sections. ${sections}. One arrest or citation can cite more than one. ${shown(count(row.citations))} were citations and ${shown(arrests)} were arrests${arrests ? `; in ${shown(alone)} of the arrests no other section was charged` : ''}.${compared.length ? ` Recorded race, as a share of these and of every arrest and citation in the file (blank entries set aside): ${compared.join('; ')}.` : ''} The file records the charge, not whether the person had housing: these sections are also charged against people who are housed, and encroachment covers objects left in the street as well as people. It does not record whether shelter was offered or available, where a restroom was open, or what became of the charge.${compared.length ? ' The race comparison is with other arrests and citations in the same file, is not adjusted for where officers were assigned, and is not a finding of unequal treatment.' : ''}`,
    question: 'For the arrests made on these sections alone, was shelter offered first, and were the charges filed, dismissed or diverted? The department\'s shelter-offer entries and the City Attorney\'s case outcomes for the same activity numbers would show it.',
  }];
}
