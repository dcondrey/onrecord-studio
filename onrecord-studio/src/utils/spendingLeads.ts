import type { AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import { budgetTable, type ServiceLead } from './serviceAccountability';

/** The City's operating actuals file: the budget file's columns without `budget_cycle`. */
const ACTUALS_COLUMNS = ['dept_name', 'report_fy', 'amount', 'account', 'account_number'];
/** The State Controller's city pay file, one row per position held. */
const PAY_COLUMNS = ['employername', 'departmentorsubdivision', 'position', 'regularpay', 'overtimepay', 'totalwages'];
/** Departments behind the subjects followed: homelessness, abatement (Environmental Services
 * handles Encampment reports) and policing. Names as budget_operating_datasd.csv spells them. */
const FOLLOWED_WORD = 'homeless';
const FOLLOWED_DEPARTMENTS = ['police', 'environmental services'];
/** A department-year is reported when actual expense is at least this share below adopted and at
 * least this many dollars below. Ten percent is the yardstick the financial workspace uses for a
 * department's year-over-year change; the dollar floor keeps out a small office's unfilled post.
 * On the files retrieved 2026-10-05, 110 of 813 department-years meet both. Not a statistical test. */
export const BELOW_ADOPTED_PERCENT = 10;
export const BELOW_ADOPTED_DOLLARS = 1_000_000;
/** A department-year is reported when actual overtime is at least this share above adopted
 * overtime and at least this many dollars above: the expense comparison's yardsticks, read the
 * other way. The dollar floor keeps out a small office whose overtime line is a few thousand
 * dollars. Not a statistical test. */
export const ABOVE_ADOPTED_OVERTIME_PERCENT = 10;
export const ABOVE_ADOPTED_OVERTIME_DOLLARS = 1_000_000;
/** Department-years listed beside the followed departments, by actual overtime. Kept small: a
 * result past the query transport's 100-row limit is truncated and gives no lead at all. */
const OVERTIME_DEPARTMENT_YEARS_LISTED = 10;
const NO_ACTUAL_ROWS_LISTED = 12;
const SETTLEMENT_YEARS = 5;
const SETTLEMENT_DEPARTMENTS_LISTED = 3;
/** An employer is described with at least this many positions in the pay file. */
export const PAY_MINIMUM_POSITIONS = 100;
const PAY_EMPLOYERS = 3;
const PAY_DEPARTMENTS_LISTED = 5;
/** A department's overtime share is stated with at least this many positions behind it. */
const PAY_DEPARTMENT_POSITIONS = 30;
const HIGH_OVERTIME_DOLLARS = 100_000;
/** The pay file covers every city and many share department names (Police, City Clerk). An
 * employer is read as the budget's city only when this many of its department names, and at least
 * half of them, are the budget's own. Measured on the 2024 file: San Diego 56 of 60; the next
 * are Sacramento 17 of 257 and Los Angeles 9 of 46. */
const BUDGET_DEPARTMENTS_MATCHED = 20;

const has = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary, names: string[]) =>
  names.every((name) => schema.columns.some((column) => column.tableName === dataset.tableName && column.columnName === name));
const latestFirst = (a: AnalyticalDatasetSummary, b: AnalyticalDatasetSummary) => b.retrievedAt.localeCompare(a.retrievedAt) || a.snapshotId.localeCompare(b.snapshotId);

/** The latest imported table with the operating actuals' columns. A budget table has them too and
 * is told apart by its `budget_cycle` column. */
export function actualsTable(schema: AnalyticalSchema): AnalyticalDatasetSummary | null {
  return schema.datasets.filter((dataset) => has(schema, dataset, ACTUALS_COLUMNS) && !has(schema, dataset, ['budget_cycle'])).sort(latestFirst)[0] ?? null;
}
const isPayTable = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary) => has(schema, dataset, PAY_COLUMNS);

/** The budget table a spending check on this table reads, when there is one. */
export function spendingCompanion(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): AnalyticalDatasetSummary | null {
  return actualsTable(schema)?.snapshotId === dataset.snapshotId || isPayTable(schema, dataset) ? budgetTable(schema) : null;
}

const text = (name: string) => `NULLIF(TRIM(CAST(${quoteIdentifier(name)} AS VARCHAR)), '')`;
const whole = (name: string) => `TRY_CAST(TRY_CAST(CAST(${quoteIdentifier(name)} AS VARCHAR) AS DOUBLE) AS INTEGER)`;
const amount = (name: string) => `TRY_CAST(${quoteIdentifier(name)} AS DOUBLE)`;
const fiscal = (expression: string) => `CASE WHEN ${expression} BETWEEN 0 AND 99 THEN 2000 + ${expression} ELSE ${expression} END`;
const expense = `CAST(${quoteIdentifier('account_number')} AS VARCHAR) LIKE '5%'`;
const adoptedRows = `LOWER(TRIM(CAST(${quoteIdentifier('budget_cycle')} AS VARCHAR))) = 'adopted'`;
const followed = (department: string) => `COALESCE(CONTAINS(LOWER(${department}), '${FOLLOWED_WORD}') OR LOWER(${department}) IN (${FOLLOWED_DEPARTMENTS.map((name) => `'${name}'`).join(', ')}), FALSE)`;
const SETTLEMENT_ACCOUNTS = `(CONTAINS(label, 'negotiated settlement') OR label LIKE 'claims-pub liab%')`;
const FOLLOWED_TEXT = `a department with "${FOLLOWED_WORD}" in its name, Police and Environmental Services`;
const ADOPTED_LIMITS = 'Adopted is the original appropriation, not the revised budget: a transfer or a change made during the year is in neither figure. An amount not spent may have been carried forward, moved to another department or returned to its fund; these tables do not say which, and nothing here says the money was put to another use.';

