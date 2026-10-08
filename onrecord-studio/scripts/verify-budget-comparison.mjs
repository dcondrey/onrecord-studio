// Opt-in numerical regression for the actual UI query builder. Requires the DuckDB CLI.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { execFileSync } from 'node:child_process';
import ts from 'typescript';

const source = fs.readFileSync(new URL('../src/utils/budgetComparison.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
const module = { exports: {} };
new Function('module', 'exports', compiled)(module, module.exports);
const { buildBudgetComparison } = module.exports;
const common = ['amount', 'report_fy', 'fund_number', 'funds_center_number', 'account_number', 'dept_name'];
const schema = {
  datasets: [{ snapshotId: 'budget', tableName: 'analysis_budget_fixture' }, { snapshotId: 'actuals', tableName: 'analysis_actuals_fixture' }],
  columns: [
    ...[...common, 'budget_cycle'].map(columnName => ({ tableName: 'analysis_budget_fixture', columnName, logicalType: columnName === 'amount' ? 'NUMBER' : 'CATEGORY' })),
    ...common.map(columnName => ({ tableName: 'analysis_actuals_fixture', columnName, logicalType: columnName === 'amount' ? 'NUMBER' : 'CATEGORY' })),
  ],
};
const selection = { budgetSnapshotId: 'budget', actualsSnapshotId: 'actuals', fiscalYear: '23', cycle: 'adopted', accountClass: 'expense' };
const definitions = 'amount DECIMAL(38,10), report_fy VARCHAR, fund_number VARCHAR, funds_center_number VARCHAR, account_number VARCHAR, dept_name VARCHAR';
const setup = `CREATE TABLE analysis_budget_fixture (${definitions}, budget_cycle VARCHAR);
CREATE TABLE analysis_actuals_fixture (${definitions});
INSERT INTO analysis_budget_fixture VALUES
  (60.01,'23','000100','010','511000','Housing','adopted'),
  (39.99,'23','000100','010','511000','Housing','Adopted'),
  (999,'23','000100','010','511000','Housing','proposed'),
  (1,'22','000100','010','511000','Housing','adopted'),
  (200,'23','000100','020','511000','Housing','adopted'),
  (NULL,'23','000100','040','511000','Housing','adopted'),
  (0,'23','000100','050','511000','Housing','adopted'),
  (50,'23','000100','010','411000','Housing','adopted');
INSERT INTO analysis_actuals_fixture VALUES
  (20,'23','000100','010','511000','Housing'),
  (10,'23','000100','010','511000','Housing'),
  (7,'23','000100','030','511000','Housing'),
  (10,'23','000100','040','511000','Housing'),
  (5,'23','000100','050','511000','Housing'),
  (40,'22','000100','010','511000','Housing'),
  (45,'23','000100','010','411000','Housing');`;
function execute(query) {
  const output = execFileSync(process.env.DUCKDB_CLI || 'duckdb', ['-json', ':memory:'], { input: `${setup}\n${query};`, encoding: 'utf8' }).trim();
  return output ? JSON.parse(output) : [];
}
const rows = execute(buildBudgetComparison(schema, selection));
assert.equal(rows.length, 5);
const at = code => rows.find(row => row.funds_center_number === code);
assert.equal(at('010').fund_number, '000100');
// Matched lines lead, largest |difference| first, so a result cut at the row limit keeps the true top lines.
assert.deepEqual(rows.map(row => row.funds_center_number).slice(0, 2), ['010', '050']);
// CLI versions encode DECIMAL/BIGINT as numbers or strings; these small fixture integers are exact.
assert.equal(Number(at('010').budget_amount), 100);
assert.equal(Number(at('010').actuals_amount), 30);
assert.equal(Number(at('010').budget_rows), 2);
assert.equal(Number(at('010').actuals_rows), 2);
assert.equal(Number(at('010').actual_minus_budget), -70);
assert.equal(Number(at('010').variance_percent), -70);
assert.equal(at('020').comparison_status, 'Missing actuals');
assert.equal(at('020').actual_minus_budget, null);
assert.equal(at('030').comparison_status, 'Missing budget baseline');
assert.equal(at('040').comparison_status, 'Incomplete amounts');
assert.equal(at('040').actual_minus_budget, null);
assert.equal(at('050').variance_percent, null);
const revenue = execute(buildBudgetComparison(schema, { ...selection, accountClass: 'revenue' }));
assert.equal(revenue.length, 1);
assert.equal(Number(revenue[0].actual_minus_budget), -5);
assert.deepEqual(execute(buildBudgetComparison(schema, { ...selection, fundNumber: "000100' OR 1=1 --" })), []);
assert.throws(() => buildBudgetComparison(schema, { ...selection, fiscalYear: "23' OR 1=1" }));
console.log('Budget comparison: correct aggregation, unmatched records, missing amounts, zero denominators, account classes, fiscal scope, and literal escaping.');
