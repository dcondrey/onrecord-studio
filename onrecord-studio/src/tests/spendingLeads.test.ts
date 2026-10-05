import { describe, expect, it } from 'vitest';
import fixture from '../../src-tauri/tests/fixtures/spending-leads.json';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import { storedLeads } from '../utils/automaticDiscovery';
import { dataGaps } from '../utils/dataGaps';
import { actualsTable, PAY_MINIMUM_POSITIONS, spendingLeads, spendingQueries } from '../utils/spendingLeads';

const schema = fixture.schema as AnalyticalSchema;
const [actuals, budget, pay] = schema.datasets;
const [comparedQuery, settledQuery, payQuery] = fixture.queries;
const without = (...snapshotIds: string[]): AnalyticalSchema => ({ ...schema, datasets: schema.datasets.filter((dataset) => !snapshotIds.includes(dataset.snapshotId)) });
const result = (rows: Record<string, unknown>[], truncated = false): SqlQueryResult => {
  const columns = [...new Set(rows.flatMap((row) => Object.keys(row)))];
  return { columns, rowCount: rows.length, elapsedMs: 1, truncated, provenance: [], rows: rows.map((row) => columns.map((name) => row[name] ?? null)) };
};

const span = { first_covered_year: 2022, last_covered_year: 2023, department_years: 8 };
const year = (adopted: number | null, actual: number | null, change: Record<string, unknown> = {}) => ({ department: 'Homelessness Strategies & Solutions', fiscal_year: 2022, adopted_expense: adopted, unreadable_adopted: 0,
  actual_rows: 5, actual_expense_rows: 4, actual_expense: actual, unreadable_actual: 0, followed: true, ...span, ...change });
const compared = (rows: Record<string, unknown>[], truncated = false) => spendingLeads([{ query: comparedQuery, result: result(rows, truncated) }]);

describe('adopted against actual operating expense', () => {
  it('reports a year 10% below adopted, not 9.999999%', () => {
    const below = (actual: number) => compared([year(100_000_000, actual)]);
    expect(below(90_000_001)).toEqual([]);
    expect(below(90_000_000)).toHaveLength(1);
    expect(below(89_999_999)).toHaveLength(1);
  });

  it('reports a year $1,000,000 below adopted, not $999,999', () => {
    const below = (actual: number) => compared([year(5_000_000, actual)]);
    expect(below(4_000_001)).toEqual([]);
    expect(below(4_000_000)).toHaveLength(1);
    expect(below(3_999_999)).toHaveLength(1);
    // Spending above adopted is the financial workspace's measure, not this one.
    expect(compared([year(5_000_000, 9_000_000)])).toEqual([]);
  });

  it('lists every covered year of the department and states the limits of an adopted amount', () => {
    const [lead] = compared([year(18_071_824, 9_815_698), year(25_157_607, 22_920_122, { fiscal_year: 2023 })]);
    expect(lead.key).toBe('spending_below_adopted:Homelessness Strategies & Solutions');
    expect(lead.title).toBe('Homelessness Strategies & Solutions: actual expense $9.8 million against $18.1 million adopted in FY 2022, 45.7% below');
    expect(lead.detail).toContain('FY 2022: $18.1 million adopted, $9.8 million actual (45.7% below). FY 2023: $25.2 million adopted, $22.9 million actual (8.9% below).');
    expect(lead.detail).toContain('At least 10% and $1,000,000 below adopted: FY 2022 ($8.3 million).');
    expect(lead.detail).toContain('Adopted is the original appropriation, not the revised budget: a transfer or a change made during the year is in neither figure.');
    expect(lead.detail).toContain('nothing here says the money was put to another use');
  });

  it('says a department-year has no actual rows, or no readable amount, and never shows it as zero', () => {
    const none = { actual_rows: null, actual_expense_rows: null, actual_expense: null, unreadable_actual: null };
    const leads = compared([year(18_000_000, 9_000_000), year(51_000_000, null, { fiscal_year: 2023, ...none }),
      year(1_000_000, null, { department: 'Business Office', followed: false, ...none })]);
    expect(leads.map((lead) => lead.key)).toEqual(['spending_below_adopted:Homelessness Strategies & Solutions', 'spending_no_actual_rows']);
    expect(leads[0].detail).toContain('FY 2023: $51.0 million adopted and no rows in the actuals table.');
    expect(leads[1].title).toBe('2 department-years have an adopted amount and no rows in the actuals table, the latest Homelessness Strategies & Solutions in FY 2023');
    expect(leads[1].detail).toContain('Of 8 department-years with an adopted expense amount in the years both tables cover (fiscal 2022 to 2023), 2 have no row of any account');
    expect(leads[1].detail).toContain('Business Office, FY 2022 ($1.0 million adopted)');
    expect(leads[1].detail).toContain('No rows is not spending of zero');
    for (const lead of leads) expect(lead.detail).not.toMatch(/\$0\b| 0\.0%|100\.0% below/);
    // Adopted rows that sum to zero are not an adopted amount.
    expect(compared([year(0, null, { department: 'Retirement', followed: false, ...none })])).toEqual([]);
    // No actual rows alone is never a year "below adopted".
    expect(compared([year(51_000_000, null, none)]).map((lead) => lead.key)).toEqual(['spending_no_actual_rows']);
    // Rows in revenue accounts only, or an unreadable amount, give no actual expense to compare.
    expect(compared([year(51_000_000, null, { actual_expense_rows: 0 })])).toEqual([]);
    expect(compared([year(51_000_000, 1_000_000, { unreadable_actual: 1 })])).toEqual([]);
    expect(compared([year(51_000_000, 1_000_000, { unreadable_adopted: 1 })])).toEqual([]);
  });

  it('gives nothing from a result the row limit cut short', () => {
    expect(compared([year(18_000_000, 9_000_000)], true)).toEqual([]);
  });

  it('needs both tables, tells them apart by budget_cycle, and is offered the actuals when only a budget is imported', () => {
    expect(actualsTable(schema)?.snapshotId).toBe('actuals');
    expect(actualsTable(without('actuals'))).toBeNull();
    expect(spendingQueries(schema, actuals).map((query) => query.id)).toEqual(['spending_adopted_actual', 'spending_settlements']);
    expect(spendingQueries(without('budget'), actuals).map((query) => query.id)).toEqual(['spending_settlements']);
    expect(spendingQueries(schema, budget)).toEqual([]);
    const fetches = (catalog: AnalyticalSchema) => dataGaps(catalog, {}, '2026-10-05T12:00:00Z').flatMap((gap) => gap.fetch ?? []);
    expect(fetches(without('actuals'))).toEqual([{ dataset: 'OPERATING_ACTUALS', year: null }]);
    expect(fetches(schema)).toEqual([]);
    expect(fetches(without('budget', 'actuals'))).toEqual([]);
  });
});