function adoptedActualQuery(budget: AnalyticalDatasetSummary, actuals: AnalyticalDatasetSummary): DiscoveryQuery {
  return {
    id: 'spending_adopted_actual', title: 'Adopted against actual operating expense, by department and fiscal year',
    method: `Per department and fiscal year (1 July to 30 June, named by the year it ends; a two-digit report_fy is read as 2000 plus that number): adopted is the sum of amount in ${budget.displayName} where budget_cycle is adopted and account_number begins with 5 (the City's expense accounts); actual is the sum of amount in ${actuals.displayName} over the same accounts. Departments are matched on dept_name without regard to letter case. Only fiscal years with rows in both tables are read; a year the actuals table does not cover is not counted. The rows returned are ${FOLLOWED_TEXT}, and any department-year with adopted rows and no row of any account in the actuals table; one whose adopted rows sum to more than zero is listed. A department-year is reported when actual is at least ${BELOW_ADOPTED_PERCENT}% below adopted and at least $${BELOW_ADOPTED_DOLLARS.toLocaleString('en-US')} below; both are fixed yardsticks, not a statistical test. A department-year with no actual rows is listed as having none, which is not spending of zero. ${ADOPTED_LIMITS}`,
    sql: `WITH adopted_lines AS (
 SELECT ${text('dept_name')} AS department, ${whole('report_fy')} AS report_fy, ${amount('amount')} AS amount
 FROM ${quoteIdentifier(budget.tableName)} WHERE ${adoptedRows} AND ${expense}
), adopted AS (
 SELECT LOWER(department) AS department_key, MIN(department) AS department, ${fiscal('report_fy')} AS fiscal_year, SUM(amount) AS adopted_expense, COUNT(*) FILTER (WHERE amount IS NULL) AS unreadable_adopted
 FROM adopted_lines WHERE report_fy IS NOT NULL AND department IS NOT NULL GROUP BY 1, 3
), actual_lines AS (
 SELECT LOWER(${text('dept_name')}) AS department_key, ${fiscal(whole('report_fy'))} AS fiscal_year, ${amount('amount')} AS amount, ${expense} AS expense
 FROM ${quoteIdentifier(actuals.tableName)}
), actual AS (
 SELECT department_key, fiscal_year, COUNT(*) AS actual_rows, COUNT(*) FILTER (WHERE expense) AS actual_expense_rows, SUM(amount) FILTER (WHERE expense) AS actual_expense,
  COUNT(*) FILTER (WHERE expense AND amount IS NULL) AS unreadable_actual
 FROM actual_lines WHERE fiscal_year IS NOT NULL AND department_key IS NOT NULL GROUP BY 1, 2
), actual_years AS (SELECT DISTINCT fiscal_year FROM actual_lines WHERE fiscal_year IS NOT NULL
), compared AS (
 SELECT adopted.department, adopted.fiscal_year, adopted_expense, unreadable_adopted, actual_rows, actual_expense_rows, actual_expense, unreadable_actual, ${followed('adopted.department')} AS followed
 FROM adopted JOIN actual_years ON actual_years.fiscal_year = adopted.fiscal_year
  LEFT JOIN actual ON actual.department_key = adopted.department_key AND actual.fiscal_year = adopted.fiscal_year
), span AS (SELECT MIN(fiscal_year) AS first_covered_year, MAX(fiscal_year) AS last_covered_year, COUNT(*) AS department_years FROM compared)
SELECT compared.*, first_covered_year, last_covered_year, department_years
FROM compared CROSS JOIN span WHERE followed OR actual_rows IS NULL ORDER BY department, fiscal_year`,
  };
}

const OVERTIME_LIMITS = 'Overtime accounts are the expense account numbers whose label in the budget table contains "overtime"; an actuals row is counted by its account number, whatever its label there, and overtime booked to another account is in neither figure. Adopted is the original appropriation, not the revised budget: a transfer made during the year to pay for overtime is in neither figure. Nothing here says why the overtime was worked or whether it was authorised.';

