import type { AnalyticalColumnSummary, AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import { numberOrNull as count, wholeNumber as shown, records, money } from './leadRows';

/** Columns of a service-request table (311-style reports), found by the publisher's own names. */
export interface ServiceRequestRoles {
  category: string;
  closed: string;
  opened: string | null;
  age: string | null;
  status: string | null;
  place: string | null;
  id: string | null;
  /** The publisher's name for the staff group that handles a report. */
  group: string | null;
  areas: string[];
}
/** What a lead's own result says a follow-up record should cover. */
export interface LeadScope { period?: string; identifiers?: string[] }
export interface ServiceLead {
  key: string;
  scope?: LeadScope;
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
  id: ['service_request_id', 'request_id', 'case_id', 'case_number', 'sr_number', 'ticket_id'],
  group: ['case_record_type', 'agency_responsible', 'responsible_department'],
};
/** Get It Done staff groups read as operating-budget departments. Only "Parks & Recreation" is
 * the budget's own spelling; the rest expand the City's abbreviations, so a lead states the
 * assignment as this app's reading. Groups left out (Parking, Traffic Engineering) are not
 * assigned. Department names verified against budget_operating_datasd.csv, 2026-10-04. */
const STAFF_GROUP_DEPARTMENTS: Record<string, string[]> = {
  'tsw': ['Transportation', 'Stormwater'],
  'tsw row': ['Transportation', 'Stormwater'],
  'storm water code enforcement': ['Stormwater'],
  'dsd': ['Development Services'],
  'esd complaint/report': ['Environmental Services'],
  'parks & recreation': ['Parks & Recreation'],
  'neighborhood policing': ['Police'],
};
export const staffGroupDepartments = (group: unknown): string[] | null => typeof group === 'string' ? STAFF_GROUP_DEPARTMENTS[group.trim().toLowerCase()] ?? null : null;
const BUDGET_COLUMNS = ['dept_name', 'report_fy', 'budget_cycle', 'amount', 'account_number'];
/** The latest imported table with the operating budget's columns. */
export function budgetTable(schema: AnalyticalSchema): AnalyticalDatasetSummary | null {
  return schema.datasets.filter((dataset) => BUDGET_COLUMNS.every((name) => schema.columns.some((column) => column.tableName === dataset.tableName && column.columnName === name)))
    .sort((a, b) => b.retrievedAt.localeCompare(a.retrievedAt) || a.snapshotId.localeCompare(b.snapshotId))[0] ?? null;
}
const districtNames = ['council_district', 'district', 'supervisor_district', 'ward'];
const communityNames = ['comm_plan_name', 'community', 'community_plan', 'neighborhood', 'zipcode', 'zip_code', 'zip'];

/** One year is a fixed yardstick, not a statistical threshold. */
const STALE_DAYS = 365;
/** An area is named only with this many reports behind its median. */
const AREA_MINIMUM_REPORTS = 30;
/** An area is named when its median is at least this multiple of the median across all areas. */
const AREA_MEDIAN_RATIO = 1.5;
/** Services the owner follows; their area rows are shown even when larger services outrank them. */
const FOLLOWED_SERVICES = /encampment|homeless|shelter|restroom/i;
const REPEAT_PLACE_REPORTS = 10;
const REPEAT_PLACE_DAYS = 90;
const REFERRED_MINIMUM_REPORTS = 100;
/** A slower area is called recurring when this many imported tables list it. */
const RECURRING_TABLES = 3;
/** Tables searched for what became of a report that left an open file. */
const OUTCOME_TABLES = 8;

export function serviceRequestRoles(columns: AnalyticalColumnSummary[]): ServiceRequestRoles | null {
  const names = new Set(columns.map((column) => column.columnName));
  const first = (candidates: string[]) => candidates.find((name) => names.has(name)) ?? null;
  const category = first(roleNames.category);
  const closed = first(roleNames.closed);
  const opened = first(roleNames.opened);
  const age = first(roleNames.age);
  if (!category || !closed || (!opened && !age)) return null;
  return { category, closed, opened, age, status: first(roleNames.status), place: first(roleNames.place), id: first(roleNames.id), group: first(roleNames.group),
    areas: [first(districtNames), first(communityNames)].filter((name): name is string => name !== null) };
}

const text = (name: string) => `NULLIF(TRIM(CAST(${quoteIdentifier(name)} AS VARCHAR)), '')`;
const instant = (name: string) => `TRY_CAST(CAST(${quoteIdentifier(name)} AS VARCHAR) AS TIMESTAMP)`;

const day = (dataset: AnalyticalDatasetSummary) => dataset.retrievedAt.slice(0, 10).replace(/[^0-9-]/g, '');
const rolesOf = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary) => serviceRequestRoles(schema.columns.filter((column) => column.tableName === dataset.tableName));
// Publishers mark an unknown age with a negative sentinel (-999); that is absence, not an age.
const ageBasis = (roles: ServiceRequestRoles, dataset: AnalyticalDatasetSummary) => roles.age ? `Age is the publisher's ${roles.age} value as of this export; a negative value is treated as not recorded.`
  : `Age is counted from ${roles.opened} to ${roles.closed}, or to the retrieval date ${day(dataset)} for a report with no close date.`;

