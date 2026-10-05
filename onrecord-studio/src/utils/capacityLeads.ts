import type { AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import { budgetTable, type ServiceLead } from './serviceAccountability';

const GET_IT_DONE = 'https://seshat.datasd.org/get_it_done_reports/';
export const OPEN_REPORTS = `${GET_IT_DONE}get_it_done_requests_open_datasd.csv`;
export const closedReports = (year: number) => `${GET_IT_DONE}get_it_done_requests_closed_${year}_datasd.csv`;

/** The emergency-shelter inventory, found by the compiler's column names. */
const INVENTORY_COLUMNS = ['year', 'city', 'year_round_beds', 'total_beds'];
const REPORT_COLUMNS = ['service_request_id', 'date_requested', 'service_name'];
const CITY = 'San Diego';
const SERVICE = 'Encampment';
/** The filter the app's own fetch of a closed year for this service records. */
const SERVICE_SUBSET = `service_name = '${SERVICE}'`;
const DEPARTMENT_WORD = 'homeless';
/** Before 2018 the inventory's year-round and seasonal columns were filled differently: 170
 * year-round beds in 2014 and 374 in 2015 against 1,550 in 2018 (measured 2026-10-04). */
const FIRST_INVENTORY_YEAR = 2018;
/** The budget file has no department with "homeless" in its name before fiscal 2021 (measured
 * 2026-10-04), so reports filed before 1 July 2020 have nothing to be set beside. */
export const EARLIEST_USEFUL_CLOSED_YEAR = 2020;
/** A fiscal year is compared only with this many Encampment reports filed in it. */
const REPORT_MINIMUM = 30;
/** A lead is raised when one series differs by this share between the first and latest year. A
 * fixed yardstick, not a statistical test. */
const CHANGE_PERCENT = 10;
const COMPARED_YEARS_MINIMUM = 2;

const has = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary, names: string[]) =>
  names.every((name) => schema.columns.some((column) => column.tableName === dataset.tableName && column.columnName === name));
const latestFirst = (a: AnalyticalDatasetSummary, b: AnalyticalDatasetSummary) => b.retrievedAt.localeCompare(a.retrievedAt) || a.snapshotId.localeCompare(b.snapshotId);
const day = (dataset: AnalyticalDatasetSummary) => dataset.retrievedAt.slice(0, 10);

/** The latest imported table with the shelter inventory's columns. */
export function shelterInventoryTable(schema: AnalyticalSchema): AnalyticalDatasetSummary | null {
  return schema.datasets.filter((dataset) => has(schema, dataset, INVENTORY_COLUMNS)).sort(latestFirst)[0] ?? null;
}

/** A closed-year import holds every Encampment report closed that year when it is the whole file
 * or the Encampment subset; the subset of reports seen in an open copy does not. */
function holdsEveryEncampmentReport(dataset: AnalyticalDatasetSummary): boolean {
  try {
    const filter: unknown = (JSON.parse(dataset.validationSummary) as { derivedSubset?: { filter?: unknown } | null }).derivedSubset?.filter;
    return filter == null || filter === SERVICE_SUBSET;
  } catch { return true; }
}

export interface EncampmentReportCoverage {
  open: AnalyticalDatasetSummary;
  /** One table per closed year, latest year first, with no year missing between them. */
  closed: AnalyticalDatasetSummary[];
  /** Fiscal years (named by the year they end) whose every filed report is in the tables above. */
  fiscalYears: number[];
  /** The closed year whose file would make one more fiscal year countable, if any is worth fetching. */
  nextClosedYear: number | null;
}

/** Closed files are published by close year, so every report filed in a fiscal year is held only
 * by the open file together with each closed file from the calendar year it began onward. */
export function encampmentReportCoverage(schema: AnalyticalSchema): EncampmentReportCoverage | null {
  const open = schema.datasets.filter((dataset) => dataset.sourceUri === OPEN_REPORTS && has(schema, dataset, REPORT_COLUMNS)).sort(latestFirst)[0];
  if (!open) return null;
  const openYear = Number(day(open).slice(0, 4));
  const closed: AnalyticalDatasetSummary[] = [];
  let year = openYear;
  for (; year >= EARLIEST_USEFUL_CLOSED_YEAR; year -= 1) {
    const uri = closedReports(year);
    // A past year's file no longer changes; the current year's must not predate the open copy,
    // or a report closed between the two retrievals would be in neither. Retrieval times are UTC
    // and the publisher's dates local, so a period counts as ended a day after its last date.
    const table = schema.datasets.filter((dataset) => dataset.sourceUri === uri && has(schema, dataset, REPORT_COLUMNS) && holdsEveryEncampmentReport(dataset)
      && (day(dataset) > `${year + 1}-01-01` || dataset.retrievedAt >= open.retrievedAt)).sort(latestFirst)[0];
    if (!table) break;
    closed.push(table);
  }
  const lastEnded = day(open) > `${openYear}-07-01` ? openYear : openYear - 1;
  const fiscalYears: number[] = [];
  for (let fiscalYear = year + 2; fiscalYear <= lastEnded; fiscalYear += 1) fiscalYears.push(fiscalYear);
  return { open, closed, fiscalYears, nextClosedYear: year >= EARLIEST_USEFUL_CLOSED_YEAR ? year : null };
}

