import type { AnalyticalDatasetSummary, AnalyticalSchema } from '../types';

const keys = ['report_fy', 'fund_number', 'funds_center_number', 'account_number'] as const;
const numeric = new Set(['INTEGER', 'NUMBER', 'DECIMAL']);
const identifier = (value: string) => `"${value.replace(/"/g, '""')}"`;
const literal = (value: string) => `'${value.replace(/'/g, "''")}'`;

export function budgetComparisonCandidates(schema: AnalyticalSchema, role: 'budget' | 'actuals'): AnalyticalDatasetSummary[] {
  return schema.datasets.filter((dataset) => {
    const columns = schema.columns.filter((column) => column.tableName === dataset.tableName);
    if (role === 'actuals' && columns.some((column) => column.columnName === 'budget_cycle')) return false;
    return [...keys, 'dept_name', ...(role === 'budget' ? ['budget_cycle'] : [])].every((key) => columns.some((column) => column.columnName === key))
      && columns.some((column) => column.columnName === 'amount' && numeric.has(column.logicalType));
  });
}

export interface BudgetComparisonSelection {
  budgetSnapshotId: string;
  actualsSnapshotId: string;
  fiscalYear: string;
  cycle: 'adopted' | 'proposed';
  accountClass: 'expense' | 'revenue';
  fundNumber?: string;
}

export function buildBudgetComparison(schema: AnalyticalSchema, selection: BudgetComparisonSelection): string {
  const budget = budgetComparisonCandidates(schema, 'budget').find((d) => d.snapshotId === selection.budgetSnapshotId);
  const actuals = budgetComparisonCandidates(schema, 'actuals').find((d) => d.snapshotId === selection.actualsSnapshotId);
  if (!budget || !actuals) throw new Error('Choose budget and actuals snapshots with the City account-level schema and numeric amounts.');
  if (budget.snapshotId === actuals.snapshotId) throw new Error('Choose two different snapshots for budget and actuals.');
  const year = selection.fiscalYear.trim();
  if (!/^\d{2}(?:\d{2})?$/.test(year)) throw new Error('Enter the fiscal-year code used in both files, such as 25 or 2025.');
  if (!['adopted', 'proposed'].includes(selection.cycle) || !['expense', 'revenue'].includes(selection.accountClass)) throw new Error('Choose a supported budget cycle and account class.');
  const filter = `CAST(report_fy AS VARCHAR) = ${literal(year)} AND account_number LIKE '${selection.accountClass === 'expense' ? '5' : '4'}%'${selection.fundNumber?.trim() ? ` AND fund_number = ${literal(selection.fundNumber.trim())}` : ''}`;
  const projectKeys = keys.map((key) => `CAST(${identifier(key)} AS VARCHAR) AS ${identifier(key)}`).join(', ');
  const aggregate = (table: string, role: 'budget' | 'actuals') => `SELECT ${projectKeys},
    STRING_AGG(DISTINCT dept_name, ' | ' ORDER BY dept_name) AS ${role}_departments,
    SUM(amount) AS ${role}_amount, COUNT(*) AS ${role}_rows,
    COUNT(*) - COUNT(amount) AS ${role}_missing_amounts
  FROM ${identifier(table)}
  WHERE ${filter}${role === 'budget' ? ` AND LOWER(TRIM(budget_cycle)) = ${literal(selection.cycle)}` : ''}
  GROUP BY 1, 2, 3, 4`;
  return `WITH budget AS (
  ${aggregate(budget.tableName, 'budget')}
), actuals AS (
  ${aggregate(actuals.tableName, 'actuals')}
)
SELECT ${keys.map((key) => `COALESCE(b.${key}, a.${key}) AS ${key}`).join(', ')},
  b.budget_departments, a.actuals_departments,
  b.budget_amount, a.actuals_amount, b.budget_rows, a.actuals_rows,
  b.budget_missing_amounts, a.actuals_missing_amounts,
  CASE WHEN b.budget_rows IS NULL THEN 'Missing budget baseline'
       WHEN a.actuals_rows IS NULL THEN 'Missing actuals'
       WHEN b.budget_missing_amounts > 0 OR a.actuals_missing_amounts > 0 THEN 'Incomplete amounts'
       ELSE 'Matched' END AS comparison_status,
  CASE WHEN b.budget_missing_amounts = 0 AND a.actuals_missing_amounts = 0
       THEN a.actuals_amount - b.budget_amount END AS actual_minus_budget,
  CASE WHEN b.budget_missing_amounts = 0 AND a.actuals_missing_amounts = 0
       THEN ROUND(100 * (a.actuals_amount - b.budget_amount) / NULLIF(ABS(b.budget_amount), 0), 2) END AS variance_percent
FROM budget b FULL OUTER JOIN actuals a
  ON ${keys.map((key) => `b.${key} = a.${key}`).join(' AND ')}
ORDER BY CASE WHEN comparison_status = 'Matched' THEN 0 ELSE 1 END,
  ABS(actual_minus_budget) DESC NULLS LAST, comparison_status, fund_number, funds_center_number, account_number`;
}

