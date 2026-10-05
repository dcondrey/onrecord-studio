import fs from 'node:fs';
import ts from 'typescript';
const modules = new Map();
function compile(name) {
  if (modules.has(name)) return modules.get(name);
  const source = fs.readFileSync(new URL(`../src/utils/${name}.ts`, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
  const module = { exports: {} };
  new Function('module', 'exports', 'require', compiled)(module, module.exports, (path) => compile(path.replace('./', '')));
  modules.set(name, module.exports);
  return module.exports;
}
const { belongingsQueries } = compile('belongingsLeads');
const quote = (value) => `"${String(value).replace(/"/g, '""')}"`;
const toCsv = (header, rows) => `${[header, ...rows].map((row) => row.map(quote).join(',')).join('\n')}\n`;
// The column names are assumed, not taken from a released log: no stored-property log has been
// imported yet. Three entries discarded, two stored (one released), one with no disposition.
const entries = [['7/1/2025', 'Tent', 'Discarded', ''], ['7/1/2025', 'Bicycle', 'Stored', '7/9/2025'], ['7/3/2025', 'Backpack', 'Stored', ''],
  ['7/3/2025', 'Bedding', 'Discarded', ''], ['12/2/2024', 'Cart', 'Discarded', ''], ['7/8/2025', 'Documents', '', '']];
const tables = {
  released: [['Date Stored', 'Item Description', 'Disposition', 'Date Released'], entries],
  bare: [['Date Stored', 'Item Description', 'Disposition'], entries.map((row) => row.slice(0, 3))],
};
const alias = (header) => header.toLowerCase().replace(/[^a-z0-9]/g, '_').replace(/^_+|_+$/g, '');
const dataset = (snapshotId, rows) => ({ snapshotId, datasetKind: 'REGIONAL_DATA', displayName: `Stored-property fixture, ${snapshotId}`, tableName: `analysis_belongings_${snapshotId}`, importedRows: rows.length, rejectedRows: 0,
  sourceUri: `https://example.test/${snapshotId}.csv`, sourceSha256: 'e'.repeat(64), retrievedAt: '2026-10-05T00:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} });
const schema = {
  datasets: Object.entries(tables).map(([name, [, rows]]) => dataset(name, rows)), templates: [],
  columns: Object.entries(tables).flatMap(([name, [header]]) => header.map((column) => ({ tableName: `analysis_belongings_${name}`, columnName: alias(column), logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null }))),
};
export const fixture = { csv: Object.fromEntries(Object.entries(tables).map(([name, table]) => [name, toCsv(...table)])), schema, queries: schema.datasets.flatMap((item) => belongingsQueries(schema, item)) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/belongings-leads.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
