import type { AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import type { ServiceLead } from './serviceAccountability';

const ISSUED_PREFIX = 'https://seshat.datasd.org/development_permits/approvals_issued_';
const ISSUED_SUFFIX = '_datasd.csv';
/** The address the app fetches the City's "Development approvals issued in <year>" file from. */
export const issuedApprovals = (year: number) => `${ISSUED_PREFIX}${year}${ISSUED_SUFFIX}`;

const TIERS = [
  { key: 'extremely_low', column: 'approval_du_extremely_low', name: 'extremely low' },
  { key: 'very_low', column: 'approval_du_very_low', name: 'very low' },
  { key: 'low', column: 'approval_du_low', name: 'low' },
  { key: 'moderate', column: 'approval_du_moderate', name: 'moderate' },
  { key: 'above_moderate', column: 'approval_du_above_moderate', name: 'above moderate' },
] as const;
const APART = [{ key: 'bonus', column: 'approval_du_bonus' }, { key: 'adu', column: 'approval_adu_total' }] as const;
const COLUMNS = ['job_id', 'approval_type', 'approval_issue_date', ...TIERS.map((tier) => tier.column), ...APART.map((item) => item.column)];
/** The approval types that permit construction, as the publisher writes them. Discretionary
 * approvals (Site Development, Coastal Development) repeat the units of the job they belong to. */
const PERMIT_TYPES = ['Building Permit', 'Combination Building Permit'];
/** A year is described only with this many jobs that have an income tier recorded. */
const JOB_MINIMUM = 30;

const has = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary) =>
  COLUMNS.every((name) => schema.columns.some((column) => column.tableName === dataset.tableName && column.columnName === name));
const latestFirst = (a: AnalyticalDatasetSummary, b: AnalyticalDatasetSummary) => b.retrievedAt.localeCompare(a.retrievedAt) || a.snapshotId.localeCompare(b.snapshotId);

/** The created, issued and closed files share one header, so the issued file is told apart only
 * by the address it was fetched from. */
function issuedYear(uri: string | null): number | null {
  if (!uri?.startsWith(ISSUED_PREFIX) || !uri.endsWith(ISSUED_SUFFIX)) return null;
  const text = uri.slice(ISSUED_PREFIX.length, uri.length - ISSUED_SUFFIX.length);
  const year = Number(text);
  return Number.isInteger(year) && String(year) === text ? year : null;
}

/** The latest copy of each imported issued-approvals year file, latest retrieved first. */
export function issuedApprovalTables(schema: AnalyticalSchema): AnalyticalDatasetSummary[] {
  const latest = new Map<string, AnalyticalDatasetSummary>();
  for (const table of schema.datasets.filter((dataset) => issuedYear(dataset.sourceUri) !== null && has(schema, dataset)).sort(latestFirst)) {
    if (!latest.has(table.sourceUri!)) latest.set(table.sourceUri!, table);
  }
  return [...latest.values()];
}

/** Every issued year file, or null unless this is the latest-retrieved one, so the lead is raised once. */
export function housingCompanions(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): AnalyticalDatasetSummary[] | null {
  const tables = issuedApprovalTables(schema);
  return tables[0]?.snapshotId === dataset.snapshotId ? tables : null;
}

const number = (name: string) => `TRY_CAST(TRIM(CAST(${quoteIdentifier(name)} AS VARCHAR)) AS DOUBLE)`;
const all = [...TIERS, ...APART];

export function housingQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const tables = housingCompanions(schema, dataset);
  if (!tables) return [];
  return [{
    id: 'housing_units_by_tier', title: 'Dwelling units on building permits issued, by income tier and issue year',
    method: `Building Permit and Combination Building Permit approvals in ${tables.map((table) => table.displayName).join(', ')}, counted once per JOB_ID; approvals with no JOB_ID are counted apart and in no total. A job is counted in the year of its earliest APPROVAL_ISSUE_DATE, so a job whose permits span two issue years is counted in the first. A job with several building permits is counted once, at the largest figure any of its permits records for each tier. Units are summed by income tier (APPROVAL_DU_EXTREMELY_LOW, _VERY_LOW, _LOW, _MODERATE, _ABOVE_MODERATE) as the permits record them; a blank tier is not a zero, and a tier no job in a year records has no total. Density-bonus units (APPROVAL_DU_BONUS) and ADUs (APPROVAL_ADU_TOTAL) are reported apart because the file does not say whether they are inside the tiers; the JADU columns are not read. APPROVAL_DU_NET_CHANGE is blank in the published file and is not read. Discretionary approvals such as Site Development and Coastal Development permits are left out because their units recur on the building permit. No target is applied.`,
    sql: `WITH permits AS (
${tables.map((table) => ` SELECT NULLIF(TRIM(CAST(${quoteIdentifier('job_id')} AS VARCHAR)), '') AS job, TRY_CAST(TRIM(CAST(${quoteIdentifier('approval_issue_date')} AS VARCHAR)) AS DATE) AS issued,
  ${all.map((item) => `${number(item.column)} AS ${item.key}`).join(', ')}
 FROM ${quoteIdentifier(table.tableName)} WHERE TRIM(CAST(${quoteIdentifier('approval_type')} AS VARCHAR)) IN (${PERMIT_TYPES.map((type) => `'${type}'`).join(', ')})`).join('\n UNION ALL\n')}
), jobs AS (
 SELECT job, YEAR(MIN(issued)) AS issued_year, COUNT(*) AS permits, ${all.map((item) => `MAX(${item.key}) AS ${item.key}`).join(', ')}
 FROM permits WHERE job IS NOT NULL GROUP BY job
)
SELECT issued_year, COUNT(*) AS jobs, COUNT(*) FILTER (WHERE ${TIERS.map((tier) => `${tier.key} IS NOT NULL`).join(' OR ')}) AS tiered_jobs,
 COUNT(*) FILTER (WHERE permits > 1) AS multi_permit_jobs, ${all.map((item) => `SUM(${item.key}) AS ${item.key}`).join(', ')},
 (SELECT COUNT(*) FROM permits WHERE job IS NULL) AS no_job_permits
FROM jobs GROUP BY issued_year ORDER BY issued_year NULLS LAST`,
  }];
}