/** The columns `buildBudgetComparison` returns that the difference chart reads. */
const COMPARISON_COLUMNS = [...keys.slice(1), 'budget_amount', 'actuals_amount', 'comparison_status', 'actual_minus_budget', 'variance_percent'] as const;

export interface BudgetComparisonLine {
  key: string;
  label: string;
  departments: string | null;
  status: string;
  fund: string | null;
  center: string | null;
  account: string | null;
  /** Decimal strings as the engine returned them; null where the side has no complete amount. */
  budget: string | null;
  actuals: string | null;
  difference: string | null;
  variancePercent: string | null;
}

/**
 * A cell as a decimal string. DECIMAL arrives as a string and is kept exactly; a DOUBLE arrives as a
 * number and is printed to cents. Anything else is not an amount.
 */
function decimalText(cell: unknown): string | null {
  if (typeof cell === 'string') return /^-?\d+(?:\.\d+)?$/.test(cell.trim()) ? cell.trim() : null;
  if (typeof cell === 'number' && Number.isFinite(cell)) return cell.toFixed(2);
  return null;
}

function cellText(cell: unknown): string | null {
  if (cell === null || cell === undefined) return null;
  const value = String(cell).trim();
  return value ? value : null;
}

/**
 * The comparison lines of a SQL result that carries the budget-comparison columns, or null for any
 * other result. Only a 'Matched' line keeps a difference; every other status is a line that was not
 * compared, whatever its difference cell holds.
 */
export function readBudgetComparison(result: { columns: string[]; rows: unknown[][] }): BudgetComparisonLine[] | null {
  const index = new Map(result.columns.map((column, i) => [column, i] as const));
  if (!COMPARISON_COLUMNS.every((column) => index.has(column))) return null;
  const at = (row: unknown[], column: string) => {
    const i = index.get(column);
    return i === undefined ? null : row[i];
  };
  return result.rows.map((row, i) => {
    const fund = cellText(at(row, 'fund_number'));
    const center = cellText(at(row, 'funds_center_number'));
    const account = cellText(at(row, 'account_number'));
    const status = cellText(at(row, 'comparison_status')) ?? 'Status not recorded';
    const matched = status === 'Matched';
    return {
      key: `${i}:${fund ?? ''}:${center ?? ''}:${account ?? ''}`,
      label: `Fund ${fund ?? '?'} / ${center ?? '?'} / ${account ?? '?'}`,
      departments: cellText(at(row, 'budget_departments')) ?? cellText(at(row, 'actuals_departments')),
      status,
      fund,
      center,
      account,
      budget: decimalText(at(row, 'budget_amount')),
      actuals: decimalText(at(row, 'actuals_amount')),
      difference: matched ? decimalText(at(row, 'actual_minus_budget')) : null,
      variancePercent: matched ? decimalText(at(row, 'variance_percent')) : null,
    };
  });
}

/** One comparison line's row, for the SQL editor: the comparison query filtered to its keys. */
export function budgetComparisonLineQuery(comparisonSql: string, line: Pick<BudgetComparisonLine, 'fund' | 'center' | 'account'>): string {
  const condition = ([['fund_number', line.fund], ['funds_center_number', line.center], ['account_number', line.account]] as const)
    .map(([column, value]) => (value === null ? `${column} IS NULL` : `${column} = ${literal(value)}`))
    .join(' AND ');
  return `SELECT * FROM (\n${comparisonSql}\n) AS comparison\nWHERE ${condition}`;
}
