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
const { serviceAccountabilityQueries } = compile('serviceAccountability');
const header = ['service_request_id', 'date_requested', 'case_age_days', 'service_name', 'date_closed', 'status', 'street_address', 'council_district', 'comm_plan_name'];
const rows = [];
const add = (count, fields) => { for (let i = 0; i < count; i += 1) rows.push([rows.length + 1, ...fields(i)]); };
// Potholes close in 5 days in two districts and 40 in the third, whose community name is
// published in two letter cases.
add(30, () => ['2025-01-06 08:00:00.000', 5, 'Pothole', '2025-01-11', 'Closed', '', 1, 'ALPHA']);
add(30, () => ['2025-01-06 08:00:00.000', 5, 'Pothole', '2025-01-11', 'Closed', '', 2, 'BETA']);
add(30, (i) => ['2025-01-06 08:00:00.000', 40, 'Pothole', '2025-02-15', 'Closed', '', 3, i % 2 ? 'GAMMA' : 'Gamma']);
add(12, (i) => [`2024-0${1 + (i % 9)}-10 09:30:00.000`, 400, 'Street Light Maintenance', '', 'In Process', '1 Main St', 1, 'ALPHA']);
// Referred reports carry no close date and the publisher's -999 age sentinel.
add(5, () => ['2025-03-01 10:00:00.000', -999, 'Other', '', 'Referred', '', 2, 'BETA']);
const csv = `${[header, ...rows].map((row) => row.join(',')).join('\n')}\n`;
const type = (name) => name === 'case_age_days' || name === 'council_district' ? 'INTEGER' : name === 'date_closed' ? 'DATE' : name === 'service_request_id' ? 'IDENTIFIER' : 'CATEGORY';
const schema = {
  datasets: [{ snapshotId: 'fixture', datasetKind: 'REGIONAL_DATA', displayName: 'Service fixture', tableName: 'analysis_fixture', importedRows: rows.length, rejectedRows: 0, sourceUri: 'https://example.test/service.csv', sourceSha256: 'b'.repeat(64), retrievedAt: '2026-10-04T00:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}' }],
  columns: header.map((columnName) => ({ tableName: 'analysis_fixture', columnName, logicalType: type(columnName), originalName: columnName, nullable: true, dictionary: null })),
  templates: [],
};
export const fixture = { csv, schema, queries: serviceAccountabilityQueries(schema, schema.datasets[0]) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/service-accountability.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
