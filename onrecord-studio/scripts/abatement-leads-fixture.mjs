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
const { abatementQueries } = compile('abatementLeads');
const quote = (value) => `"${String(value).replace(/"/g, '""')}"`;
const toCsv = (header, rows) => `${[header, ...rows].map((row) => row.map(quote).join(',')).join('\n')}\n`;
// Headers and the time format are the ones in the logs San Diego released to requests 25-5806
// and 26-772. Places A to D have one notice and one abatement: 2 hours, 30 hours, 96 hours, and
// one closed an hour before posting. E has two notices; F has no coordinates; G and H have no
// notice, and only G falls inside the notice log's period.
const place = (letter) => [`${letter} Street`, `32.7${letter.charCodeAt(0)}`, `-117.1${letter.charCodeAt(0)}`];
const posted = [['Case Number', 'Date Posted', 'Customer Provided Address', 'Geolocation (Latitude)', 'Geolocation (Longitude)'],
  [['1', '7/1/2025, 8:00 AM', ...place('A')], ['2', '7/1/2025, 8:00 AM', ...place('B')], ['3', '7/1/2025, 8:00 AM', ...place('C')], ['4', '7/3/2025, 9:00 AM', ...place('D')],
    ['5', '7/2/2025, 9:00 AM', ...place('E')], ['6', '7/10/2025, 1:30 PM', ...place('E')], ['7', '7/4/2025, 9:00 AM', 'F Street', '', '']]];
const completed = [['Customer Provided Address', 'Geolocation (Latitude)', 'Geolocation (Longitude)', 'Date/Time Closed'],
  [[...place('A'), '7/1/2025, 10:00 AM'], [...place('B'), '7/2/2025, 2:00 PM'], [...place('C'), '7/5/2025, 8:00 AM'], [...place('D'), '7/3/2025, 8:00 AM'],
    [...place('E'), '7/9/2025, 8:00 AM'], [...place('G'), '7/6/2025, 12:15 PM'], [...place('H'), '7/31/2025, 12:15 PM']]];
const alias = (header) => header.toLowerCase().replace(/[^a-z0-9]/g, '_').replace(/^_+|_+$/g, '');
const dataset = (snapshotId, rows) => ({ snapshotId, datasetKind: 'REGIONAL_DATA', displayName: `Abatement fixture, ${snapshotId}`, tableName: `analysis_abatement_${snapshotId}`, importedRows: rows.length, rejectedRows: 0,
  sourceUri: `https://example.test/${snapshotId}.csv`, sourceSha256: 'd'.repeat(64), retrievedAt: '2026-10-04T00:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}' });
const tables = { posted, completed };
const schema = {
  datasets: Object.entries(tables).map(([name, [, rows]]) => dataset(name, rows)), templates: [],
  columns: Object.entries(tables).flatMap(([name, [header]]) => header.map((column) => ({ tableName: `analysis_abatement_${name}`, columnName: alias(column), logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null }))),
};
export const fixture = { csv: { posted: toCsv(...posted), completed: toCsv(...completed) }, schema, queries: abatementQueries(schema, schema.datasets[0]) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/abatement-leads.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
