import type { AnalyticalColumnSummary, AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';

/** Columns of a service-request table (311-style reports), found by the publisher's own names. */
export interface ServiceRequestRoles {
  category: string;
  closed: string;
  opened: string | null;
  age: string | null;
  status: string | null;
  place: string | null;
  areas: string[];
}
export interface ServiceLead {
  key: string;
  priority: number;
  title: string;
  detail: string;
  question: string;
  query: DiscoveryQuery;
}

const roleNames = {
  category: ['service_name', 'service_type', 'request_type', 'complaint_type', 'case_type', 'problem_type', 'issue_type', 'category'],
  closed: ['date_closed', 'closed_date', 'closed_datetime', 'date_resolved', 'resolved_date', 'date_completed', 'completed_date'],
  opened: ['date_requested', 'requested_date', 'requested_datetime', 'date_opened', 'opened_date', 'open_date', 'date_created', 'created_date', 'date_reported', 'reported_date'],
  age: ['case_age_days', 'age_days', 'days_open'],
  status: ['status', 'case_status', 'request_status'],
  place: ['street_address', 'incident_address', 'address', 'location'],
};
const districtNames = ['council_district', 'district', 'supervisor_district', 'ward'];
const communityNames = ['comm_plan_name', 'community', 'community_plan', 'neighborhood', 'zipcode', 'zip_code', 'zip'];

/** One year is a fixed yardstick, not a statistical threshold. */
const STALE_DAYS = 365;
/** An area is named only with this many reports behind its median. */
const AREA_MINIMUM_REPORTS = 30;
/** An area is named when its median is at least this multiple of the median across all areas. */
const AREA_MEDIAN_RATIO = 1.5;
const REPEAT_PLACE_REPORTS = 10;
const REPEAT_PLACE_DAYS = 90;
const REFERRED_MINIMUM_REPORTS = 100;
/** A slower area is called recurring when this many imported tables list it. */
const RECURRING_TABLES = 3;

export function serviceRequestRoles(columns: AnalyticalColumnSummary[]): ServiceRequestRoles | null {
  const names = new Set(columns.map((column) => column.columnName));
  const first = (candidates: string[]) => candidates.find((name) => names.has(name)) ?? null;
  const category = first(roleNames.category);
  const closed = first(roleNames.closed);
  const opened = first(roleNames.opened);
  const age = first(roleNames.age);
  if (!category || !closed || (!opened && !age)) return null;
  return { category, closed, opened, age, status: first(roleNames.status), place: first(roleNames.place),
    areas: [first(districtNames), first(communityNames)].filter((name): name is string => name !== null) };
}

const text = (name: string) => `NULLIF(TRIM(CAST(${quoteIdentifier(name)} AS VARCHAR)), '')`;
const instant = (name: string) => `TRY_CAST(CAST(${quoteIdentifier(name)} AS VARCHAR) AS TIMESTAMP)`;

export function serviceAccountabilityQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const roles = serviceRequestRoles(schema.columns.filter((column) => column.tableName === dataset.tableName));
  if (!roles) return [];
  const closedAt = text(roles.closed);
  // A referred report often has no close date; it left this table rather than staying open in it.
  const referred = roles.status ? `COALESCE(CONTAINS(LOWER(CAST(${quoteIdentifier(roles.status)} AS VARCHAR)), 'refer'), FALSE)` : 'FALSE';
  const retrieved = `CAST('${dataset.retrievedAt.slice(0, 10).replace(/[^0-9-]/g, '')}' AS TIMESTAMP)`;
  const days = roles.age ? `TRY_CAST(${quoteIdentifier(roles.age)} AS DOUBLE)`
    : `CAST(DATE_DIFF('day', ${instant(roles.opened!)}, COALESCE(${instant(roles.closed)}, ${retrieved})) AS DOUBLE)`;
  // Publishers mark an unknown age with a negative sentinel (-999); that is absence, not an age.
  const ageBasis = roles.age ? `Age is the publisher's ${roles.age} value as of this export; a negative value is treated as not recorded.`
    : `Age is counted from ${roles.opened} to ${roles.closed}, or to the retrieval date ${dataset.retrievedAt.slice(0, 10)} for a report with no close date.`;
  const scope = (area: string | null) => `WITH service_scope AS (
 SELECT ${text(roles.category)} AS category, ${area ? `UPPER(${text(area)})` : 'NULL'} AS area, ${roles.place ? text(roles.place) : 'NULL'} AS place,
  ${roles.opened ? instant(roles.opened) : 'NULL'} AS opened_at, ${closedAt} IS NULL AND NOT ${referred} AS unresolved,
  ${closedAt} IS NOT NULL AND NOT ${referred} AS resolved, ${referred} AS referred, CASE WHEN ${days} >= 0 THEN ${days} END AS age_days
 FROM ${quoteIdentifier(dataset.tableName)}
), service_reports AS (SELECT * FROM service_scope WHERE category IS NOT NULL)`;
  const queries: DiscoveryQuery[] = [{
    id: 'service_backlog', title: 'Reports still open, by service',
    method: `Reports with no ${roles.closed} value${roles.status ? ` and a ${roles.status} that is not a referral` : ''}, grouped by ${roles.category}. ${ageBasis} A report counts as open more than a year when its age exceeds ${STALE_DAYS} days. Totals cover every service, not only the rows shown.`,
    sql: `${scope(null)}, service_backlog AS (
 SELECT category, COUNT(*) AS open_reports, MEDIAN(age_days) AS median_age_days,
  COUNT(*) FILTER (WHERE age_days > ${STALE_DAYS}) AS open_over_one_year, MAX(age_days) AS oldest_age_days, CAST(MIN(opened_at) AS DATE) AS earliest_opened
 FROM service_reports WHERE unresolved GROUP BY category
)
SELECT *, SUM(open_reports) OVER () AS all_open_reports, SUM(open_over_one_year) OVER () AS all_open_over_one_year
FROM service_backlog ORDER BY open_over_one_year DESC, open_reports DESC, category`,
  }];
  const byArea = (area: string, rows: string, counted: string) => `${scope(area)}, service_area AS (
 SELECT category, area, COUNT(*) AS ${counted}, MEDIAN(age_days) AS median_days, COUNT(*) FILTER (WHERE age_days > ${STALE_DAYS}) AS over_one_year
 FROM service_reports WHERE ${rows} AND area IS NOT NULL AND age_days IS NOT NULL GROUP BY category, area
), service_all_areas AS (
 SELECT category, COUNT(DISTINCT area) AS areas, MEDIAN(age_days) AS all_median_days
 FROM service_reports WHERE ${rows} AND area IS NOT NULL AND age_days IS NOT NULL GROUP BY category
)
SELECT service_area.*, service_all_areas.all_median_days, ROUND(median_days / all_median_days, 2) AS times_all_areas
FROM service_area JOIN service_all_areas ON service_area.category = service_all_areas.category`;
  for (const area of roles.areas) {
    queries.push({
      id: `service_backlog_area:${area}`, title: `Open reports older in some areas (${area})`,
      method: `Median age of open reports per ${roles.category} and ${area}, against the median for that service across all areas. An area is listed with at least ${AREA_MINIMUM_REPORTS} open reports, at least three areas reporting, and a median at least ${AREA_MEDIAN_RATIO} times the all-area median. ${ageBasis}`,
      sql: `${byArea(area, 'unresolved', 'open_reports')}
WHERE open_reports >= ${AREA_MINIMUM_REPORTS} AND areas >= 3 AND all_median_days >= 30 AND median_days >= ${AREA_MEDIAN_RATIO} * all_median_days
ORDER BY over_one_year DESC, open_reports DESC, service_area.category, area`,
    }, {
      id: `service_wait_area:${area}`, title: `Slower closure in some areas (${area})`,
      method: `Median days to close per ${roles.category} and ${area}, for reports with a ${roles.closed} value that were not referred, against the median for that service across all areas. An area is listed with at least ${AREA_MINIMUM_REPORTS} closed reports, at least three areas reporting, a median at least ${AREA_MEDIAN_RATIO} times the all-area median, and at least seven days slower. ${ageBasis}`,
      sql: `${byArea(area, 'resolved', 'closed_reports')}
WHERE closed_reports >= ${AREA_MINIMUM_REPORTS} AND areas >= 3 AND all_median_days >= 1 AND median_days >= ${AREA_MEDIAN_RATIO} * all_median_days AND median_days - all_median_days >= 7
ORDER BY closed_reports * (median_days - all_median_days) DESC, service_area.category, area`,
    });
  }
  if (roles.place) queries.push({
    id: 'service_repeat_places', title: 'Places reported again and again',
    method: `Open reports sharing the same ${roles.category} and the same ${roles.place} text, where at least ${REPEAT_PLACE_REPORTS} are open and the oldest has been open more than ${REPEAT_PLACE_DAYS} days. The place is the text the publisher recorded; a street name without a number can cover many blocks, and several reports can describe one fault.`,
    sql: `${scope(null)}
SELECT category, place, COUNT(*) AS open_reports, CAST(MIN(opened_at) AS DATE) AS earliest_opened, MAX(age_days) AS oldest_age_days
FROM service_reports WHERE unresolved AND place IS NOT NULL GROUP BY category, place
HAVING COUNT(*) >= ${REPEAT_PLACE_REPORTS} AND MAX(age_days) > ${REPEAT_PLACE_DAYS} ORDER BY open_reports DESC, category, place`,
  });
  if (roles.status) queries.push({
    id: 'service_referred', title: 'Reports referred elsewhere',
    method: `Reports whose ${roles.status} names a referral, per ${roles.category}. A service is listed with at least ${REFERRED_MINIMUM_REPORTS} referred reports making up at least a quarter of its reports. Totals cover every service.`,
    sql: `${scope(null)}, service_referrals AS (
 SELECT category, COUNT(*) AS reports, COUNT(*) FILTER (WHERE referred) AS referred_reports
 FROM service_reports GROUP BY category
), service_referral_totals AS (
 SELECT *, ROUND(100.0 * referred_reports / reports, 1) AS referred_percent,
  SUM(reports) OVER () AS all_reports, SUM(referred_reports) OVER () AS all_referred_reports
 FROM service_referrals
)
SELECT * FROM service_referral_totals WHERE referred_reports > 0 ORDER BY referred_reports DESC, category`,
  });
  return queries;
}

