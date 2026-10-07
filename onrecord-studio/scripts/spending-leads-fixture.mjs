import fs from 'node:fs';
import ts from 'typescript';
const modules = new Map();
function compile(name) {
  if (modules.has(name)) return modules.get(name);
  const source = fs.readFileSync(new URL(`../src/utils/${name}.ts`, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const module = { exports: {} };
  new Function('module', 'exports', 'require', compiled)(module, module.exports, (path) => compile(path.replace('./', '')));
  modules.set(name, module.exports);
  return module.exports;
}
const { spendingQueries } = compile('spendingLeads');
const quote = (value) => `"${String(value).replace(/"/g, '""')}"`;
const toCsv = (header, rows) => `${[header, ...rows].map((row) => row.map(quote).join(',')).join('\n')}\n`;

// Headers are the publishers' own: budget_operating_datasd.csv, actuals_operating_datasd.csv and
// the State Controller's <year>_City.csv.
const actualsHeader = ['amount', 'report_fy', 'fund_type', 'fund_number', 'dept_name', 'funds_center_number', 'account', 'account_number'];
const budgetHeader = [...actualsHeader.slice(0, 2), 'budget_cycle', ...actualsHeader.slice(2)];
const WAGES = ['Salaried Wages', '500011']; const REVENUE = ['Charges for Services', '420000']; const OVERTIME = ['Overtime Budgeted', '500128'];
// The actuals file can label an overtime account number otherwise; the number is what matches.
const PREMIUM = ['Premium Pay Sworn', '500128'];
// A file read with a fractional row types account_number as DOUBLE; 500128.0 is still 500128.
const OVERTIME_DOUBLE = ['Overtime Budgeted', '500128.0'];
const SETTLEMENT = ['Negotiated Settlement', '512182']; const CLAIM = ['Claims-Pub Liab Vehicle Body Injury', '512004']; const LEASE = ['Lease Settlement Expense', '512052'];
const homelessness = 'Homelessness Strategies & Solutions';
// Twenty departments the pay file's San Diego rows share with the budget, one position each.
const offices = ['City Attorney', 'City Auditor', 'City Clerk', 'City Treasurer', 'Communications', 'Compliance', 'Development Services', 'Library', 'Personnel',
  ...Array.from({ length: 9 }, (_, index) => `Council District ${index + 1}`)];
const adopted = (amount, fy, department, [account, number], cycle = 'adopted') => [amount, fy, cycle, 'General Fund', '100000', department, '1001000011', account, number];
// Fiscal 2024 is adopted and not in the actuals file, so it is not compared. A revenue account and
// a proposed line are not adopted expense. Environmental Services has no actual rows in fiscal
// 2023, and Business Office none in 2022. Overtime is adopted for fiscal 2022 and 2023 beside the
// pay file's 2024 and 2025: Police actual overtime is above adopted in 2022 and less than
// $1,000,000 above in 2023; Homelessness has rows and none in overtime; Environmental Services none.
// Transportation is not followed and is listed for its actual overtime.
const budget = toCsv(budgetHeader, [
  adopted(18_000_000, '22', homelessness, WAGES), adopted(9_999_999, '22', homelessness, REVENUE), adopted(25_000_000, '23', homelessness, WAGES),
  adopted(7_777_777, '23', homelessness, WAGES, 'proposed'), adopted(44_000_000, '24', homelessness, WAGES),
  adopted(100_000_000, '22', 'Police', WAGES), adopted(100_000_000, '23', 'Police', WAGES), adopted(4_000_000, '22', 'Police', OVERTIME), adopted(5_000_000, '23', 'Police', OVERTIME),
  adopted(300_000, '23', homelessness, OVERTIME), adopted(2_000_000, '23', 'Environmental Services', OVERTIME), adopted(49_000_000, '24', 'Police', OVERTIME), adopted(46_000_000, '25', 'Police', OVERTIME),
  adopted(1_000_000, '22', 'Transportation', OVERTIME),
  adopted(50_000_000, '22', 'Environmental Services', WAGES), adopted(51_000_000, '23', 'Environmental Services', WAGES),
  adopted(1_000_000, '22', 'Business Office', WAGES), adopted(5_000_000, '22', 'Library', WAGES),
  ...offices.map((office) => adopted(10_000, '24', office, WAGES))]);
const actual = (amount, fy, department, [account, number]) => [amount, fy, 'General Fund', '100000', department, '1001000011', account, number];
// Fiscal 2021 has rows and none in a settlement account; fiscal 2022 has one department's
// reversal; fiscal 2023 three departments. The lease settlement is another account.
const actuals = toCsv(actualsHeader, [
  actual(100, '21', 'Library', WAGES),
  actual(9_000_000, '22', homelessness, WAGES), actual(500, '22', homelessness, REVENUE), actual(101_000_000, '22', 'Police', WAGES),
  actual(49_000_000, '22', 'Environmental Services', WAGES), actual(1_000_000, '22', 'Library', WAGES), actual(-5_000, '22', 'Library', SETTLEMENT),
  actual(22_600_000, '23', homelessness, WAGES), actual(50_000, '23', homelessness, SETTLEMENT), actual(87_900_000, '23', 'Police', WAGES), actual(6_000_000, '22', 'Police', OVERTIME_DOUBLE),
  actual(2_500_000, '22', 'Transportation', OVERTIME),
  actual(3_000_000, '23', 'Police', OVERTIME), actual(2_600_000, '23', 'Police', PREMIUM),
  actual(2_000_000, '23', 'Police', SETTLEMENT), actual(100_000, '23', 'Police', CLAIM), actual(300_000, '23', 'Library', SETTLEMENT), actual(777, '23', 'Library', LEASE)]);

const payHeader = ['Year', 'EmployerType', 'EmployerName', 'DepartmentOrSubdivision', 'Position', 'ElectedOfficial', 'Judicial', 'OtherPositions', 'MinPositionSalary', 'MaxPositionSalary', 'ReportedBaseWage',
  'RegularPay', 'OvertimePay', 'LumpSumPay', 'OtherPay', 'TotalWages', 'DefinedBenefitPlanContribution', 'EmployeesRetirementCostCovered', 'DeferredCompensationPlan', 'HealthDentalVision',
  'TotalRetirementAndHealthContribution', 'PensionFormula', 'EmployerURL', 'EmployerPopulation', 'LastUpdatedDate', 'EmployerCounty', 'SpecialDistrictActivities', 'IncludesUnfundedLiability', 'SpecialDistrictType'];
const position = (employer, department, regular, overtime) => ['2024', 'City', employer, department, 'Officer', 'False', 'False', '', '50000', '120000', '', regular, overtime, '0', '0',
  overtime === '' ? regular : regular + overtime, '0', '0', '0', '0', '0', '2.0% @ 62', 'example.test', '1000', '06/24/2025', 'San Diego', '', 'False', ''];
const times = (count, row) => Array.from({ length: count }, () => row);
// San Diego: 113 positions. Police has two at exactly 100,000 overtime, one at 99,999 and one with
// no overtime figure; the homelessness department has too few positions for a share. Fire-Rescue
// is not a budget department here. Oceanside has exactly 100 positions, Smallville 99.
const pay = toCsv(payHeader, [
  ...times(56, position('San Diego', 'Police', 100_000, 20_000)), ...times(2, position('San Diego', 'Police', 100_000, 100_000)),
  position('San Diego', 'Police', 100_000, 99_999), position('San Diego', 'Police', 100_000, ''),
  ...times(30, position('San Diego', 'Fire-Rescue', 80_000, 50_000)), ...times(5, position('San Diego', homelessness, 90_000, 400)),
  ...offices.map((office) => position('San Diego', office, 70_000, 0)),
  ...times(60, position('Oceanside', 'Police', 100_000, 10_000)), ...times(40, position('Oceanside', 'Harbor', 60_000, 1_000)),
  ...times(99, position('Smallville', 'Police', 100_000, 90_000))]);

const table = (snapshotId, displayName, sourceUri) => ({ snapshotId, datasetKind: 'REGIONAL_DATA', displayName, tableName: `analysis_${snapshotId}`, importedRows: 1, rejectedRows: 0,
  sourceUri, sourceSha256: 'c'.repeat(64), retrievedAt: '2026-10-05T12:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} });
const columns = (dataset, header) => header.map((column) => ({ tableName: dataset.tableName, columnName: column.toLowerCase(), logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null }));
const budgetTable = { ...table('budget', 'Budget fixture', 'https://example.test/budget.csv'), datasetKind: 'OPERATING_BUDGET' };
const actualsTable = table('actuals', 'Actuals fixture', 'https://example.test/actuals.csv');
const payTable = table('pay', 'Pay fixture', null);
const schema = { datasets: [actualsTable, budgetTable, payTable], templates: [], columns: [...columns(actualsTable, actualsHeader), ...columns(budgetTable, budgetHeader), ...columns(payTable, payHeader)] };
export const fixture = { budget, actuals, pay, schema, queries: [...spendingQueries(schema, actualsTable), ...spendingQueries(schema, payTable)] };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/spending-leads.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
