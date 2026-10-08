import fs from 'node:fs';
import { compileUtil as compile } from './compile-util.mjs';
const { capacityQueries, OPEN_REPORTS, closedReports } = compile('capacityLeads');
const quote = (value) => `"${String(value).replace(/"/g, '""')}"`;
const toCsv = (header, rows) => `${[header, ...rows].map((row) => row.map(quote).join(',')).join('\n')}\n`;

// Headers are the publishers' own: the UCSD Homelessness Hub layer HIC_ES_2014_2024 as GeoJSON,
// budget_operating_datasd.csv, and the Get It Done open and closed-year files.
const inventoryHeader = ['HMIS_Org_ID', 'HMIS_Proj_ID', 'Year', 'Organization_Name', 'Project_Name', 'City', 'Final_Address', 'X', 'Y', 'Inclement', 'Bed_Type', 'Victim_Service_Provider',
  'Year_Round_Beds', 'Total_Seasonal_Beds', 'Overflow_Beds', 'PIT_Count', 'Total_Beds', 'Utilization_Rate', 'Project_Name_Cleaned', 'Housing_Type', 'ObjectId'];
// Year, City, Year_Round_Beds, Total_Beds. 2017 is before the years read; Carlsbad is another
// city; one 2025 project records no year-round count, which is not a count of zero.
const projects = [[2017, 'San Diego', 5, 9], [2023, 'San Diego', 80, 85], [2024, 'San Diego', 100, 110], [2024, 'San Diego', 50, 60], [2025, 'San Diego', 120, 130],
  [2025, 'San Diego', null, 40], [2025, 'Carlsbad', 50, 50], [2026, 'San Diego', 90, 95]];
const inventory = JSON.stringify({ type: 'FeatureCollection', features: projects.map(([year, city, yearRound, total], index) => ({ type: 'Feature', id: index + 1, geometry: null,
  properties: { HMIS_Org_ID: 15, HMIS_Proj_ID: String(200 + index), Year: year, Organization_Name: 'Provider', Project_Name: `Shelter ${index + 1}`, City: city, Final_Address: '1 Main St', X: -117.1, Y: 32.7,
    Inclement: 'No', Bed_Type: 'Facility-based beds', Victim_Service_Provider: 'No', Year_Round_Beds: yearRound, Total_Seasonal_Beds: 0, Overflow_Beds: 0, PIT_Count: 1, Total_Beds: total,
    Utilization_Rate: null, Project_Name_Cleaned: `Shelter ${index + 1}`, Housing_Type: 'Site-based single site', ObjectId: index + 1 } })) });

const budgetHeader = ['amount', 'report_fy', 'budget_cycle', 'fund_type', 'fund_number', 'dept_name', 'funds_center_number', 'account', 'account_number'];
const line = (amount, fy, cycle, department, account) => [amount, fy, cycle, 'General Fund', '100000', department, '1001000011', 'Salaried Wages', account];
const department = 'Homelessness Strategies & Solutions';
// FY 23 has adopted rows and no such department; FY 26 rows carry no budget cycle, as published.
// A revenue account (4...) and a proposed line are not adopted expense.
const budget = toCsv(budgetHeader, [line(900, '23', 'adopted', 'Police', '500011'), line(1000, '24', 'adopted', department, '500011'), line(500, '24', 'adopted', department, '512000'),
  line(9999, '24', 'adopted', department, '420000'), line(7777, '24', 'proposed', department, '500011'), line(800, '24', 'adopted', 'Police', '500011'),
  line(2000, '25', 'adopted', department, '500011'), line(3000, '26', '', department, '500011')]);

const reportHeader = ['service_request_id', 'service_request_parent_id', 'sap_notification_number', 'date_requested', 'case_age_days', 'case_record_type', 'service_name', 'service_name_detail',
  'date_closed', 'status', 'lat', 'lng', 'street_address', 'zipcode', 'council_district', 'comm_plan_code', 'comm_plan_name', 'park_name', 'case_origin', 'referred', 'iamfloc', 'floc', 'public_description'];
const report = (id, filed, service, closed) => [id, '', '', filed, '3', 'ESD Complaint/Report', service, '', closed, closed ? 'Closed' : 'In Process', '32.7', '-117.1', '1 Main St', '92101', '3', '56', 'Downtown', '', 'Mobile', '', '', '', ''];
// Fiscal 2025 runs 2024-07-01 to 2025-06-30. Its four reports sit in the open file and in the
// 2024, 2025 and 2026 closed files; report 9001 is in both the open and the 2025 file.
const reports = {
  open: toCsv(reportHeader, [report('9001', '2024-11-02 08:00:00.000', 'Encampment', ''), report('9002', '2024-11-03 08:00:00.000', 'Pothole', ''), report('9003', '2025-08-01 08:00:00.000', 'Encampment', '')]),
  2026: toCsv(reportHeader, [report('5001', '2025-02-01 09:00:00.000', 'Encampment', '2026-01-05')]),
  2025: toCsv(reportHeader, [report('8001', '2024-07-01 00:05:00.000', 'Encampment', '2025-01-05'), report('8002', '2024-06-30 23:50:00.000', 'Encampment', '2025-01-06'),
    report('9001', '2024-11-02 08:00:00.000', 'Encampment', '2025-03-01'), report('8003', '2024-08-01 10:00:00.000', 'Homeless Outreach', '2025-02-01')]),
  2024: toCsv(reportHeader, [report('7001', '2023-09-01 09:00:00.000', 'Encampment', '2024-02-01'), report('7002', '2024-07-10 09:00:00.000', 'Encampment', '2024-08-01')]),
  2023: toCsv(reportHeader, [report('6001', '2023-08-15 09:00:00.000', 'Encampment', '2023-12-01'), report('6002', '2023-03-01 09:00:00.000', 'Encampment', '2023-04-01')]),
};

const table = (snapshotId, displayName, sourceUri, datasetKind = 'REGIONAL_DATA') => ({ snapshotId, datasetKind, displayName, tableName: `analysis_${snapshotId}`, importedRows: 1, rejectedRows: 0,
  sourceUri, sourceSha256: 'c'.repeat(64), retrievedAt: '2026-10-04T12:00:00Z', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} });
const columns = (dataset, header) => header.map((column) => ({ tableName: dataset.tableName, columnName: column.toLowerCase(), logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null }));
const inventoryTable = table('shelter', 'Shelter inventory fixture', 'https://example.test/hic.geojson');
const budgetTable = table('budget', 'Budget fixture', 'https://example.test/budget.csv', 'OPERATING_BUDGET');
const reportTables = Object.fromEntries(Object.keys(reports).map((name) => [name, name === 'open' ? table('open', 'Open reports fixture', OPEN_REPORTS)
  : table(`closed_${name}`, `Closed ${name} fixture`, closedReports(Number(name)))]));
const schema = { datasets: [inventoryTable, budgetTable, ...Object.values(reportTables)], templates: [],
  columns: [...columns(inventoryTable, inventoryHeader), ...columns(budgetTable, budgetHeader), ...Object.values(reportTables).flatMap((dataset) => columns(dataset, reportHeader))] };
export const fixture = { inventory, budget, reports, schema, queries: capacityQueries(schema, inventoryTable) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/capacity-leads.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
