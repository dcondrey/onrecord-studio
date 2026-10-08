import fs from 'node:fs';
import { compileUtil as compile } from './compile-util.mjs';
const { abatementQueries } = compile('abatementLeads');
const quote = (value) => `"${String(value).replace(/"/g, '""')}"`;
const toCsv = (header, rows) => `${[header, ...rows].map((row) => row.map(quote).join(',')).join('\n')}\n`;
// Two abatements with complete reports around them (P on 2025-03-15, Q on 2025-06-10), one too
// close to the end of the reports (R, 2025-09-25) and one without coordinates. Reports: at P, two
// before, one on the day, two after within 150 m, and one 500 m north after; at Q, one 400 m east
// before; one report 2 km away; one report 15 days after P (outside the window); one report held
// by both tables (counted once); one Pothole report at P.
const completed = [['Customer Provided Address', 'Geolocation (Latitude)', 'Geolocation (Longitude)', 'Date/Time Closed'], [
  ['P Street', '32.7100', '-117.1500', '3/15/2025, 9:00 AM'], ['Q Street', '32.7300', '-117.1300', '6/10/2025, 9:00 AM'],
  ['R Street', '32.7500', '-117.1100', '9/25/2025, 9:00 AM'], ['S Street', '', '', '4/1/2025, 9:00 AM']]];
const reportHeader = ['service_request_id', 'date_requested', 'service_name', 'lat', 'lng'];
const closed2025 = [reportHeader, [
  ['1', '2025-03-05 08:00:00', 'Encampment', '32.7100', '-117.1500'], ['2', '2025-03-10 08:00:00', 'Encampment', '32.7105', '-117.1500'],
  ['3', '2025-03-15 12:00:00', 'Encampment', '32.7100', '-117.1500'], ['4', '2025-03-20 08:00:00', 'Encampment', '32.7100', '-117.1505'],
  ['5', '2025-03-22 08:00:00', 'Encampment', '32.7145', '-117.1500'], ['6', '2025-06-01 08:00:00', 'Encampment', '32.7300', '-117.1257'],
  ['7', '2025-03-16 08:00:00', 'Encampment', '32.7280', '-117.1500'], ['8', '2025-03-30 08:00:00', 'Encampment', '32.7100', '-117.1500'],
  ['9', '2025-03-12 08:00:00', 'Pothole', '32.7100', '-117.1500'], ['10', '2025-03-18 08:00:00', 'Encampment', '32.7100', '-117.1500']]];
// Ten more reports 300 m north of Q in the fortnight before and fifteen after: with the one to
// the east, eleven before and fifteen after, so Q's ring is compared and has not risen by half.
for (let index = 0; index < 25; index += 1) closed2025[1].push([String(100 + index), index < 10 ? '2025-06-05 08:00:00' : '2025-06-15 08:00:00', 'Encampment', '32.7327', '-117.1300']);
const open = [reportHeader, [['10', '2025-03-18 08:00:00', 'Encampment', '32.7100', '-117.1500'], ['11', '2025-09-20 08:00:00', 'Encampment', '', '']]];
const alias = (header) => header.toLowerCase().replace(/[^a-z0-9]/g, '_').replace(/^_+|_+$/g, '');
const tables = { completed, closed2025, open };
const uri = { completed: null, closed2025: 'https://seshat.datasd.org/get_it_done_reports/get_it_done_requests_closed_2025_datasd.csv', open: 'https://seshat.datasd.org/get_it_done_reports/get_it_done_requests_open_datasd.csv' };
const dataset = (snapshotId) => ({ snapshotId, datasetKind: 'REGIONAL_DATA', displayName: `Nearby fixture, ${snapshotId}`, tableName: `analysis_nearby_${snapshotId}`, importedRows: tables[snapshotId][1].length, rejectedRows: 0,
  sourceUri: uri[snapshotId], sourceSha256: 'a'.repeat(64), retrievedAt: '2025-10-01T12:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} });
const schema = {
  datasets: Object.keys(tables).map(dataset), templates: [],
  columns: Object.entries(tables).flatMap(([name, [header]]) => header.map((column) => ({ tableName: `analysis_nearby_${name}`, columnName: alias(column), logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null }))),
};
export const fixture = { csv: Object.fromEntries(Object.entries(tables).map(([name, [header, rows]]) => [name, toCsv(header, rows)])), schema, queries: abatementQueries(schema, schema.datasets[0]) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/abatement-reports-nearby.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