/** The tables the comparison reads beside the inventory, or null when one of them is missing. */
export function capacityCompanions(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): { budget: AnalyticalDatasetSummary; reports: EncampmentReportCoverage } | null {
  if (shelterInventoryTable(schema)?.snapshotId !== dataset.snapshotId) return null;
  const budget = budgetTable(schema); const reports = encampmentReportCoverage(schema);
  return budget && reports && reports.fiscalYears.length >= COMPARED_YEARS_MINIMUM ? { budget, reports } : null;
}

const text = (name: string) => `TRIM(CAST(${quoteIdentifier(name)} AS VARCHAR))`;
const whole = (name: string) => `TRY_CAST(TRY_CAST(CAST(${quoteIdentifier(name)} AS VARCHAR) AS DOUBLE) AS INTEGER)`;
const amount = (name: string) => `TRY_CAST(${quoteIdentifier(name)} AS DOUBLE)`;

export function capacityQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const companions = capacityCompanions(schema, dataset);
  if (!companions) return [];
  const { budget, reports } = companions;
  const reportTables = [reports.open, ...reports.closed];
  const openYear = day(reports.open).slice(0, 4);
  return [{
    id: 'capacity_comparison', title: 'Homelessness budget, Encampment reports and emergency shelter beds, by fiscal year',
    method: `Three series for the City of ${CITY} per fiscal year (1 July to 30 June, named by the year it ends). Budget: the sum of amount in ${budget.displayName} where budget_cycle is adopted, account_number begins with 5 (the City's expense accounts) and dept_name contains "${DEPARTMENT_WORD}"; a two-digit report_fy is read as 2000 plus that number. A year with no such department has no amount, which is not an amount of zero, and rows with no budget_cycle are not counted as adopted. Reports: rows whose service_name is ${SERVICE} in ${reports.open.displayName} and the closed-reports tables for ${Number(openYear) - reports.closed.length + 1} to ${openYear}, counted once per service_request_id and filing time, by the fiscal year of date_requested. The publisher files a closed report under the year it was closed, so a fiscal year is counted only when the open file and every closed file from the calendar year it began through ${openYear} are imported, each retrieved at least a day after its year ended or no earlier than the open copy. Here that is FY ${reports.fiscalYears[0]} to FY ${reports.fiscalYears.at(-1)!}; other years are not counted. Beds: the sum of Year_Round_Beds, and of Total_Beds, over the rows of ${dataset.displayName} whose City is ${CITY}, per Year. The inventory is a count made on one night each January, so the January 2025 count is set beside fiscal year 2025, seven months into it. Years before ${FIRST_INVENTORY_YEAR} are left out because the year-round and seasonal columns were filled differently then. A fiscal year is compared when all three series are present and at least ${REPORT_MINIMUM} ${SERVICE} reports were filed in it. A lead is raised with at least ${COMPARED_YEARS_MINIMUM} such years and one series differing by ${CHANGE_PERCENT}% or more between the first and the latest; ${CHANGE_PERCENT}% is a fixed yardstick, not a statistical test. The series come from three publishers and nothing here shows why any of them moved.`,
    sql: `WITH budget_lines AS (
 SELECT ${text('dept_name')} AS department, ${whole('report_fy')} AS report_fy, ${amount('amount')} AS amount,
  CAST(${quoteIdentifier('account_number')} AS VARCHAR) LIKE '5%' AS expense, COALESCE(CONTAINS(LOWER(${text('dept_name')}), '${DEPARTMENT_WORD}'), FALSE) AS named
 FROM ${quoteIdentifier(budget.tableName)}
 WHERE LOWER(${text('budget_cycle')}) = 'adopted'
), budget_years AS (
 SELECT CASE WHEN report_fy BETWEEN 0 AND 99 THEN 2000 + report_fy ELSE report_fy END AS fiscal_year, COUNT(*) AS adopted_rows, COUNT(*) FILTER (WHERE named) AS department_rows,
  SUM(amount) FILTER (WHERE named AND expense) AS adopted_expense, COUNT(*) FILTER (WHERE named AND expense AND amount IS NULL) AS unreadable_amounts,
  STRING_AGG(DISTINCT department, '; ') FILTER (WHERE named) AS departments
 FROM budget_lines WHERE report_fy IS NOT NULL GROUP BY 1
), report_rows AS (
${reportTables.map((table) => ` SELECT ${text('service_request_id')} AS report_id, TRY_CAST(CAST(${quoteIdentifier('date_requested')} AS VARCHAR) AS TIMESTAMP) AS filed_at
 FROM ${quoteIdentifier(table.tableName)} WHERE ${text('service_name')} = '${SERVICE}'`).join('\n UNION ALL\n')}
), reports AS (SELECT DISTINCT report_id, filed_at FROM report_rows
), report_years AS (
 SELECT YEAR(filed_at) + CAST(MONTH(filed_at) >= 7 AS INTEGER) AS fiscal_year, COUNT(*) AS encampment_reports FROM reports WHERE filed_at IS NOT NULL GROUP BY 1
), report_span AS (
 SELECT COUNT(*) FILTER (WHERE filed_at IS NULL) AS undated_reports FROM reports
), bed_years AS (
 SELECT ${whole('year')} AS fiscal_year, COUNT(*) AS shelter_projects, SUM(${amount('year_round_beds')}) AS year_round_beds, SUM(${amount('total_beds')}) AS total_beds,
  COUNT(*) FILTER (WHERE ${amount('year_round_beds')} IS NULL) AS projects_without_year_round, COUNT(*) FILTER (WHERE ${amount('total_beds')} IS NULL) AS projects_without_total
 FROM ${quoteIdentifier(dataset.tableName)} WHERE LOWER(${text('city')}) = '${CITY.toLowerCase()}' GROUP BY 1
)
SELECT bed_years.fiscal_year, adopted_rows, department_rows, departments, adopted_expense, unreadable_amounts,
 CASE WHEN bed_years.fiscal_year IN (${reports.fiscalYears.join(', ')}) THEN COALESCE(encampment_reports, 0) END AS encampment_reports, undated_reports,
 shelter_projects, year_round_beds, total_beds, projects_without_year_round, projects_without_total
FROM bed_years LEFT JOIN budget_years ON budget_years.fiscal_year = bed_years.fiscal_year
 LEFT JOIN report_years ON report_years.fiscal_year = bed_years.fiscal_year CROSS JOIN report_span
WHERE bed_years.fiscal_year >= ${FIRST_INVENTORY_YEAR} ORDER BY bed_years.fiscal_year`,
  }];
}