describe('settlement and claim accounts', () => {
  const row = (fiscalYear: number, reporting: number | null, amount: number | null, department: string | null = null, departmentAmount: number | null = null) => ({ fiscal_year: fiscalYear, departments_in_table: 63,
    departments_reporting: reporting, year_amount: amount, year_rows: reporting === null ? null : reporting * 2, unreadable_amounts: reporting === null ? null : 0, department, department_amount: departmentAmount, position: department ? 1 : null });
  const settled = (rows: Record<string, unknown>[]) => spendingLeads([{ query: settledQuery, result: result(rows) }]);

  it('states each year with the departments reporting and compares no year with another', () => {
    const [lead] = settled([row(2023, 38, 24_564_162, 'Police', 9_000_000), row(2022, 1, 325_370, 'Risk Management', 325_370), row(2021, 1, -414_126, 'Risk Management', -414_126)]);
    expect(lead.title).toBe('Settlement and public-liability claim accounts: $24.6 million recorded by 38 of 63 departments in FY 2023');
    expect(lead.detail).toContain('FY 2023: $24.6 million in 76 rows from 38 of 63 departments; Police $9.0 million.');
    expect(lead.detail).toContain('FY 2021: -$414,126 in 2 rows from 1 of 63 departments; Risk Management -$414,126.');
    expect(lead.detail).toContain('differs between the years listed (1 to 38), so the yearly figures are not compared');
    expect(lead.detail).toContain('A negative figure is as recorded');
    expect(`${lead.title} ${lead.detail}`).not.toMatch(/\b(up|down|rose|fell|increase|decrease|more than|times)\b/i);
  });

  it('says a year has no rows in these accounts, never zero, and is silent when no year has any', () => {
    const [lead] = settled([row(2023, null, null), row(2022, 2, 50_000, 'Library', 40_000)]);
    expect(lead.title).toContain('in FY 2022');
    expect(lead.detail).toContain('FY 2023: no rows in these accounts (63 departments in the table), which is not an amount of zero.');
    expect(lead.detail).not.toMatch(/\$0\b/);
    expect(settled([row(2023, null, null)])).toEqual([]);
  });
});

