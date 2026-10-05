// Shared fixture builder: frontend verifies it stays current; Rust executes the generated
// queries through the real restricted-query boundary after a real CSV import.
import fs from 'node:fs';
import ts from 'typescript';

const source = fs.readFileSync(new URL('../src/utils/dataDiscovery.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
const module = { exports: {} };
new Function('module', 'exports', compiled)(module, module.exports);
const { buildDiscoveryPlan } = module.exports;
const rows = [
  ...Array.from({ length: 9 }, (_, i) => `A,22,${i + 1},adopted`), 'A,22,100,adopted',
  ...Array.from({ length: 10 }, () => 'A,23,10,adopted'), 'A,25,200,adopted',
  'B,22,0,adopted', 'B,23,5,adopted', 'C,22,,adopted', 'C,23,10,adopted',
  'D,unknown,7,adopted', 'A,23,999,proposed', ',23,3,adopted',
];
export const csv = `group,year,amount,phase\n${rows.join('\n')}\n`;
export const schema = {
  datasets: [{ snapshotId: 'fixture', datasetKind: 'REGIONAL_DATA', displayName: 'Discovery fixture', tableName: 'analysis_fixture',
    importedRows: rows.length, rejectedRows: 0, sourceUri: 'https://example.test/fixture.csv', sourceSha256: 'a'.repeat(64),
    retrievedAt: '2026-09-14T00:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} }],
  columns: ['group', 'year', 'amount', 'phase'].map(columnName => ({ tableName: 'analysis_fixture', columnName, logicalType: columnName === 'amount' ? 'INTEGER' : 'TEXT', originalName: columnName, nullable: true, dictionary: null })), templates: [],
};
export const selection = { snapshotId: 'fixture', measure: 'amount', aggregation: 'sum', dimension: 'group', period: 'year', periodMode: 'year_code', filters: [{ column: 'phase', operator: 'equals', value: 'adopted' }] };
const calendarSelection = { ...selection, periodMode: 'month' };
export const fixture = { csv, schema, selection, queries: buildDiscoveryPlan(schema, selection).queries,
  calendar: { csv: csv.replaceAll(',22,', ',2026-01-03,').replaceAll(',23,', ',2026-02-03,').replaceAll(',25,', ',2026-04-03,'),
    selection: calendarSelection, queries: buildDiscoveryPlan(schema, calendarSelection).queries },
};
if (process.argv.includes('--write')) {
  fs.mkdirSync(new URL('../src-tauri/tests/fixtures/', import.meta.url), { recursive: true });
  fs.writeFileSync(new URL('../src-tauri/tests/fixtures/discovery.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
}