function overtimeQuery(budget: AnalyticalDatasetSummary, actuals: AnalyticalDatasetSummary): DiscoveryQuery {
  const overtimeAccount = `${whole('account_number')} IN (SELECT account_number FROM ot_accounts)`;
  return {
    id: 'spending_overtime_actual', title: 'Adopted against actual overtime, by department and fiscal year',
    method: `Overtime accounts are the account numbers in ${budget.displayName} that begin with 5 (the City's expense accounts) and whose account label contains "overtime"; account numbers are compared as whole numbers, so 500128 and 500128.0 are one account. Per department and fiscal year (1 July to 30 June, named by the year it ends; a two-digit report_fy is read as 2000 plus that number): adopted overtime is the sum of amount in ${budget.displayName} where budget_cycle is adopted and the account is an overtime account; actual overtime is the sum of amount in ${actuals.displayName} over the same account numbers, whatever the label there. Departments are matched on dept_name without regard to letter case. Only fiscal years with rows in both tables are read, and only department-years with an adopted overtime row: a department with overtime in the actuals and no adopted overtime line is not listed. The rows returned are ${FOLLOWED_TEXT}, and the ${OVERTIME_DEPARTMENT_YEARS_LISTED} department-years with most actual overtime. A department-year is reported when actual overtime is at least ${ABOVE_ADOPTED_OVERTIME_PERCENT}% above adopted overtime and at least $${ABOVE_ADOPTED_OVERTIME_DOLLARS.toLocaleString('en-US')} above; both are fixed yardsticks, not a statistical test. A department-year with no actual rows, or with rows and none in an overtime account, is listed as such, which is not overtime of zero. ${OVERTIME_LIMITS}`,
    sql: `WITH ot_accounts AS (
 SELECT DISTINCT ${whole('account_number')} AS account_number FROM ${quoteIdentifier(budget.tableName)}
 WHERE ${expense} AND COALESCE(CONTAINS(LOWER(CAST(${quoteIdentifier('account')} AS VARCHAR)), 'overtime'), FALSE) AND ${whole('account_number')} IS NOT NULL
), adopted_lines AS (
 SELECT ${text('dept_name')} AS department, ${fiscal(whole('report_fy'))} AS fiscal_year, ${amount('amount')} AS amount, CAST(${whole('account_number')} AS VARCHAR) || ' ' || COALESCE(${text('account')}, '(no label)') AS account_line
 FROM ${quoteIdentifier(budget.tableName)} WHERE ${adoptedRows} AND ${overtimeAccount}
), adopted AS (
 SELECT LOWER(department) AS department_key, MIN(department) AS department, fiscal_year, SUM(amount) AS adopted_overtime, COUNT(*) FILTER (WHERE amount IS NULL) AS unreadable_adopted,
  STRING_AGG(DISTINCT account_line, ', ' ORDER BY account_line) AS adopted_accounts
 FROM adopted_lines WHERE fiscal_year IS NOT NULL AND department IS NOT NULL GROUP BY 1, 3
), actual_lines AS (
 SELECT LOWER(${text('dept_name')}) AS department_key, ${fiscal(whole('report_fy'))} AS fiscal_year, ${amount('amount')} AS amount, COALESCE(${overtimeAccount}, FALSE) AS overtime,
  CAST(${whole('account_number')} AS VARCHAR) || ' ' || COALESCE(${text('account')}, '(no label)') AS account_line
 FROM ${quoteIdentifier(actuals.tableName)}
), actual AS (
 SELECT department_key, fiscal_year, COUNT(*) AS actual_rows, COUNT(*) FILTER (WHERE overtime) AS actual_ot_rows, SUM(amount) FILTER (WHERE overtime) AS actual_overtime,
  COUNT(*) FILTER (WHERE overtime AND amount IS NULL) AS unreadable_actual, STRING_AGG(DISTINCT account_line, ', ' ORDER BY account_line) FILTER (WHERE overtime) AS actual_accounts
 FROM actual_lines WHERE fiscal_year IS NOT NULL AND department_key IS NOT NULL GROUP BY 1, 2
), actual_years AS (SELECT DISTINCT fiscal_year FROM actual_lines WHERE fiscal_year IS NOT NULL
), compared AS (
 SELECT adopted.department, adopted.fiscal_year, adopted_overtime, unreadable_adopted, adopted_accounts, actual_rows, actual_ot_rows, actual_overtime, unreadable_actual, actual_accounts, ${followed('adopted.department')} AS followed
 FROM adopted JOIN actual_years ON actual_years.fiscal_year = adopted.fiscal_year
  LEFT JOIN actual ON actual.department_key = adopted.department_key AND actual.fiscal_year = adopted.fiscal_year
), ranked AS (
 SELECT *, ROW_NUMBER() OVER (ORDER BY actual_overtime DESC NULLS LAST, department, fiscal_year) AS position FROM compared
), span AS (SELECT MIN(fiscal_year) AS first_covered_year, MAX(fiscal_year) AS last_covered_year, COUNT(*) AS department_years FROM compared)
SELECT ranked.*, first_covered_year, last_covered_year, department_years
FROM ranked CROSS JOIN span WHERE followed OR position <= ${OVERTIME_DEPARTMENT_YEARS_LISTED} ORDER BY department, fiscal_year`,
  };
}