describe('overtime in the State Controller pay file', () => {
  const employer = { employer: 'San Diego', positions: 14_570, regular_pay: 1_048_215_357, overtime_pay: 160_052_249, high_overtime_positions: 211, positions_without_overtime: 0, pay_years: 1, pay_year: 2024,
    departments: 60, budget_departments_matched: 56, budget_city: true, overtime_departments_same_year: 40, overtime_departments_next_year: 40 };
  const department = (name: string, positions: number, regular: number, overtime: number, same: number | null, next: number | null, change: Record<string, unknown> = {}) => ({ ...employer, department: name,
    department_positions: positions, department_regular_pay: regular, department_overtime_pay: overtime, adopted_overtime_same_year: same, adopted_overtime_next_year: next, ...change });
  const paid = (rows: Record<string, unknown>[]) => spendingLeads([{ query: payQuery, result: result(rows) }]);

  it('is recognised by its headers and needs no budget table', () => {
    expect(spendingQueries(schema, pay).map((query) => query.id)).toEqual(['spending_pay_overtime']);
    expect(spendingQueries(without('budget'), pay)[0].method).toContain('No operating budget table is imported, so no adopted overtime is shown.');
    const renamed = { ...schema, columns: schema.columns.filter((column) => column.columnName !== 'overtimepay') };
    expect(spendingQueries(renamed, pay)).toEqual([]);
  });

  it('sets overtime beside both fiscal years the calendar year spans and says they are not the same period', () => {
    const [lead] = paid([department('Fire-Rescue', 1652, 100_448_512, 61_513_869, 48_442_491, 44_805_463), department('Homelessness Strategies & Solutions', 16, 1_962_029, 2005, null, null)]);
    expect(lead.title).toBe('San Diego: $160.1 million overtime paid in calendar 2024, 15.3% of regular pay; 211 positions with $100,000 or more');
    expect(lead.detail).toContain('Fire-Rescue: $61.5 million overtime, 61.2% of its regular pay of $100.4 million (1,652 positions); $48.4 million adopted for FY 2024, $44.8 million adopted for FY 2025.');
    // Sixteen positions are too few for a share; no adopted overtime row is not an adopted amount of zero.
    expect(lead.detail).toContain('Homelessness Strategies & Solutions: $2,005 overtime (16 positions); no adopted overtime row for FY 2024, no adopted overtime row for FY 2025.');
    expect(lead.detail).toContain('calendar 2024 is the second half of FY 2024 and the first half of FY 2025, so the adopted amounts are beside the overtime paid, not for the same period');
    expect(lead.detail).toContain('Positions with overtime above regular pay are not counted, because a low regular figure can be part of a year in the position.');
    expect(lead.detail).not.toMatch(/\$0 adopted/);
  });

  it('shows no adopted amount for an employer not read as the budget\'s city, or for a table of several years', () => {
    const other = { employer: 'Oceanside', budget_city: false, budget_departments_matched: 8, departments: 16 };
    const [lead] = paid([department('Police', 300, 30_000_000, 6_000_000, null, null, other)]);
    expect(lead.detail).toContain('No adopted overtime is set beside these figures');
    expect(lead.detail).not.toContain('adopted for FY');
    const [several] = paid([department('Police', 300, 30_000_000, 6_000_000, 5_000_000, 5_000_000, { pay_years: 2 })]);
    expect(several.title).toContain('in the years in the table');
    expect(several.detail).not.toContain('adopted for FY');
    // A budget year with no adopted overtime row for any department is said as that.
    const [uncycled] = paid([department('Police', 300, 30_000_000, 6_000_000, 5_000_000, null, { overtime_departments_next_year: 0 })]);
    expect(uncycled.detail).toContain('$5.0 million adopted for FY 2024, no adopted overtime rows in the budget table for FY 2025.');
  });

  it(`describes an employer with ${PAY_MINIMUM_POSITIONS} positions, not ${PAY_MINIMUM_POSITIONS - 1}, and only one that paid overtime`, () => {
    const sized = (positions: number, overtime = 6_000_000) => paid([department('Police', 300, 30_000_000, 6_000_000, null, null, { positions, overtime_pay: overtime })]);
    expect(sized(PAY_MINIMUM_POSITIONS - 1)).toEqual([]);
    expect(sized(PAY_MINIMUM_POSITIONS)).toHaveLength(1);
    expect(sized(PAY_MINIMUM_POSITIONS + 1)).toHaveLength(1);
    expect(sized(500, 0)).toEqual([]);
  });
});

describe('spending leads as stored findings', () => {
  it('cites the budget table beside the table each lead was counted from', () => {
    const profile = (results: { query: typeof comparedQuery; result: SqlQueryResult }[]) => ({ fingerprint: 'f', checkedAt: 'now', results });
    const stored = storedLeads(schema, {
      actuals: profile([{ query: comparedQuery, result: result([year(18_000_000, 9_000_000)]) }]),
      pay: profile([{ query: payQuery, result: result([{ employer: 'San Diego', positions: 113, regular_pay: 10_110_000, overtime_pay: 2_921_999, high_overtime_positions: 2, pay_years: 1, pay_year: 2024, budget_city: true,
        department: 'Police', department_positions: 60, department_regular_pay: 6_000_000, department_overtime_pay: 1_419_999 }]) }]),
    });
    expect(stored.leads).toEqual([
      expect.objectContaining({ family: 'SPENDING_RECORDS', key: 'actuals:spending_below_adopted:Homelessness Strategies & Solutions', snapshotIds: ['actuals', 'budget'], sql: comparedQuery.sql }),
      expect.objectContaining({ family: 'SPENDING_RECORDS', key: 'pay:spending_pay_overtime:San Diego', snapshotIds: ['pay', 'budget'], sql: payQuery.sql }),
    ]);
  });
});
