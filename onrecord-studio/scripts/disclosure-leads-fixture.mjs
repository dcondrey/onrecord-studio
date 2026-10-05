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
const { disclosureQueries } = compile('disclosureLeads');
const quote = (value) => `"${String(value).replace(/"/g, '""')}"`;
const toCsv = (header, rows) => `${[header, ...rows].map((row) => row.map(quote).join(',')).join('\n')}\n`;
// Headers and values are the ones in the City of San Diego filing portal's agency-report export.
// Four Form 803 reports name official A, one names B, one names nobody; one Form 802 carries the
// portal's "not electronically filed" note in its type; one Form 801 has no readable date.
const header = ['Filing Date', 'Filing Type', 'Description', 'Agency', 'Official'];
const rows = [
  ['2026-01-05T08:00:00+00:00', 'FPPC 803', 'Behest Payments; Payee Information: A Club', 'City of San Diego', 'Official A'],
  ['2026-02-05T08:00:00+00:00', 'FPPC 803', 'Behest Payments; Payee Information: B Fund', 'City of San Diego', 'Official A'],
  ['2026-03-05T08:00:00+00:00', 'FPPC 803', 'Behest Payments; Payee Information: C Event', 'City of San Diego', 'Official A'],
  ['2026-08-28T07:00:00+00:00', 'FPPC 803', 'Behest Payments; Train Ticket', 'City of San Diego', 'Official A'],
  ['2026-07-01T07:00:00+00:00', 'FPPC 803', 'Behest Payments; Payee Information: D Club', 'City of San Diego', 'Official B'],
  ['2026-06-01T07:00:00+00:00', 'FPPC 803', 'Behest Payments; Payee Information: E Club', 'City of San Diego', ''],
  ['2026-04-01T07:00:00+00:00', 'FPPC 802; Not electronically filed.', 'Ticket distribution', 'City of San Diego', ''],
  ['', 'FPPC 801', 'Payments to Agency; a sports team', 'City of San Diego', ''],
];
const alias = (name) => name.toLowerCase().replace(/[^a-z0-9]/g, '_').replace(/^_+|_+$/g, '');
const dataset = { snapshotId: 'reports', datasetKind: 'REGIONAL_DATA', displayName: 'Agency report fixture', tableName: 'analysis_agency_reports', importedRows: rows.length, rejectedRows: 0,
  sourceUri: null, sourceSha256: 'f'.repeat(64), retrievedAt: '2026-10-05T00:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} };
const schema = { datasets: [dataset], templates: [],
  columns: header.map((column) => ({ tableName: dataset.tableName, columnName: alias(column), logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null })) };
export const fixture = { csv: toCsv(header, rows), schema, queries: disclosureQueries(schema, dataset) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/disclosure-leads.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
