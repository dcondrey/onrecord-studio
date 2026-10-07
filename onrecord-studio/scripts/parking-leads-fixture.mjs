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
const { parkingQueries } = compile('parkingLeads');
const header = ['citation_id', 'date_issue', 'date_creation', 'location', 'sector1', 'vio_code', 'vio_desc', 'vio_fine'];
// The publisher quotes every header and value. Code spacing, the SDMC prefix, the _W warning
// suffix and both spellings of the parking-enforcement unit are as written in the 2016, 2022 and
// 2025 files.
const OVERSIZED = 'OVERSIZED/NON-MOTORIZED/RV PROHIBITED';
const UNMOVED = '72 HOUR PARKING';
const rows = [
  ['70000001', '2022-07-01', '2022-07-01', '3500 SPORTS ARENA BLVD', 'CSD (S) POLICE', '86.0139(A)', OVERSIZED, '112.5'],
  ['70000002', '2022-07-02', '2022-07-02', '3500 SPORTS ARENA BLVD', 'CSD (S) POLICE', '86.0139 (A)', 'OVERSIZED/NON-MOTORIZED/RV', '112.5'],
  ['70000003', '2022-07-03', '2022-07-03', '3500 Sports Arena Blvd ', 'CSD (I) PARKING ENF', '86.0139(A) SDMC', 'OVERSIZED VEH', '112.5'],
  // Corrected later: the earlier entry's blank unit and misspelt place are not read.
  ['70000004', '2022-07-04', '2022-07-04', '3500 SPORTS ARENA BL', '', '86.0139(A)', OVERSIZED, '112.5'],
  ['70000004', '2022-07-04', '2022-08-15', '3500 SPORTS ARENA BLVD', 'CSD (S) POLICE', '86.0139(A)', OVERSIZED, '112.5'],
  ['70000005', '2022-07-05', '2022-07-05', '1900 FRONT ST', 'CSD I PARKING ENF', '86.0139(A)', OVERSIZED, 'n/a'],
  ['70000006', '2022-07-06', '2022-07-06', '', '', '86.0139(A)', OVERSIZED, '112.5'],
  ['70000007', '2022-07-07', '2022-07-07', '1900 FRONT ST', 'CSD (W) PARK RANGERS', '86.0139(A)', OVERSIZED, '112.5'],
  ['70000008', '2022-07-08', '2022-07-08', '1900 FRONT ST', 'CSD (S) POLICE', '86.0139(A)_W', `${OVERSIZED}/WARNING`, '0.0'],
  ['70000009', '2022-07-09', '2022-07-09', '1900 FRONT ST', 'CSD (S) POLICE', '86.0139(B)', 'NON-MOTORIZED VEHICLE', '112.5'],
  ['70000010', '2022-12-31', '2022-12-31', '4000 HARBOR DR', 'CSD (I) PARKING ENF', '86.0137(G) SDMC', UNMOVED, '53.5'],
  ['70000011', '2022-10-01', '2022-10-01', '4000 HARBOR DR', 'CSD (S) POLICE', 'SDMC 86.0137(G)', UNMOVED, '53.5'],
  ['70000012', '2022-10-02', '2022-10-02', ' 4000 harbor dr', 'CSD (I) PARKING ENF', '86.0137(G)S DMC', UNMOVED, '53.5'],
  // Entered twice at the same time by different units: conflicting, under no unit or total.
  ['70000013', '2022-10-03', '2022-10-03', '600 W BROADWAY', 'CSD (S) POLICE', '86.0137(G) SDMC', UNMOVED, '53.5'],
  ['70000013', '2022-10-03', '2022-10-03', '600 W BROADWAY', 'CSD (I) PARKING ENF', '86.0137(G) SDMC', UNMOVED, '53.5'],
  ['70000014', '2022-10-04', '2022-10-04', '600 W BROADWAY', 'CSD (G) STREET SWEEPING', '86.0112(E)S DMC', 'VIOLATION OF SIGNS-STREET SWEEPING', '52.5'],
  ['70000015', '2022-10-05', '2022-10-05', '600 W BROADWAY', 'CSD (S) POLICE', '86.0137(F) SDMC', 'VEHICLE HABITATION', '52.5'],
  // Entered twice at the same time, differing only in spelling: one parking-enforcement citation.
  ['70000016', '2022-10-06', '2022-10-06', '7700 SIEMPRE VIVA RD', 'CSD (I) PARKING ENF', '86.0137(G) SDMC', UNMOVED, '53.5'],
  ['70000016', '2022-10-06', '2022-10-06', '7700 SIEMPRE VIVA RD', 'CSD I PARKING ENF', 'SDMC 86.0137(G)', UNMOVED, '53.50'],
  // The earlier numbers of 86.0137(G) and 86.0137(F), as written in the 2012 and 2016 files.
  ['70000017', '2022-10-07', '2022-10-07', '7700 SIEMPRE VIVA RD', 'CSD (S) POLICE', '86.23(G)SDMC', 'VEHICLE NOT MOVED 1/10 MILE IN 72 HOURS', '53.5'],
  ['70000018', '2022-10-08', '2022-10-08', '7700 SIEMPRE VIVA RD', 'CSD (S) POLICE', '86.23(G)_W', 'VEHICLE NOT MOVED 1/10 MILE IN 72 HOURS/WARNING', '0.0'],
  ['70000019', '2022-10-09', '2022-10-09', '600 W BROADWAY', 'CSD (S) POLICE', '86.23(F)SDMC', 'LIVING IN VEHICLE PARKED ON STREET', '52.5'],
];
const quote = (value) => `"${value}"`;
const csv = `${[header, ...rows].map((row) => row.map(quote).join(',')).join('\n')}\n`;
const dataset = { snapshotId: 'parking_2022_2', datasetKind: 'REGIONAL_DATA', displayName: 'Parking citations 2022, July to December', tableName: 'analysis_parking_2022_2', importedRows: rows.length, rejectedRows: 0,
  sourceUri: 'https://seshat.datasd.org/parking_citations/parking_citations_2022_part2_datasd.csv', sourceSha256: 'e'.repeat(64), retrievedAt: '2026-10-07T00:00:00Z',
  observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} };
const schema = { datasets: [dataset], templates: [],
  columns: header.map((column) => ({ tableName: dataset.tableName, columnName: column, logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null })) };
export const fixture = { csv: { parking_2022_2: csv }, schema, queries: parkingQueries(schema, dataset) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/parking-leads.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