/** One row per report, in the roles every service check reads. */
function reportRows(roles: ServiceRequestRoles, dataset: AnalyticalDatasetSummary, area: string | null): string {
  const closedAt = text(roles.closed);
  // A referred report often has no close date; it left this table rather than staying open in it.
  const referred = roles.status ? `COALESCE(CONTAINS(LOWER(CAST(${quoteIdentifier(roles.status)} AS VARCHAR)), 'refer'), FALSE)` : 'FALSE';
  const days = roles.age ? `TRY_CAST(${quoteIdentifier(roles.age)} AS DOUBLE)`
    : `CAST(DATE_DIFF('day', ${instant(roles.opened!)}, COALESCE(${instant(roles.closed)}, CAST('${day(dataset)}' AS TIMESTAMP))) AS DOUBLE)`;
  return `SELECT ${roles.id ? text(roles.id) : 'NULL'} AS report_id, ${text(roles.category)} AS category, ${area ? `UPPER(${text(area)})` : 'NULL'} AS area, ${roles.place ? text(roles.place) : 'NULL'} AS place, ${roles.group ? text(roles.group) : 'NULL'} AS staff_group,
  ${roles.opened ? instant(roles.opened) : 'NULL'} AS opened_at, ${closedAt} IS NULL AND NOT ${referred} AS unresolved,
  ${closedAt} IS NOT NULL AND NOT ${referred} AS resolved, ${referred} AS referred, CASE WHEN ${days} >= 0 THEN ${days} END AS age_days
 FROM ${quoteIdentifier(dataset.tableName)}`;
}

/** The tables a copy of a source is read against: the earliest earlier copy of the same source,
 * and other service-report tables retrieved since then that can record what became of a report. */
export function serviceCompanions(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): { earlier: AnalyticalDatasetSummary; outcomes: AnalyticalDatasetSummary[] } | null {
  if (!dataset.sourceUri || !rolesOf(schema, dataset)?.id) return null;
  const identified = schema.datasets.filter((other) => other.snapshotId !== dataset.snapshotId && rolesOf(schema, other)?.id);
  const earlier = identified.filter((other) => other.sourceUri === dataset.sourceUri && day(other) < day(dataset))
    .sort((a, b) => a.retrievedAt.localeCompare(b.retrievedAt) || a.snapshotId.localeCompare(b.snapshotId))[0];
  if (!earlier) return null;
  const outcomes = identified.filter((other) => other.sourceUri !== dataset.sourceUri && day(other) >= day(earlier))
    .sort((a, b) => b.retrievedAt.localeCompare(a.retrievedAt) || a.snapshotId.localeCompare(b.snapshotId)).slice(0, OUTCOME_TABLES);
  return { earlier, outcomes };
}

