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
const { stopQueries } = compile('stopLeads');
const toCsv = (header, rows) => `${[header, ...rows].map((row) => row.join(',')).join('\n')}\n`;
// Headers keep the publisher's upper case. Three persons perceived as unhoused, three not (two in
// one stop), and one with the older "0" entry. One stop date is the publisher's year-23 typo.
const tables = {
  stops: [['UID', 'ID', 'PID', 'STOPDATE', 'PERCEIVEDUNHOUSED'], [
    ['1c', 1, 1, '2025-02-01', 'True'], ['2c', 2, 1, '2025-03-01', 'True'], ['3c', 3, 1, '0023-11-27', 'True'],
    ['4c', 4, 1, '2025-01-15', 'False'], ['4c', 4, 2, '2025-01-15', 'False'], ['5c', 5, 1, '2025-04-01', '0'], ['6c', 6, 1, '2025-05-01', 'False']]],
  // Person 2:1 has two results; person 6:1 has none.
  results: [['UID', 'ID', 'PID', 'RESULT'], [
    ['1c', 1, 1, 'Custodial Arrest pursuant to outstanding warrant'], ['2c', 2, 1, 'Custodial Arrest without warrant'], ['2c', 2, 1, 'Citation for infraction'],
    ['3c', 3, 1, 'Field interview card completed'], ['4c', 4, 1, 'Citation for infraction'], ['4c', 4, 2, 'In-field cite and release']]],
  reasons: [['UID', 'ID', 'PID', 'REASONFORSTOPCODETEXT'], [
    ['1c', 1, 1, '647(E) PC - DIS CON:LODGE W/O CONSENT (M) 32111'], ['2c', 2, 1, '647(E) PC - DIS CON:LODGE W/O CONSENT (M) 32111'],
    ['4c', 4, 1, '647(E) PC - DIS CON:LODGE W/O CONSENT (M) 32111'], ['3c', 3, 1, '65002 ZZ - LOCAL ORDINANCE VIOL (I) 65002']]],
  property: [['UID', 'ID', 'PID', 'BASISFORPROPERTYSEIZURE'], [
    ['1c', 1, 1, 'Safekeeping as allowed by law/statute'], ['1c', 1, 1, 'Evidence'], ['4c', 4, 1, 'Contraband']]],
};
const dataset = (name) => ({ snapshotId: name, datasetKind: name === 'stops' ? 'RIPA_STOPS' : 'REGIONAL_DATA', displayName: `Stop fixture, ${name}`, tableName: `analysis_stop_${name}`, importedRows: tables[name][1].length, rejectedRows: 0,
  sourceUri: `https://example.test/${name}.csv`, sourceSha256: 'c'.repeat(64), retrievedAt: '2026-10-04T00:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}' });
const schema = {
  datasets: Object.keys(tables).map(dataset), templates: [],
  columns: Object.entries(tables).flatMap(([name, [header]]) => header.map((column) => ({ tableName: `analysis_stop_${name}`, columnName: column.toLowerCase(), logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null }))),
};
export const fixture = { csv: Object.fromEntries(Object.entries(tables).map(([name, [header, rows]]) => [name, toCsv(header, rows)])), schema, queries: stopQueries(schema, schema.datasets[0]) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/stop-leads.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