function settlementQuery(actuals: AnalyticalDatasetSummary): DiscoveryQuery {
  return {
    id: 'spending_settlements', title: 'Settlement and public-liability claim accounts, by fiscal year',
    method: `Rows of ${actuals.displayName} whose account label contains "Negotiated Settlement" or begins "Claims-Pub Liab", summed per fiscal year and department for the ${SETTLEMENT_YEARS} latest fiscal years in the table. Lease settlements and workers' compensation claims are other accounts and are left out. Each year is given with the number of departments that have a row in these accounts and the number of departments in the table that year; the ${SETTLEMENT_DEPARTMENTS_LISTED} largest departments are named, with ${FOLLOWED_TEXT} when they have such a row. A year or department with no row in these accounts is said to have none, which is not an amount of zero. The yearly figures are not compared with each other: on the file retrieved 2026-10-05 the Negotiated Settlement account has rows from 1 department in fiscal 2021 and 2022 and 38 in fiscal 2023, so a difference between years can be a change in where the City records a payment. The accounts record what was booked to a department's ledger, not the cases, the claimants or the reasons.`,
    sql: `WITH lines AS (
 SELECT ${text('dept_name')} AS department, ${fiscal(whole('report_fy'))} AS fiscal_year, ${amount('amount')} AS amount, LOWER(${text('account')}) AS label
 FROM ${quoteIdentifier(actuals.tableName)}
), years AS (
 SELECT fiscal_year, COUNT(DISTINCT LOWER(department)) AS departments_in_table FROM lines WHERE fiscal_year IS NOT NULL GROUP BY 1 ORDER BY fiscal_year DESC LIMIT ${SETTLEMENT_YEARS}
), settled AS (
 SELECT fiscal_year, department, SUM(amount) AS department_amount, COUNT(*) AS department_rows, COUNT(*) FILTER (WHERE amount IS NULL) AS unreadable
 FROM lines WHERE ${SETTLEMENT_ACCOUNTS} AND department IS NOT NULL AND fiscal_year IN (SELECT fiscal_year FROM years) GROUP BY 1, 2
), ranked AS (
 SELECT *, ROW_NUMBER() OVER (PARTITION BY fiscal_year ORDER BY ABS(department_amount) DESC NULLS LAST, department) AS position, COUNT(*) OVER (PARTITION BY fiscal_year) AS departments_reporting,
  SUM(department_amount) OVER (PARTITION BY fiscal_year) AS year_amount, SUM(department_rows) OVER (PARTITION BY fiscal_year) AS year_rows, SUM(unreadable) OVER (PARTITION BY fiscal_year) AS unreadable_amounts,
  ${followed('department')} AS followed
 FROM settled
)
SELECT years.fiscal_year, departments_in_table, departments_reporting, year_amount, year_rows, unreadable_amounts, department, department_amount, department_rows, position, followed
FROM years LEFT JOIN ranked ON ranked.fiscal_year = years.fiscal_year AND (position <= ${SETTLEMENT_DEPARTMENTS_LISTED} OR followed)
ORDER BY years.fiscal_year DESC, position`,
  };
}

function payQuery(schema: AnalyticalSchema, pay: AnalyticalDatasetSummary, budget: AnalyticalDatasetSummary | null): DiscoveryQuery {
  const year = has(schema, pay, ['year']) ? whole('year') : 'NULL';
  const beside = budget
    ? ` An employer is read as the city of ${budget.displayName} only when at least ${BUDGET_DEPARTMENTS_MATCHED} of its department names, and at least half of them, are that table's own dept_name values. For that employer each department listed is set beside its adopted overtime: the sum of amount where budget_cycle is adopted, account_number begins with 5 and the account label contains "overtime", for the fiscal year with the pay file's year number and the one after. The pay file covers a calendar year and the budget a fiscal year from 1 July to 30 June, so calendar 2024 is the second half of fiscal 2024 and the first half of fiscal 2025; the figures are beside each other, not the same period. A department with no adopted overtime row is said to have none, which is not an amount of zero.`
    : ' No operating budget table is imported, so no adopted overtime is shown.';
  return {
    id: 'spending_pay_overtime', title: 'Overtime paid, by employer and department',
    method: `Rows of ${pay.displayName}, one per position held as the State Controller publishes it (no names; a person who held two positions is two rows). Per EmployerName with at least ${PAY_MINIMUM_POSITIONS} rows: the sums of OvertimePay and RegularPay, and the count of positions whose OvertimePay is $${HIGH_OVERTIME_DOLLARS.toLocaleString('en-US')} or more. Per DepartmentOrSubdivision: the same sums and overtime as a share of regular pay, for the ${PAY_DEPARTMENTS_LISTED} departments with most overtime and for ${FOLLOWED_TEXT}; a share is stated with at least ${PAY_DEPARTMENT_POSITIONS} positions. At most ${PAY_EMPLOYERS} employers are described: the budget's city first, then by number of rows. A row with no overtime figure adds nothing to a sum and is counted apart. Positions whose overtime is above their regular pay are not counted: a low regular figure can be part of a year in the position, and the file does not say which.${beside} The figures are amounts employers reported to the Controller; nothing here says why overtime was worked or whether it was authorised.`,
    sql: `WITH pay AS (
 SELECT ${text('employername')} AS employer, ${text('departmentorsubdivision')} AS department, ${amount('regularpay')} AS regular_pay, ${amount('overtimepay')} AS overtime_pay, ${year} AS pay_year
 FROM ${quoteIdentifier(pay.tableName)}
), budget_lines AS (
 ${budget ? `SELECT LOWER(${text('dept_name')}) AS department_key, ${fiscal(whole('report_fy'))} AS fiscal_year, ${amount('amount')} AS amount,
  ${adoptedRows} AND ${expense} AND COALESCE(CONTAINS(LOWER(CAST(${quoteIdentifier('account')} AS VARCHAR)), 'overtime'), FALSE) AS adopted_overtime
 FROM ${quoteIdentifier(budget.tableName)}` : 'SELECT CAST(NULL AS VARCHAR) AS department_key, CAST(NULL AS INTEGER) AS fiscal_year, CAST(NULL AS DOUBLE) AS amount, FALSE AS adopted_overtime WHERE FALSE'}
), budget_departments AS (SELECT DISTINCT department_key FROM budget_lines WHERE department_key IS NOT NULL
), budget_overtime AS (
 SELECT department_key, fiscal_year, SUM(amount) AS adopted_overtime FROM budget_lines WHERE adopted_overtime AND department_key IS NOT NULL AND fiscal_year IS NOT NULL GROUP BY 1, 2
), employers AS (
 SELECT employer, COUNT(*) AS positions, SUM(regular_pay) AS regular_pay, SUM(overtime_pay) AS overtime_pay, COUNT(*) FILTER (WHERE overtime_pay >= ${HIGH_OVERTIME_DOLLARS}) AS high_overtime_positions,
  COUNT(*) FILTER (WHERE overtime_pay IS NULL) AS positions_without_overtime, COUNT(DISTINCT pay_year) AS pay_years, MIN(pay_year) AS pay_year, COUNT(DISTINCT LOWER(department)) AS departments,
  COUNT(DISTINCT LOWER(department)) FILTER (WHERE LOWER(department) IN (SELECT department_key FROM budget_departments)) AS budget_departments_matched
 FROM pay WHERE employer IS NOT NULL GROUP BY 1 HAVING COUNT(*) >= ${PAY_MINIMUM_POSITIONS}
), chosen AS (
 SELECT *, budget_departments_matched >= ${BUDGET_DEPARTMENTS_MATCHED} AND 2 * budget_departments_matched >= departments AS budget_city
 FROM employers ORDER BY budget_city DESC, positions DESC, employer LIMIT ${PAY_EMPLOYERS}
), department_pay AS (
 SELECT employer, department, COUNT(*) AS department_positions, SUM(regular_pay) AS department_regular_pay, SUM(overtime_pay) AS department_overtime_pay
 FROM pay WHERE department IS NOT NULL AND employer IN (SELECT employer FROM chosen) GROUP BY 1, 2
), ranked AS (
 SELECT *, ROW_NUMBER() OVER (PARTITION BY employer ORDER BY department_overtime_pay DESC NULLS LAST, department) AS position, ${followed('department')} AS followed FROM department_pay
)
SELECT chosen.employer, positions, regular_pay, overtime_pay, high_overtime_positions, positions_without_overtime, pay_years, pay_year, departments, budget_departments_matched, budget_city,
 ranked.department, department_positions, department_regular_pay, department_overtime_pay, position, followed,
 same_year.adopted_overtime AS adopted_overtime_same_year, next_year.adopted_overtime AS adopted_overtime_next_year,
 (SELECT COUNT(*) FROM budget_overtime WHERE budget_overtime.fiscal_year = chosen.pay_year) AS overtime_departments_same_year,
 (SELECT COUNT(*) FROM budget_overtime WHERE budget_overtime.fiscal_year = chosen.pay_year + 1) AS overtime_departments_next_year
FROM chosen JOIN ranked ON ranked.employer = chosen.employer AND (position <= ${PAY_DEPARTMENTS_LISTED} OR followed)
 LEFT JOIN budget_overtime AS same_year ON budget_city AND same_year.department_key = LOWER(ranked.department) AND same_year.fiscal_year = chosen.pay_year
 LEFT JOIN budget_overtime AS next_year ON budget_city AND next_year.department_key = LOWER(ranked.department) AND next_year.fiscal_year = chosen.pay_year + 1
ORDER BY budget_city DESC, positions DESC, chosen.employer, position`,
  };
}