function movementQuery(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery | null {
  const companions = serviceCompanions(schema, dataset);
  if (!companions) return null;
  const rows = (table: AnalyticalDatasetSummary) => reportRows(rolesOf(schema, table)!, table, null);
  const { earlier, outcomes } = companions;
  return {
    id: 'service_movement', title: 'Reports that left the open file, and new ones',
    method: `Reports open in the copy retrieved ${day(earlier)} that are not open in the copy retrieved ${day(dataset)}, and reports open in the later copy that were not open in the earlier one, matched on the publisher's ${rolesOf(schema, dataset)!.id}. What became of a report that left is read from the later copy${outcomes.length ? ` and ${outcomes.map((table) => table.displayName).join(', ')}` : ''}: a close date, a referral, or no record in any of them. A close date records that a report was closed, not what work was done.`,
    sql: `WITH earlier_reports AS (
 ${rows(earlier)}
), later_reports AS (
 ${rows(dataset)}
), earlier_open AS (SELECT * FROM earlier_reports WHERE unresolved AND category IS NOT NULL AND report_id IS NOT NULL
), later_open AS (SELECT * FROM later_reports WHERE unresolved AND category IS NOT NULL AND report_id IS NOT NULL
), outcome_rows AS (
 SELECT report_id, resolved, referred FROM later_reports${outcomes.map((table) => `
 UNION ALL SELECT report_id, resolved, referred FROM (
 ${rows(table)}
 )`).join('')}
), outcome_records AS (
 SELECT report_id, BOOL_OR(resolved) AS resolved, BOOL_OR(referred) AS referred FROM outcome_rows WHERE report_id IS NOT NULL AND (resolved OR referred) GROUP BY report_id
), service_left AS (
 SELECT category, COUNT(*) AS left_reports, COUNT(*) FILTER (WHERE age_days > ${STALE_DAYS}) AS left_over_one_year,
  COUNT(*) FILTER (WHERE outcome_records.resolved) AS closed_on_record,
  COUNT(*) FILTER (WHERE outcome_records.referred AND NOT outcome_records.resolved) AS referred_on_record,
  COUNT(*) FILTER (WHERE outcome_records.report_id IS NULL) AS no_outcome_record
 FROM earlier_open LEFT JOIN outcome_records ON outcome_records.report_id = earlier_open.report_id
 WHERE NOT EXISTS (SELECT 1 FROM later_open WHERE later_open.report_id = earlier_open.report_id) GROUP BY category
), service_arrived AS (
 SELECT category, COUNT(*) AS arrived_reports FROM later_open
 WHERE NOT EXISTS (SELECT 1 FROM earlier_open WHERE earlier_open.report_id = later_open.report_id) GROUP BY category
), service_movement AS (
 SELECT COALESCE(service_left.category, service_arrived.category) AS category, COALESCE(left_reports, 0) AS left_reports,
  COALESCE(left_over_one_year, 0) AS left_over_one_year, COALESCE(closed_on_record, 0) AS closed_on_record,
  COALESCE(referred_on_record, 0) AS referred_on_record, COALESCE(no_outcome_record, 0) AS no_outcome_record, COALESCE(arrived_reports, 0) AS arrived_reports
 FROM service_left FULL JOIN service_arrived ON service_left.category = service_arrived.category
)
SELECT *, SUM(left_reports) OVER () AS all_left_reports, SUM(left_over_one_year) OVER () AS all_left_over_one_year,
 SUM(closed_on_record) OVER () AS all_closed_on_record, SUM(referred_on_record) OVER () AS all_referred_on_record,
 SUM(no_outcome_record) OVER () AS all_no_outcome_record, SUM(arrived_reports) OVER () AS all_arrived_reports, ${outcomes.length} AS outcome_tables
FROM service_movement ORDER BY left_reports DESC, arrived_reports DESC, category`,
  };
}

export function serviceAccountabilityQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const roles = rolesOf(schema, dataset);
  if (!roles) return [];
  const basis = ageBasis(roles, dataset);
  const scope = (area: string | null) => `WITH service_scope AS (
 ${reportRows(roles, dataset, area)}
), service_reports AS (SELECT * FROM service_scope WHERE category IS NOT NULL)`;
  const queries: DiscoveryQuery[] = [{
    id: 'service_backlog', title: 'Reports still open, by service',
    method: `Reports with no ${roles.closed} value${roles.status ? ` and a ${roles.status} that is not a referral` : ''}, grouped by ${roles.category}. ${basis} A report counts as open more than a year when its age exceeds ${STALE_DAYS} days. Totals cover every service, not only the rows shown.${roles.group ? ` The staff group is the publisher's ${roles.group} value, the one most open reports of a service carry, with how many carry it.` : ''}`,
    sql: `${scope(null)}, service_open AS (
 SELECT category, COUNT(*) AS open_reports, MEDIAN(age_days) AS median_age_days,
  COUNT(*) FILTER (WHERE age_days > ${STALE_DAYS}) AS open_over_one_year, MAX(age_days) AS oldest_age_days, CAST(MIN(opened_at) AS DATE) AS earliest_opened
 FROM service_reports WHERE unresolved GROUP BY category
), service_groups AS (
 SELECT category, staff_group, COUNT(*) AS group_reports FROM service_reports WHERE unresolved AND staff_group IS NOT NULL GROUP BY category, staff_group
), service_owner AS (
 SELECT category, FIRST(staff_group ORDER BY group_reports DESC, staff_group) AS staff_group, MAX(group_reports) AS staff_group_reports FROM service_groups GROUP BY category
), service_backlog AS (
 SELECT service_open.*, staff_group, staff_group_reports FROM service_open LEFT JOIN service_owner ON service_owner.category = service_open.category
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
      method: `Median age of open reports per ${roles.category} and ${area}, against the median for that service across all areas. An area is listed with at least ${AREA_MINIMUM_REPORTS} open reports, at least three areas reporting, a median at least ${AREA_MEDIAN_RATIO} times the all-area median, and either an all-area median of 30 days or more or an area median over ${STALE_DAYS} days. ${basis}`,
      sql: `${byArea(area, 'unresolved', 'open_reports')}
WHERE open_reports >= ${AREA_MINIMUM_REPORTS} AND areas >= 3 AND (all_median_days >= 30 OR median_days > ${STALE_DAYS}) AND median_days >= ${AREA_MEDIAN_RATIO} * all_median_days
ORDER BY over_one_year DESC, open_reports DESC, service_area.category, area`,
    }, {
      id: `service_wait_area:${area}`, title: `Slower closure in some areas (${area})`,
      method: `Median days to close per ${roles.category} and ${area}, for reports with a ${roles.closed} value that were not referred, against the median for that service across all areas. An area is listed with at least ${AREA_MINIMUM_REPORTS} closed reports, at least three areas reporting, a median at least ${AREA_MEDIAN_RATIO} times the all-area median, and at least seven days slower. ${basis}`,
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
  const movement = movementQuery(schema, dataset);
  if (movement) queries.push(movement);
  const budget = roles.group ? budgetTable(schema) : null;
  if (budget) {
    const departments = [...new Set(Object.values(STAFF_GROUP_DEPARTMENTS).flat())].map((name) => `'${name.toLowerCase().replace(/'/g, "''")}'`).join(', ');
    queries.push({
      id: 'service_budget', title: 'Adopted budget of the departments behind the backlog',
      method: `Adopted operating expense per department and fiscal year from ${budget.displayName}, for the two latest fiscal years with rows marked adopted: the sum of amount where budget_cycle is adopted and account_number begins with 5 (the City's expense accounts; accounts beginning with 4 are revenue). A year whose rows carry no budget_cycle is not counted as adopted. A department's budget covers all its work, not one service.`,
      sql: `WITH budget_lines AS (
 SELECT ${text('dept_name')} AS department, TRY_CAST(CAST(${quoteIdentifier('report_fy')} AS VARCHAR) AS INTEGER) AS fiscal_year, TRY_CAST(${quoteIdentifier('amount')} AS DOUBLE) AS amount
 FROM ${quoteIdentifier(budget.tableName)}
 WHERE LOWER(TRIM(CAST(${quoteIdentifier('budget_cycle')} AS VARCHAR))) = 'adopted' AND CAST(${quoteIdentifier('account_number')} AS VARCHAR) LIKE '5%'
), budget_years AS (SELECT DISTINCT fiscal_year FROM budget_lines WHERE fiscal_year IS NOT NULL ORDER BY fiscal_year DESC LIMIT 2)
SELECT department, fiscal_year, SUM(amount) AS adopted_expense, COUNT(*) AS budget_rows, COUNT(*) FILTER (WHERE amount IS NULL) AS unreadable_amounts
FROM budget_lines WHERE fiscal_year IN (SELECT fiscal_year FROM budget_years) AND LOWER(department) IN (${departments})
GROUP BY department, fiscal_year ORDER BY department, fiscal_year DESC`,
    });
  }
  return queries;
}