const count = (value: unknown): number | null => {
  if (value == null || value === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};
const shown = (value: number) => Math.round(value).toLocaleString('en-US');
const records = (result: SqlQueryResult) => result.rows.map((row) => Object.fromEntries(result.columns.map((name, i) => [name, row[i]])));
const areaLabel = (column: string, value: unknown) => /district|ward/.test(column) && /^\d+$/.test(String(value)) ? `${column.replace(/_/g, ' ')} ${String(value)}` : String(value);

export function serviceAccountabilityLeads(results: { query: DiscoveryQuery; result?: SqlQueryResult }[]): ServiceLead[] {
  const leads: ServiceLead[] = [];
  for (const { query, result } of results) {
    if (!result?.rows.length) continue;
    const rows = records(result);
    const [id, area] = query.id.split(':');
    if (id === 'service_backlog') {
      const open = count(rows[0].all_open_reports) ?? 0;
      const stale = count(rows[0].all_open_over_one_year) ?? 0;
      if (stale < 25) continue;
      const worst = rows.filter((row) => (count(row.open_over_one_year) ?? 0) > 0).slice(0, 3).map((row) =>
        `${String(row.category)}: ${shown(count(row.open_over_one_year)!)} of ${shown(count(row.open_reports) ?? 0)} open reports are older than a year${row.earliest_opened ? `, the earliest filed ${String(row.earliest_opened).slice(0, 10)}` : ''}.`);
      leads.push({ key: 'service_backlog', priority: 98, query,
        title: `${shown(stale)} of ${shown(open)} open reports have been open more than a year`,
        detail: `${worst.join(' ')} An open report is not proof the problem persists: the table records no inspections or work done.`,
        question: `Which department owns ${String(rows[0].category)}, and what do its budget and staffing say about this backlog?`,
      });
    } else if (id === 'service_backlog_area' || id === 'service_wait_area') {
      const open = id === 'service_backlog_area';
      rows.slice(0, 2).forEach((row, index) => {
        const median = count(row.median_days); const overall = count(row.all_median_days);
        if (median === null || overall === null) return;
        const reports = shown(count(open ? row.open_reports : row.closed_reports) ?? 0);
        leads.push({ key: `${query.id}:${index}`, priority: (open ? 95 : 94) - index, query,
          title: open
            ? `${String(row.category)}: open reports in ${areaLabel(area, row.area)} are a median ${shown(median)} days old, against ${shown(overall)} across all areas`
            : `${String(row.category)}: reports in ${areaLabel(area, row.area)} took a median ${shown(median)} days to close, against ${shown(overall)} across all areas`,
          detail: `${reports} ${open ? 'open' : 'closed'} reports in this area${open ? `, ${shown(count(row.over_one_year) ?? 0)} of them older than a year` : ''}. Area is the publisher's ${area} field, compared without regard to letter case. The difference can reflect what was reported as well as how it was handled; the table records no crew assignments or funding.`,
          question: `What does the responsible department say explains the difference in ${areaLabel(area, row.area)}, and is there a published schedule for this work?`,
        });
      });
    } else if (id === 'service_repeat_places') {
      rows.slice(0, 2).forEach((row, index) => leads.push({ key: `service_repeat_places:${index}`, priority: 96 - index, query,
        title: `${shown(count(row.open_reports) ?? 0)} open ${String(row.category)} reports name ${String(row.place)}${row.earliest_opened ? `, the earliest from ${String(row.earliest_opened).slice(0, 10)}` : ''}`,
        detail: 'Each is a separate report still open at the same recorded place. The place is the publisher’s text: a street name without a number can cover many blocks, and several reports can describe one fault.',
        question: 'Has the City inspected this place, and what did it tell the people who reported it?',
      }));
    } else if (id === 'service_referred') {
      const all = count(rows[0].all_reports) ?? 0;
      const referred = count(rows[0].all_referred_reports) ?? 0;
      const heavy = rows.filter((row) => (count(row.referred_reports) ?? 0) >= REFERRED_MINIMUM_REPORTS && 4 * (count(row.referred_reports) ?? 0) >= (count(row.reports) ?? 0));
      if (!heavy.length) continue;
      leads.push({ key: 'service_referred', priority: 90, query,
        title: `${shown(referred)} of ${shown(all)} reports were referred elsewhere, with no outcome recorded here`,
        detail: `${heavy.slice(0, 3).map((row) => `${String(row.category)}: ${shown(count(row.referred_reports)!)} of ${shown(count(row.reports) ?? 0)} referred.`).join(' ')} A referred report leaves this table; whether the receiving office acted is not recorded.`,
        question: 'Which offices received these reports, and do they publish what happened to them? A records request for the referral log would show it.',
      });
    }
  }
  return leads;
}

/** The same service and area listed as slower to close in several imported tables (for example
 * one table per year). One table's difference can be chance; the same one recurring is a pattern. */
export function recurringServiceLeads(tables: { snapshotId: string; displayName: string; results: { query: DiscoveryQuery; result?: SqlQueryResult }[] }[]): (ServiceLead & { snapshotId: string })[] {
  const seen = new Map<string, { area: string; category: string; place: string; hits: { snapshotId: string; displayName: string; median: number; overall: number; reports: number; query: DiscoveryQuery }[] }>();
  for (const table of tables) for (const { query, result } of table.results) {
    const [id, area] = query.id.split(':');
    if (id !== 'service_wait_area' || !result) continue;
    for (const row of records(result)) {
      const median = count(row.median_days); const overall = count(row.all_median_days);
      if (median === null || overall === null) continue;
      const key = JSON.stringify([area, row.category, row.area]);
      const entry = seen.get(key) ?? { area, category: String(row.category), place: areaLabel(area, row.area), hits: [] };
      entry.hits.push({ snapshotId: table.snapshotId, displayName: table.displayName, median, overall, reports: count(row.closed_reports) ?? 0, query });
      seen.set(key, entry);
    }
  }
  return [...seen.entries()].filter(([, entry]) => entry.hits.length >= RECURRING_TABLES)
    .sort(([a, x], [b, y]) => y.hits.length - x.hits.length || y.hits.reduce((sum, hit) => sum + hit.reports, 0) - x.hits.reduce((sum, hit) => sum + hit.reports, 0) || a.localeCompare(b))
    .slice(0, 3).map(([key, entry], index) => ({
      snapshotId: entry.hits[0].snapshotId, key: `service_recurring:${key}`, priority: 99 - index, query: entry.hits[0].query,
      title: `${entry.category} reports in ${entry.place} closed slower than the all-area median in ${entry.hits.length} imported tables`,
      detail: `${entry.hits.map((hit) => `${hit.displayName}: median ${shown(hit.median)} days against ${shown(hit.overall)} (${shown(hit.reports)} reports).`).join(' ')} Tables are compared as imported and can overlap in the reports they hold. The difference can reflect what was reported as well as how it was handled.`,
      question: `Has the responsible department explained why ${entry.category} takes longer in ${entry.place}, and has it been raised with the council office?`,
    }));
}

/** The same published source imported on two dates: how its open backlog moved between them. */
export function backlogChangeLeads(tables: { snapshotId: string; sourceUri: string | null; retrievedAt: string; results: { query: DiscoveryQuery; result?: SqlQueryResult }[] }[]): (ServiceLead & { snapshotId: string })[] {
  const bySource = new Map<string, { snapshotId: string; day: string; query: DiscoveryQuery; rows: Record<string, unknown>[] }[]>();
  for (const table of tables) {
    const backlog = table.results.find((entry) => entry.query.id === 'service_backlog');
    if (!table.sourceUri || !backlog?.result?.rows.length) continue;
    bySource.set(table.sourceUri, [...(bySource.get(table.sourceUri) ?? []), { snapshotId: table.snapshotId, day: table.retrievedAt.slice(0, 10), query: backlog.query, rows: records(backlog.result) }]);
  }
  const leads: (ServiceLead & { snapshotId: string })[] = [];
  for (const snapshots of bySource.values()) {
    const ordered = [...snapshots].sort((a, b) => a.day.localeCompare(b.day));
    const earlier = ordered[0]; const later = ordered[ordered.length - 1];
    if (earlier.day === later.day) continue;
    const before = count(earlier.rows[0].all_open_over_one_year); const after = count(later.rows[0].all_open_over_one_year);
    const openBefore = count(earlier.rows[0].all_open_reports); const openAfter = count(later.rows[0].all_open_reports);
    if (before === null || after === null || openBefore === null || openAfter === null || before === after) continue;
    const prior = new Map(earlier.rows.map((row) => [String(row.category), count(row.open_over_one_year) ?? 0]));
    const moved = later.rows.map((row) => ({ category: String(row.category), now: count(row.open_over_one_year) ?? 0, then: prior.get(String(row.category)) }))
      .filter((row): row is { category: string; now: number; then: number } => row.then !== undefined && row.then !== row.now)
      .sort((a, b) => Math.abs(b.now - b.then) - Math.abs(a.now - a.then) || a.category.localeCompare(b.category)).slice(0, 3);
    leads.push({ snapshotId: later.snapshotId, key: `service_backlog_change:${earlier.snapshotId}`, priority: 99, query: later.query,
      title: `Open reports older than a year ${after > before ? 'rose' : 'fell'} from ${shown(before)} to ${shown(after)} between ${earlier.day} and ${later.day}`,
      detail: `All open reports went from ${shown(openBefore)} to ${shown(openAfter)}. ${moved.map((row) => `${row.category}: ${shown(row.then)} to ${shown(row.now)}.`).join(' ')} The two dates are when each copy was retrieved. A report leaves the open file when it is closed or referred; this comparison does not say which.`,
      question: 'Which reports left the open file between these dates, and were they repaired or closed without work?',
    });
  }
  return leads;
}
