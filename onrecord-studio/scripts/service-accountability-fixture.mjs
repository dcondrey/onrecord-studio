import fs from 'node:fs';
import { compileUtil as compile } from './compile-util.mjs';
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
// The publisher's staff group per service; one street light in each copy sits under another group.
const staffGroup = (row) => row[0] === 100 ? 'Traffic Engineering' : { Pothole: 'TSW', 'Street Light Maintenance': 'TSW', Graffiti: 'Parking' }[row[3]] ?? '';
const toCsv = (records) => `${[[...header, 'case_record_type'], ...records.map((row) => [...row, staffGroup(row)])].map((row) => row.join(',')).join('\n')}\n`;
// Transportation's adopted expense is 150 in FY 25 and 80 in FY 24. A revenue account, a proposed
// cycle and a year with no cycle are in the file and are not adopted expense.
const budgetHeader = ['amount', 'report_fy', 'budget_cycle', 'fund_number', 'dept_name', 'funds_center_number', 'account', 'account_number'];
const budgetCsv = `${[budgetHeader,
  [100, 25, 'adopted', 100000, 'Transportation', 2116000011, 'Salaried Wages', 500011], [50, 25, 'adopted', 100000, 'Transportation', 2116000011, 'Overtime Budgeted', 500121],
  [999, 25, 'adopted', 100000, 'Transportation', 2116000011, 'Parking Citations', 420011], [70, 25, 'proposed', 100000, 'Transportation', 2116000011, 'Salaried Wages', 500011],
  [500, 26, '', 100000, 'Transportation', 2116000011, 'Salaried Wages', 500011], [80, 24, 'adopted', 100000, 'Transportation', 2116000011, 'Salaried Wages', 500011],
  [30, 25, 'adopted', 100000, 'Stormwater', 2114000011, 'Salaried Wages', 500011], [7, 25, 'adopted', 100000, 'Library', 1714000011, 'Salaried Wages', 500011],
].map((row) => row.join(',')).join('\n')}\n`;
const csv = toCsv(rows);
// An earlier copy of the same source, all open. Ten street lights are still open in the later
// copy (two more arrived); six reports left. Report 1 is closed in the later copy itself, 201-203
// and 204 are closed and referred in the closed-reports table, and 205 is in neither.
const open = (id, age, service) => [id, '2025-01-06 08:00:00.000', age, service, '', 'In Process', '', 1, 'ALPHA'];
const earlierRows = [...Array.from({ length: 10 }, (_, i) => open(91 + i, 380, 'Street Light Maintenance')),
  open(1, 5, 'Pothole'), open(201, 400, 'Pothole'), open(202, 400, 'Pothole'), open(203, 400, 'Pothole'), open(204, 10, 'Pothole'), open(205, 500, 'Graffiti')];
const closedRows = [...[201, 202, 203].map((id) => [id, '2025-01-06 08:00:00.000', 410, 'Pothole', '2026-09-20', 'Closed', '', 1, 'ALPHA']),
  [204, '2026-09-04 08:00:00.000', 12, 'Pothole', '', 'Referred', '', 1, 'ALPHA'], [900, '2026-01-06 08:00:00.000', 3, 'Pothole', '2026-01-09', 'Closed', '', 2, 'BETA']];
const type = (name) => name === 'case_age_days' || name === 'council_district' ? 'INTEGER' : name === 'date_closed' ? 'DATE' : name === 'service_request_id' ? 'IDENTIFIER' : 'CATEGORY';
const tables = [['analysis_fixture', 'fixture', 'https://example.test/service.csv', '2026-10-04T00:00:00Z'], ['analysis_earlier_fixture', 'earlier', 'https://example.test/service.csv', '2026-09-14T00:00:00Z'],
  ['analysis_closed_fixture', 'closed', 'https://example.test/closed.csv', '2026-10-04T00:00:00Z']];
const schema = {
  datasets: [{ snapshotId: 'fixture', datasetKind: 'REGIONAL_DATA', displayName: 'Service fixture', tableName: 'analysis_fixture', importedRows: rows.length, rejectedRows: 0, sourceUri: 'https://example.test/service.csv', sourceSha256: 'b'.repeat(64), retrievedAt: '2026-10-04T00:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} }],
  columns: tables.flatMap(([tableName]) => [...header, 'case_record_type'].map((columnName) => ({ tableName, columnName, logicalType: type(columnName), originalName: columnName, nullable: true, dictionary: null }))),
  templates: [],
};
schema.datasets.push(...tables.slice(1).map(([tableName, snapshotId, sourceUri, retrievedAt]) => ({ ...schema.datasets[0], tableName, snapshotId, sourceUri, retrievedAt, displayName: `Service fixture, ${snapshotId}` })));
schema.datasets.push({ ...schema.datasets[0], tableName: 'analysis_budget_fixture', snapshotId: 'budget', datasetKind: 'OPERATING_BUDGET', sourceUri: 'https://example.test/budget.csv', displayName: 'Budget fixture', partialDataset: false });
schema.columns.push(...budgetHeader.map((columnName) => ({ tableName: 'analysis_budget_fixture', columnName, logicalType: columnName === 'amount' ? 'NUMBER' : 'CATEGORY', originalName: columnName, nullable: true, dictionary: null })));
export const fixture = { csv, earlierCsv: toCsv(earlierRows), closedCsv: toCsv(closedRows), budgetCsv, schema, queries: serviceAccountabilityQueries(schema, schema.datasets[0]) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/service-accountability.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