const count = (value: unknown): number | null => {
  if (value == null || value === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};
const shown = (value: number) => Math.round(value).toLocaleString('en-US');
const money = (value: number) => Math.abs(value) >= 1_000_000 ? `$${(value / 1_000_000).toFixed(1)} million` : `$${shown(value)}`;
const moved = (from: number, to: number) => from !== to && Math.abs(to - from) * 100 >= CHANGE_PERCENT * Math.abs(from);
const change = (from: number, to: number) => from === to ? 'unchanged' : from === 0 ? 'up from none' : `${to > from ? 'up' : 'down'} ${(100 * Math.abs(to - from) / Math.abs(from)).toFixed(1)}%`;

interface Year { fiscalYear: number; expense: number; reports: number; yearRound: number; total: number | null; projects: number; withoutYearRound: number; withoutTotal: number }

export function capacityLeads(results: { query: DiscoveryQuery; result?: SqlQueryResult }[]): ServiceLead[] {
  const entry = results.find((item) => item.query.id === 'capacity_comparison');
  if (!entry?.result?.rows.length) return [];
  const rows = entry.result.rows.map((values) => Object.fromEntries(entry.result!.columns.map((name, index) => [name, values[index]])));
  const compared: Year[] = []; const setAside = new Map<string, number[]>();
  for (const row of rows) {
    const fiscalYear = count(row.fiscal_year);
    if (fiscalYear === null) continue;
    const expense = count(row.adopted_expense); const reports = count(row.encampment_reports); const yearRound = count(row.year_round_beds);
    const departmentRows = count(row.department_rows);
    // Absence is said as absence: no department, no adopted rows and no complete count are three
    // different facts, and none of them is an amount or a count of zero.
    const reason = count(row.adopted_rows) === null ? 'the budget table has no rows marked adopted'
      : departmentRows === 0 ? `the budget table lists no department with "${DEPARTMENT_WORD}" in its name, which is not an amount of zero`
        : expense === null || (count(row.unreadable_amounts) ?? 0) > 0 ? 'the department\'s adopted expense could not be read in full'
          : reports === null ? `the imported files cannot hold every ${SERVICE} report filed, so none is counted`
            : reports < REPORT_MINIMUM ? `fewer than ${REPORT_MINIMUM} ${SERVICE} reports were filed`
              : yearRound === null ? 'the inventory records no year-round bed count' : null;
    if (reason) { setAside.set(reason, [...(setAside.get(reason) ?? []), fiscalYear]); continue; }
    compared.push({ fiscalYear, expense: expense!, reports: reports!, yearRound: yearRound!, total: count(row.total_beds), projects: count(row.shelter_projects) ?? 0,
      withoutYearRound: count(row.projects_without_year_round) ?? 0, withoutTotal: count(row.projects_without_total) ?? 0 });
  }
  if (compared.length < COMPARED_YEARS_MINIMUM) return [];
  const first = compared[0]; const latest = compared.at(-1)!;
  if (!moved(first.expense, latest.expense) && !moved(first.reports, latest.reports) && !moved(first.yearRound, latest.yearRound)) return [];
  const departments = [...new Set(rows.flatMap((row) => typeof row.departments === 'string' ? row.departments.split('; ') : []))].join(', ');
  const undated = count(rows[0].undated_reports) ?? 0;
  const perYear = compared.map((year) => `FY ${year.fiscalYear}: ${money(year.expense)} adopted, ${shown(year.reports)} ${SERVICE} reports filed, ${shown(year.yearRound)} year-round emergency shelter beds${year.total === null ? '' : ` (${shown(year.total)} beds of every kind)`} in the January ${year.fiscalYear} count.`);
  const unrecorded = compared.filter((year) => year.withoutYearRound > 0 || year.withoutTotal > 0).map((year) =>
    `In the ${year.fiscalYear} count, ${year.withoutYearRound} of ${year.projects} projects record no year-round bed count and ${year.withoutTotal} no total; they add nothing to the sums.`);
  return [{ key: 'capacity_comparison', priority: 97, query: entry.query,
    title: `Adopted homelessness budget ${money(first.expense)} to ${money(latest.expense)}; ${SERVICE} reports ${shown(first.reports)} to ${shown(latest.reports)}; year-round emergency shelter beds ${shown(first.yearRound)} to ${shown(latest.yearRound)}, fiscal ${first.fiscalYear} to ${latest.fiscalYear}`,
    detail: `Three counts for the City of ${CITY}, set beside each other for each fiscal year (1 July to 30 June) in which all three are available. ${perYear.join(' ')} Between FY ${first.fiscalYear} and FY ${latest.fiscalYear}: adopted operating expense of ${departments || 'the department'} ${money(first.expense)} to ${money(latest.expense)} (${change(first.expense, latest.expense)}); ${SERVICE} reports filed ${shown(first.reports)} to ${shown(latest.reports)} (${change(first.reports, latest.reports)}); year-round emergency shelter beds ${shown(first.yearRound)} to ${shown(latest.yearRound)} (${change(first.yearRound, latest.yearRound)}).${unrecorded.length ? ` ${unrecorded.join(' ')}` : ''}${undated ? ` ${shown(undated)} ${SERVICE} reports have no readable filing date and are in no year.` : ''}${setAside.size ? ` Not compared: ${[...setAside].map(([reason, years]) => `${years.map((year) => `FY ${year}`).join(', ')} (${reason})`).join('; ')}.` : ''} The first and latest years are endpoints, and the years between do not move in a straight line; every year is listed above. The budget figure is the amount adopted for one department, not all homelessness spending: the San Diego Housing Commission and the County spend separately, and an adopted amount is not an amount spent. The beds are emergency shelter only; transitional and permanent housing are not in the inventory. The reports are complaints residents filed, not people or encampments, and one place can be reported many times. The inventory was compiled by UCSD's Homelessness Hub from the Regional Task Force on Homelessness count and states no licence. The bed count is one night each January; the other two series cover the whole fiscal year. Nothing here shows why any series moved.`,
    question: 'Which programmes did each year\'s adopted amount pay for, and how many beds did each fund? The department\'s budget volumes, its actual expenditure by programme and the Housing Commission\'s shelter operating agreements for the same years would show it.',
  }];
}