const count = (value: unknown): number | null => {
  if (value == null || value === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};
const shown = (value: number) => Math.round(value).toLocaleString('en-US');
const units = (value: number) => `${shown(value)} unit${Math.round(value) === 1 ? '' : 's'}`;
const recorded = (value: number | null) => value === null ? 'not recorded' : shown(value);
const records = (result: SqlQueryResult) => result.rows.map((row) => Object.fromEntries(result.columns.map((name, i) => [name, row[i]])));
type Entry = { query: DiscoveryQuery; result?: SqlQueryResult };
/** Extremely-low plus very-low units. A tier no job recorded is named as not recorded, never added as 0. */
const lowest = (row: Record<string, unknown>) => {
  const eli = count(row.extremely_low); const vli = count(row.very_low);
  if (eli === null && vli === null) return null;
  return { sum: (eli ?? 0) + (vli ?? 0), households: eli === null ? 'very-low income households' : vli === null ? 'extremely-low income households' : 'extremely-low and very-low income households',
    missing: eli === null ? ' (no extremely-low tier recorded)' : vli === null ? ' (no very-low tier recorded)' : '' };
};
const CAVEAT = 'Units are as the permits record them: units on permits issued, not units built or occupied. A job with no tier recorded is in no tier total, which is not a count of zero.';

export function housingLeads(results: Entry[], currentYear = new Date().getUTCFullYear()): ServiceLead[] {
  const entry = results.find((item) => item.query.id === 'housing_units_by_tier');
  if (!entry?.result?.rows.length || entry.result.truncated) return [];
  const rows = records(entry.result);
  const years = rows.filter((row) => count(row.issued_year) !== null && (count(row.tiered_jobs) ?? 0) >= JOB_MINIMUM).sort((a, b) => count(a.issued_year)! - count(b.issued_year)!);
  const undated = rows.filter((row) => count(row.issued_year) === null).reduce((sum, row) => sum + (count(row.jobs) ?? 0), 0);
  const noJob = count(rows[0].no_job_permits) ?? 0;
  const soFar = (year: number) => year >= currentYear ? ' so far' : '';
  const side = years.length >= 2
    ? ` Extremely-low and very-low income units on building permits, by issue year: ${years.map((row) => `${count(row.issued_year)!}${soFar(count(row.issued_year)!)}: ${((lower) => lower ? `${shown(lower.sum)}${lower.missing}` : 'not recorded')(lowest(row))}`).join('; ')}.`
    : '';
  return years.map((row) => {
    const year = count(row.issued_year)!;
    const tiers = TIERS.map((tier) => ({ ...tier, value: count(row[tier.key]) }));
    const tiered = tiers.reduce((sum, tier) => sum + (tier.value ?? 0), 0);
    const lower = lowest(row);
    const jobs = count(row.jobs) ?? 0; const tieredJobs = count(row.tiered_jobs) ?? 0; const multi = count(row.multi_permit_jobs) ?? 0;
    return {
      key: `housing_units_by_tier:${year}`, priority: 95, query: entry.query, scope: { period: `${year}-01-01 to ${year}-12-31` },
      title: `Building permits issued in ${year}${soFar(year)} ${lower ? `carry ${units(lower.sum)} for ${lower.households}${lower.missing}` : 'record no extremely-low or very-low income tier'}, of ${units(tiered)} with an income tier`,
      detail: `Building and Combination Building Permits issued in ${year}${soFar(year)}, counted once per job: ${shown(jobs)} jobs, ${shown(tieredJobs)} with an income tier recorded and ${shown(jobs - tieredJobs)} with no tier recorded. Units by income tier: ${tiers.map((tier) => `${tier.name} ${recorded(tier.value)}`).join(', ')}. Reported apart, because the file does not say whether they are inside the tiers: density-bonus units ${recorded(count(row.bonus))}, ADUs ${recorded(count(row.adu))}.${multi ? ` ${shown(multi)} jobs hold more than one building permit; each is counted once, at the largest figure any of its permits records for each tier.` : ''}${side}${undated ? ` ${shown(undated)} jobs have no readable issue date and are in no year.` : ''}${noJob ? ` ${shown(noJob)} building permits have no job recorded and are in no total.` : ''} ${CAVEAT}`,
      question: `What does the City's Annual Progress Report table A2 for ${year} record as building permits issued by income tier, and what is the City's RHNA allocation for each tier? The report's table A2 and the allocation would show what the City reported for the same tiers.`,
    };
  });
}