export function spendingQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const budget = budgetTable(schema);
  if (isPayTable(schema, dataset)) return [payQuery(schema, dataset, budget)];
  if (actualsTable(schema)?.snapshotId !== dataset.snapshotId) return [];
  return [...(budget ? [adoptedActualQuery(budget, dataset), overtimeQuery(budget, dataset)] : []), settlementQuery(dataset)];
}

const count = (value: unknown): number | null => {
  if (value == null || value === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};
const flag = (value: unknown) => value === true || value === 'true' || value === 1;
const shown = (value: number) => Math.round(value).toLocaleString('en-US');
const money = (value: number) => {
  const size = Math.abs(value);
  return `${value < 0 ? '-' : ''}$${size >= 1_000_000_000 ? `${(size / 1_000_000_000).toFixed(2)} billion` : size >= 1_000_000 ? `${(size / 1_000_000).toFixed(1)} million` : shown(size)}`;
};
const percent = (part: number, total: number) => `${(100 * part / total).toFixed(1)}%`;
const records = (result: SqlQueryResult) => result.rows.map((row) => Object.fromEntries(result.columns.map((name, index) => [name, row[index]])));
type Entry = { query: DiscoveryQuery; result?: SqlQueryResult };
/** A result cut short by the row limit would give short counts, so it gives none. */
const rowsOf = (results: Entry[], id: string) => {
  const entry = results.find((item) => item.query.id === id);
  return entry?.result?.rows.length && !entry.result.truncated ? { query: entry.query, rows: records(entry.result) } : null;
};

const belowAdopted = (adopted: number, actual: number) => adopted > 0 && (adopted - actual) * 100 >= BELOW_ADOPTED_PERCENT * adopted && adopted - actual >= BELOW_ADOPTED_DOLLARS;

function adoptedActualLeads(results: Entry[]): ServiceLead[] {
  const read = rowsOf(results, 'spending_adopted_actual');
  if (!read) return [];
  const { query, rows } = read;
  const covered = `fiscal ${count(rows[0].first_covered_year)} to ${count(rows[0].last_covered_year)}`;
  const leads: ServiceLead[] = [];
  const departments = new Map<string, Record<string, unknown>[]>();
  for (const row of rows) if (flag(row.followed)) departments.set(String(row.department), [...(departments.get(String(row.department)) ?? []), row]);
  for (const [department, years] of departments) {
    const lines: string[] = []; const reported: { year: number; adopted: number; actual: number }[] = [];
    for (const row of years) {
      const year = count(row.fiscal_year)!; const adopted = count(row.adopted_expense); const actual = count(row.actual_expense);
      // Absence is said as absence: no rows, no expense rows and an unreadable amount are three
      // different facts, and none of them is spending of zero.
      if (count(row.actual_rows) === null) lines.push(`FY ${year}: ${adopted === null ? 'an adopted amount' : `${money(adopted)} adopted`} and no rows in the actuals table.`);
      else if (adopted === null || (count(row.unreadable_adopted) ?? 0) > 0) lines.push(`FY ${year}: not compared (the adopted amount could not be read in full).`);
      else if ((count(row.actual_expense_rows) ?? 0) === 0 || actual === null) lines.push(`FY ${year}: ${money(adopted)} adopted; the actuals table has rows for the department and none in an expense account, so no actual expense is stated.`);
      else if ((count(row.unreadable_actual) ?? 0) > 0) lines.push(`FY ${year}: not compared (the actual amount could not be read in full).`);
      else {
        const difference = adopted - actual;
        lines.push(`FY ${year}: ${money(adopted)} adopted, ${money(actual)} actual${adopted > 0 && difference !== 0 ? ` (${percent(Math.abs(difference), adopted)} ${difference > 0 ? 'below' : 'above'})` : ''}.`);
        if (belowAdopted(adopted, actual)) reported.push({ year, adopted, actual });
      }
    }
    const latest = reported.at(-1);
    if (!latest) continue;
    leads.push({ key: `spending_below_adopted:${department}`, priority: 97, query,
      title: `${department}: actual expense ${money(latest.actual)} against ${money(latest.adopted)} adopted in FY ${latest.year}, ${percent(latest.adopted - latest.actual, latest.adopted)} below${reported.length > 1 ? `; ${reported.length - 1} other year${reported.length > 2 ? 's' : ''} at least ${BELOW_ADOPTED_PERCENT}% below` : ''}`,
      detail: `Operating expense of ${department}, adopted against actual, for every fiscal year both tables cover (${covered}). ${lines.join(' ')} At least ${BELOW_ADOPTED_PERCENT}% and $${shown(BELOW_ADOPTED_DOLLARS)} below adopted: ${reported.map((item) => `FY ${item.year} (${money(item.adopted - item.actual)})`).join(', ')}. ${ADOPTED_LIMITS}`,
      question: `What does the revised budget of ${department} for FY ${latest.year} show, and where did the ${money(latest.adopted - latest.actual)} not spent go? The year-end budget monitoring report and the Council's mid-year budget amendments would show a transfer or a carry-forward.`,
    });
  }
  // An adopted sum of zero is a department the budget lists without an amount to spend.
  const absent = rows.filter((row) => count(row.actual_rows) === null && (count(row.adopted_expense) ?? 0) > 0).sort((a, b) => count(b.fiscal_year)! - count(a.fiscal_year)! || (count(b.adopted_expense) ?? 0) - (count(a.adopted_expense) ?? 0));
  if (absent.length) {
    const first = absent[0]; const named = (row: Record<string, unknown>) => `${String(row.department)}, FY ${count(row.fiscal_year)} (${money(count(row.adopted_expense)!)} adopted)`;
    leads.push({ key: 'spending_no_actual_rows', priority: 93, query,
      title: absent.length === 1 ? `${String(first.department)} has an adopted amount for FY ${count(first.fiscal_year)} and no rows in the actuals table`
        : `${shown(absent.length)} department-years have an adopted amount and no rows in the actuals table, the latest ${String(first.department)} in FY ${count(first.fiscal_year)}`,
      detail: `Of ${shown(count(rows[0].department_years) ?? 0)} department-years with an adopted expense amount in the years both tables cover (${covered}), ${shown(absent.length)} ${absent.length === 1 ? 'has' : 'have'} no row of any account under that department name in the actuals table: ${absent.slice(0, NO_ACTUAL_ROWS_LISTED).map(named).join('; ')}${absent.length > NO_ACTUAL_ROWS_LISTED ? `; and ${shown(absent.length - NO_ACTUAL_ROWS_LISTED)} more` : ''}. No rows is not spending of zero: the table is silent on these departments in those years. A department renamed or merged during the year can be recorded under another name, and a reserve is appropriated to be transferred, not spent under its own name. ${ADOPTED_LIMITS}`,
      question: `Under which department name, if any, was the amount adopted for ${String(first.department)} in FY ${count(first.fiscal_year)} recorded as spent? The adopted budget volume and the year-end financial report for that year would show a reorganisation or a transfer.`,
    });
  }
  return leads;
}

const aboveAdoptedOvertime = (adopted: number, actual: number) => adopted > 0 && (actual - adopted) * 100 >= ABOVE_ADOPTED_OVERTIME_PERCENT * adopted && actual - adopted >= ABOVE_ADOPTED_OVERTIME_DOLLARS;

function overtimeLeads(results: Entry[]): ServiceLead[] {
  const read = rowsOf(results, 'spending_overtime_actual');
  if (!read) return [];
  const { query, rows } = read;
  const covered = `fiscal ${count(rows[0].first_covered_year)} to ${count(rows[0].last_covered_year)}`;
  const departments = new Map<string, Record<string, unknown>[]>();
  for (const row of rows) if (row.department != null) departments.set(String(row.department), [...(departments.get(String(row.department)) ?? []), row]);
  const leads: ServiceLead[] = [];
  for (const [department, years] of departments) {
    const lines: string[] = []; const reported: { year: number; adopted: number; actual: number; accounts: string }[] = [];
    for (const row of years) {
      const year = count(row.fiscal_year)!; const adopted = count(row.adopted_overtime); const actual = count(row.actual_overtime);
      if (count(row.actual_rows) === null) lines.push(`FY ${year}: ${adopted === null ? 'an adopted overtime line' : `${money(adopted)} adopted overtime`} and no rows in the actuals table.`);
      else if (adopted === null || (count(row.unreadable_adopted) ?? 0) > 0) lines.push(`FY ${year}: not compared (the adopted overtime amount could not be read in full).`);
      else if ((count(row.actual_ot_rows) ?? 0) === 0) lines.push(`FY ${year}: ${money(adopted)} adopted overtime; the actuals table has rows for the department and no overtime rows recorded, so no actual overtime is stated.`);
      else if (actual === null || (count(row.unreadable_actual) ?? 0) > 0) lines.push(`FY ${year}: not compared (the actual overtime amount could not be read in full).`);
      else {
        const difference = actual - adopted;
        lines.push(`FY ${year}: ${money(adopted)} adopted overtime, ${money(actual)} actual${adopted > 0 && difference !== 0 ? ` (${percent(Math.abs(difference), adopted)} ${difference > 0 ? 'above' : 'below'})` : ''}.`);
        if (aboveAdoptedOvertime(adopted, actual)) reported.push({ year, adopted, actual, accounts: String(row.actual_accounts ?? '') });
      }
    }
    const latest = reported.at(-1);
    if (!latest) continue;
    const scope = flag(years[0].followed) ? `for every fiscal year both tables cover with an adopted overtime line (${covered})`
      : `for the years both tables cover (${covered}) in which it is among the ${OVERTIME_DEPARTMENT_YEARS_LISTED} department-years with most actual overtime`;
    leads.push({ key: `spending_overtime_above_adopted:${department}`, priority: 95, query,
      title: `${department}: actual overtime ${money(latest.actual)} against ${money(latest.adopted)} adopted in FY ${latest.year}, ${percent(latest.actual - latest.adopted, latest.adopted)} above${reported.length > 1 ? `; ${reported.length - 1} other year${reported.length > 2 ? 's' : ''} at least ${ABOVE_ADOPTED_OVERTIME_PERCENT}% above` : ''}`,
      detail: `Overtime of ${department}, adopted against actual, ${scope}. ${lines.join(' ')} At least ${ABOVE_ADOPTED_OVERTIME_PERCENT}% and $${shown(ABOVE_ADOPTED_OVERTIME_DOLLARS)} above adopted overtime: ${reported.map((item) => `FY ${item.year} (${money(item.actual - item.adopted)})`).join(', ')}; both are fixed yardsticks, not a statistical test. Actual overtime in FY ${latest.year} is in ${latest.accounts}. ${OVERTIME_LIMITS}`,
      question: `What does the revised budget of ${department} for FY ${latest.year} show for overtime, and which units and assignments account for the ${money(latest.actual - latest.adopted)} above the adopted line? The department's overtime reports and the year-end budget monitoring report would show the transfers and the reasons.`,
    });
  }
  return leads;
}

function settlementLeads(results: Entry[]): ServiceLead[] {
  const read = rowsOf(results, 'spending_settlements');
  if (!read) return [];
  const years = new Map<number, Record<string, unknown>[]>();
  for (const row of read.rows) {
    const year = count(row.fiscal_year);
    if (year !== null) years.set(year, [...(years.get(year) ?? []), row]);
  }
  const recorded = [...years].filter(([, rows]) => count(rows[0].departments_reporting) !== null);
  if (!recorded.length) return [];
  const [latestYear, latestRows] = recorded[0]; const latest = latestRows[0];
  const from = (row: Record<string, unknown>) => `${shown(count(row.departments_reporting)!)} of ${shown(count(row.departments_in_table) ?? 0)} departments`;
  const lines = [...years].map(([year, rows]) => {
    const total = rows[0];
    if (count(total.departments_reporting) === null) return `FY ${year}: no rows in these accounts (${shown(count(total.departments_in_table) ?? 0)} departments in the table), which is not an amount of zero.`;
    const unreadable = count(total.unreadable_amounts) ?? 0;
    const named = rows.filter((row) => row.department != null && count(row.department_amount) !== null).map((row) => `${String(row.department)} ${money(count(row.department_amount)!)}`);
    return `FY ${year}: ${count(total.year_amount) === null ? 'no readable amount' : money(count(total.year_amount)!)} in ${shown(count(total.year_rows) ?? 0)} rows from ${from(total)}${unreadable ? `, ${shown(unreadable)} of them unreadable and in no sum` : ''}${named.length ? `; ${named.join(', ')}` : ''}.`;
  });
  const reporting = recorded.map(([, rows]) => count(rows[0].departments_reporting)!);
  const negative = read.rows.some((row) => (count(row.year_amount) ?? 0) < 0 || (count(row.department_amount) ?? 0) < 0);
  return [{ key: 'spending_settlements', priority: 92, query: read.query,
    title: `Settlement and public-liability claim accounts: ${count(latest.year_amount) === null ? 'rows' : money(count(latest.year_amount)!)} recorded by ${from(latest)} in FY ${latestYear}`,
    detail: `Rows booked to the Negotiated Settlement and public-liability claim accounts in the operating actuals, for the ${years.size} latest fiscal year${years.size > 1 ? 's' : ''} in the table. ${lines.join(' ')} Departments named are the ${SETTLEMENT_DEPARTMENTS_LISTED} largest each year, with ${FOLLOWED_TEXT} when they have a row; a department not named may still have one.${Math.min(...reporting) !== Math.max(...reporting) || recorded.length < years.size ? ` The number of departments with a row in these accounts differs between the years listed (${recorded.length < years.size ? 'none' : shown(Math.min(...reporting))} to ${shown(Math.max(...reporting))}), so the yearly figures are not compared: a larger figure can be a change in where the City records a payment, not in what it paid.` : ' The yearly figures are stated, not compared: the table does not say whether the City recorded these payments the same way each year.'}${negative ? ' A negative figure is as recorded: a reversal or a recovery booked to the account.' : ''} The accounts do not name the cases, the claimants or the reasons, and a payment made from the Public Liability fund can be recorded under another department.`,
    question: `Which claims and cases make up the FY ${latestYear} figure, and why do the departments recording these accounts differ from year to year? The Risk Management annual report and the Council's settlement resolutions for the year would show it.`,
  }];
}

function payLeads(results: Entry[]): ServiceLead[] {
  const read = rowsOf(results, 'spending_pay_overtime');
  if (!read) return [];
  const employers = new Map<string, Record<string, unknown>[]>();
  for (const row of read.rows) if (typeof row.employer === 'string') employers.set(row.employer, [...(employers.get(row.employer) ?? []), row]);
  const leads: ServiceLead[] = [];
  for (const [employer, rows] of employers) {
    const total = rows[0]; const positions = count(total.positions); const overtime = count(total.overtime_pay); const regular = count(total.regular_pay);
    if (positions === null || positions < PAY_MINIMUM_POSITIONS || overtime === null || overtime <= 0) continue;
    // More than one year in the table: a sum over them is not a year's overtime, and no fiscal year matches.
    const year = count(total.pay_years) === 1 ? count(total.pay_year) : null;
    const period = year === null ? 'the years in the table' : `calendar ${year}`;
    const budgetCity = flag(total.budget_city) && year !== null;
    const adopted = (row: Record<string, unknown>, column: string, fiscalYear: number) => {
      const value = count(row[`adopted_overtime_${column}`]);
      if (value !== null) return `${money(value)} adopted for FY ${fiscalYear}`;
      return (count(row[`overtime_departments_${column}`]) ?? 0) > 0 ? `no adopted overtime row for FY ${fiscalYear}` : `no adopted overtime rows in the budget table for FY ${fiscalYear}`;
    };
    const lines = rows.filter((row) => row.department != null).map((row) => {
      const paid = count(row.department_overtime_pay); const base = count(row.department_regular_pay); const held = count(row.department_positions) ?? 0;
      const share = paid !== null && base !== null && base > 0 && held >= PAY_DEPARTMENT_POSITIONS ? `, ${percent(paid, base)} of its regular pay of ${money(base)}` : '';
      return `${String(row.department)}: ${paid === null ? 'no overtime figure recorded' : `${money(paid)} overtime`}${share} (${shown(held)} positions)${budgetCity ? `; ${adopted(row, 'same_year', year!)}, ${adopted(row, 'next_year', year! + 1)}` : ''}.`;
    });
    const high = count(total.high_overtime_positions) ?? 0; const without = count(total.positions_without_overtime) ?? 0;
    leads.push({ key: `spending_pay_overtime:${employer}`, priority: budgetCity ? 96 : 91, query: read.query,
      title: `${employer}: ${money(overtime)} overtime paid in ${period}${regular !== null && regular > 0 ? `, ${percent(overtime, regular)} of regular pay` : ''}; ${shown(high)} positions with $${shown(HIGH_OVERTIME_DOLLARS)} or more`,
      detail: `${employer} reported ${shown(positions)} positions to the State Controller for ${period}: ${regular === null ? 'no readable regular pay' : `${money(regular)} regular pay`} and ${money(overtime)} overtime. ${shown(high)} positions had overtime of $${shown(HIGH_OVERTIME_DOLLARS)} or more.${without ? ` ${shown(without)} positions record no overtime figure and add nothing to the sums.` : ''} By department: ${lines.join(' ')}${budgetCity ? ` ${shown(count(total.budget_departments_matched) ?? 0)} of this employer's ${shown(count(total.departments) ?? 0)} department names are the budget table's own, so it is read as the budget's city. The pay file covers calendar ${year} and the budget fiscal years from 1 July to 30 June: calendar ${year} is the second half of FY ${year} and the first half of FY ${year! + 1}, so the adopted amounts are beside the overtime paid, not for the same period. Adopted is the original appropriation, not the revised budget.` : ' No adopted overtime is set beside these figures: no imported budget table is read as this employer\'s.'} A row is a position held, not a person, and the file has no names. Positions with overtime above regular pay are not counted, because a low regular figure can be part of a year in the position. Nothing here says why the overtime was worked or whether it was authorised.`,
      question: `Which units and assignments account for the overtime in ${String(rows.find((row) => row.department != null)?.department ?? employer)}, and how many of its budgeted positions were vacant in ${year ?? 'those years'}? The department's overtime reports and vacancy reports to the budget committee would show it.`,
    });
  }
  return leads;
}

export function spendingLeads(results: Entry[]): ServiceLead[] {
  return [...adoptedActualLeads(results), ...overtimeLeads(results), ...settlementLeads(results), ...payLeads(results)];
}