const listed = (names: string[]) => names.length > 1 ? `${names.slice(0, -1).join(', ')} and ${names.at(-1)!}` : names[0];

/** Who the publisher says handles the services named, and what the budget table adopts for them.
 * `budget` is undefined when no budget table was read: that is said, never shown as an amount. */
function ownership(rows: Record<string, unknown>[], budget: Record<string, unknown>[] | undefined): { text: string; departments: string[] } {
  const groups = new Map<string, string[]>();
  for (const row of rows) {
    const carried = count(row.staff_group_reports); const open = count(row.open_reports);
    // Named only when most of the service's open reports carry the group.
    if (typeof row.staff_group !== 'string' || carried === null || open === null || 2 * carried <= open) continue;
    groups.set(row.staff_group, [...(groups.get(row.staff_group) ?? []), `${String(row.category)} (${shown(carried)} of ${shown(open)} open reports)`]);
  }
  const sentences: string[] = []; const named = new Set<string>();
  for (const [group, services] of groups) {
    const departments = staffGroupDepartments(group);
    if (!departments) { sentences.push(`The publisher lists ${listed(services)} under its staff group ${group}, which this app does not assign to a budget department.`); continue; }
    departments.forEach((name) => named.add(name));
    sentences.push(`The publisher lists ${listed(services)} under its staff group ${group}, read here as the ${listed(departments)} department${departments.length > 1 ? 's' : ''}.`);
  }
  if (!named.size) return { text: sentences.join(' '), departments: [] };
  if (!budget) return { text: `${sentences.join(' ')} No operating budget table is imported, so no adopted amount is shown.`, departments: [...named] };
  const lines = [...named].flatMap((name) => {
    const years = budget.filter((row) => String(row.department).toLowerCase() === name.toLowerCase() && count(row.fiscal_year) !== null && count(row.adopted_expense) !== null && (count(row.unreadable_amounts) ?? 0) === 0)
      .sort((a, b) => count(b.fiscal_year)! - count(a.fiscal_year)!);
    return years.length ? [`${name} ${years.map((row) => `${money(count(row.adopted_expense)!)} in FY ${count(row.fiscal_year)!}`).join(', ')}`] : [];
  });
  const missing = [...named].filter((name) => !lines.some((line) => line.startsWith(`${name} `)));
  return { departments: [...named], text: [...sentences,
    ...(lines.length ? [`Adopted operating expense: ${lines.join('; ')}. A department's budget covers all its work, not these services alone.`] : []),
    ...(missing.length ? [`The budget table has no readable adopted amount for ${listed(missing)}.`] : [])].join(' ') };
}

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
      const budget = results.find((entry) => entry.query.id === 'service_budget')?.result;
      const owner = ownership(rows.filter((row) => (count(row.open_over_one_year) ?? 0) > 0).slice(0, 3), budget ? records(budget) : undefined);
      leads.push({ key: 'service_backlog', priority: 98, query,
        title: `${shown(stale)} of ${shown(open)} open reports have been open more than a year`,
        detail: `${worst.join(' ')}${owner.text ? ` ${owner.text}` : ''} An open report is not proof the problem persists: the table records no inspections or work done.`,
        question: owner.departments.length ? `What do the ${listed(owner.departments)} budget and staffing say about the ${String(rows[0].category)} backlog?`
          : `Which department owns ${String(rows[0].category)}, and what do its budget and staffing say about this backlog?`,
      });
    } else if (id === 'service_backlog_area' || id === 'service_wait_area') {
      const open = id === 'service_backlog_area';
      [...rows.slice(0, 2), ...rows.slice(2).filter((row) => FOLLOWED_SERVICES.test(String(row.category))).slice(0, 2)].forEach((row, index) => {
        const median = count(row.median_days); const overall = count(row.all_median_days);
        if (median === null || overall === null) return;
        const reports = shown(count(open ? row.open_reports : row.closed_reports) ?? 0);
        leads.push({ key: `${query.id}:${index}`, priority: FOLLOWED_SERVICES.test(String(row.category)) ? 99 : (open ? 95 : 94) - index, query,
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
export function recurringServiceLeads(tables: { snapshotId: string; displayName: string; results: { query: DiscoveryQuery; result?: SqlQueryResult }[] }[]): (ServiceLead & { snapshotId: string; reads: string[] })[] {
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
      snapshotId: entry.hits[0].snapshotId, reads: entry.hits.slice(1).map((hit) => hit.snapshotId), key: `service_recurring:${key}`, priority: 99 - index, query: entry.hits[0].query,
      title: `${entry.category} reports in ${entry.place} closed slower than the all-area median in ${entry.hits.length} imported tables`,
      detail: `${entry.hits.map((hit) => `${hit.displayName}: median ${shown(hit.median)} days against ${shown(hit.overall)} (${shown(hit.reports)} reports).`).join(' ')} Tables are compared as imported and can overlap in the reports they hold. The difference can reflect what was reported as well as how it was handled.`,
      question: `Has the responsible department explained why ${entry.category} takes longer in ${entry.place}, and has it been raised with the council office?`,
    }));
}

function movementLead(movement: { query: DiscoveryQuery; result?: SqlQueryResult } | undefined, earlierDay: string, laterDay: string): ServiceLead | null {
  if (!movement?.result?.rows.length) return null;
  const rows = records(movement.result);
  const total = (name: string) => count(rows[0][name]) ?? 0;
  const left = total('all_left_reports'); const arrived = total('all_arrived_reports');
  if (left + arrived === 0) return null;
  const stale = total('all_left_over_one_year'); const unrecorded = total('all_no_outcome_record');
  // With no table that could hold a close record, "no record" is an unanswered question, not a count.
  const searched = total('outcome_tables') > 0 || unrecorded < left;
  const services = rows.filter((row) => (count(row.left_reports) ?? 0) > 0).slice(0, 3).map((row) =>
    `${String(row.category)}: ${shown(count(row.left_reports)!)} left (${shown(count(row.left_over_one_year) ?? 0)} open more than a year), ${shown(count(row.arrived_reports) ?? 0)} arrived.`);
  const outcome = left === 0 ? '' : searched
    ? `Of those that left, ${shown(total('all_closed_on_record'))} have a close date and ${shown(total('all_referred_on_record'))} were referred in the imported tables; ${shown(unrecorded)} have no close or referral record in any of them. `
    : 'This source lists open reports only, so whether they were closed or referred is not in it; the publisher’s closed-reports table would say. ';
  return { key: 'service_movement', priority: 99, query: movement.query,
    title: `${shown(left)} reports left the open file and ${shown(arrived)} new ones arrived between ${earlierDay} and ${laterDay}`,
    detail: `${outcome}${shown(stale)} of those that left had been open more than a year. ${services.join(' ')} A close date records that a report was closed, not what work was done.`,
    question: searched && unrecorded > 0 ? `What became of the ${shown(unrecorded)} reports that left the open file with no close or referral record?`
      : arrived > left ? 'New reports outnumbered the ones that left. Does the department’s staffing plan or budget request address the difference?'
        : `Were the ${shown(stale)} long-open reports repaired, or closed without work? A records request for the work orders would show it.`,
  };
}

/** The same published source imported on two dates: how its open backlog moved between them. */
export function backlogChangeLeads(tables: { snapshotId: string; sourceUri: string | null; retrievedAt: string; results: { query: DiscoveryQuery; result?: SqlQueryResult }[] }[]): (ServiceLead & { snapshotId: string; reads: string[] })[] {
  const bySource = new Map<string, { snapshotId: string; day: string; query: DiscoveryQuery; rows: Record<string, unknown>[]; movement?: { query: DiscoveryQuery; result?: SqlQueryResult } }[]>();
  for (const table of tables) {
    const backlog = table.results.find((entry) => entry.query.id === 'service_backlog');
    if (!table.sourceUri || !backlog?.result?.rows.length) continue;
    bySource.set(table.sourceUri, [...(bySource.get(table.sourceUri) ?? []), { snapshotId: table.snapshotId, day: table.retrievedAt.slice(0, 10), query: backlog.query, rows: records(backlog.result), movement: table.results.find((entry) => entry.query.id === 'service_movement') }]);
  }
  const leads: (ServiceLead & { snapshotId: string; reads: string[] })[] = [];
  for (const snapshots of bySource.values()) {
    const ordered = [...snapshots].sort((a, b) => a.day.localeCompare(b.day));
    const earlier = ordered[0]; const later = ordered[ordered.length - 1];
    if (earlier.day === later.day) continue;
    const lead = movementLead(later.movement, earlier.day, later.day);
    if (lead) leads.push({ ...lead, snapshotId: later.snapshotId, reads: [earlier.snapshotId] });
    const before = count(earlier.rows[0].all_open_over_one_year); const after = count(later.rows[0].all_open_over_one_year);
    const openBefore = count(earlier.rows[0].all_open_reports); const openAfter = count(later.rows[0].all_open_reports);
    if (before === null || after === null || openBefore === null || openAfter === null || before === after) continue;
    const prior = new Map(earlier.rows.map((row) => [String(row.category), count(row.open_over_one_year) ?? 0]));
    const moved = later.rows.map((row) => ({ category: String(row.category), now: count(row.open_over_one_year) ?? 0, then: prior.get(String(row.category)) }))
      .filter((row): row is { category: string; now: number; then: number } => row.then !== undefined && row.then !== row.now)
      .sort((a, b) => Math.abs(b.now - b.then) - Math.abs(a.now - a.then) || a.category.localeCompare(b.category)).slice(0, 3);
    leads.push({ snapshotId: later.snapshotId, reads: [earlier.snapshotId], key: `service_backlog_change:${earlier.snapshotId}`, priority: 99, query: later.query,
      title: `Open reports older than a year ${after > before ? 'rose' : 'fell'} from ${shown(before)} to ${shown(after)} between ${earlier.day} and ${later.day}`,
      detail: `All open reports went from ${shown(openBefore)} to ${shown(openAfter)}. ${moved.map((row) => `${row.category}: ${shown(row.then)} to ${shown(row.now)}.`).join(' ')} The two dates are when each copy was retrieved. A report leaves the open file when it is closed or referred; this comparison does not say which.`,
      question: 'Which reports left the open file between these dates, and were they repaired or closed without work?',
    });
  }
  return leads;
}
