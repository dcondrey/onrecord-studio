import fs from 'node:fs';
import { compileUtil as compile } from './compile-util.mjs';
const { chargeQueries } = compile('chargeLeads');
const quote = (value) => `"${String(value).replace(/"/g, '""')}"`;
const toCsv = (header, rows) => `${[header, ...rows].map((row) => row.map(quote).join(',')).join('\n')}\n`;
// Headers and values are the ones in the logs SDPD released to requests 25-7080 and 26-6187: one
// row per charge, "NULL" for an empty entry, 54.0110 written both with and without its last zero.
// Five of the eight arrests and citations cite a counted section; arrest 1 cites nothing else,
// arrest 2 also cites battery, and 647(F) is not 647(E).
const header = ['activityNumber', 'ActivityType', 'activityDate', 'violationSection', 'race'];
const rows = [
  ['2500000101', 'ARREST', '2025-01-01 00:02:00', '63.0404(C)', 'BLACK'],
  ['2500000201', 'ARREST', '2025-01-02 10:00:00', '54.011', 'WHITE'], ['2500000201', 'ARREST', '2025-01-02 10:00:00', '242', 'WHITE'],
  ['2500000301', 'ARREST', '2025-01-03 23:10:00', '647(F)DK', 'HISPANIC'],
  ['2780001', 'CITATION', '2025-01-04 08:10:00', '56.55', 'WHITE'], ['2780002', 'CITATION', '2025-01-05 09:00:00', '647(E)', 'BLACK'],
  ['2780003', 'CITATION', '2025-01-06 01:57:00', '16025A', 'HISPANIC'], ['2780004', 'CITATION', '2025-02-01 07:30:00', '54.0110', 'NULL'],
  ['2780005', 'CITATION', '2025-02-02 12:00:00', '63.0102B7', 'WHITE'],
];
const dataset = { snapshotId: 'charges', datasetKind: 'REGIONAL_DATA', displayName: 'Charge fixture', tableName: 'analysis_charge_log', importedRows: rows.length, rejectedRows: 0,
  sourceUri: 'https://example.test/charges.csv', sourceSha256: 'e'.repeat(64), retrievedAt: '2026-10-04T00:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} };
const schema = { datasets: [dataset], templates: [],
  columns: header.map((column) => ({ tableName: dataset.tableName, columnName: column.toLowerCase(), logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null })) };
export const fixture = { csv: toCsv(header, rows), schema, queries: chargeQueries(schema, dataset) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/charge-leads.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
