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
const { housingQueries } = compile('housingLeads');
// The published header of approvals_issued_2025_datasd.csv; the created and closed files share it.
const header = ['DEVELOPMENT_ID', 'PROJECT_ID', 'PROJECT_TYPE', 'PROJECT_STATUS', 'PROJECT_PROCESSING_CODE', 'PROJECT_CREATE_DATE', 'PROJECT_DEEMEDCOMPLETE_DATE', 'PROJECT_TRUST_ACCOUNT_NO',
  'PROJECT_TITLE', 'PROJECT_SCOPE', 'JOB_ID', 'JOB_DRAWING_NUMBER', 'GIS_ADDRESS', 'GIS_APN', 'JOB_BC_CODE', 'JOB_BC_CODE_DESCRIPTION', 'GIS_LATITUDE', 'GIS_LONGITUDE', 'APPROVAL_ID',
  'APPROVAL_CATEGORY_CODE', 'APPROVAL_PROCESSING_CODE', 'APPROVAL_TYPE', 'APPROVAL_STATUS', 'APPROVAL_SCOPE', 'APPROVAL_CREATE_DATE', 'APPROVAL_ISSUE_DATE', 'APPROVAL_CLOSE_DATE',
  'APPROVAL_EXPIRE_DATE', 'APPROVAL_VALUATION', 'APPROVAL_DU_NET_CHANGE', 'APPROVAL_STORIES', 'APPROVAL_FLOOR_AREA', 'APPROVAL_DU_EXTREMELY_LOW', 'APPROVAL_DU_VERY_LOW', 'APPROVAL_DU_LOW',
  'APPROVAL_DU_MODERATE', 'APPROVAL_DU_ABOVE_MODERATE', 'APPROVAL_DU_FUTURE_DEMO', 'APPROVAL_DU_BONUS', 'APPROVAL_ADU_EXTREMELY_LOW', 'APPROVAL_ADU_VERY_LOW', 'APPROVAL_ADU_LOW',
  'APPROVAL_ADU_MODERATE', 'APPROVAL_ADU_ABOVE_MODERATE', 'APPROVAL_ADU_BONUS', 'APPROVAL_ADU_TOTAL', 'APPROVAL_JADU_EXTREMELY_LOW', 'APPROVAL_JADU_VERY_LOW', 'APPROVAL_JADU_LOW',
  'APPROVAL_JADU_MODERATE', 'APPROVAL_JADU_ABOVE_MODERATE', 'APPROVAL_JADU_BONUS', 'APPROVAL_JADU_TOTAL', 'APPROVAL_PERMIT_HOLDER'];
/** One approval: units are [extremely low, very low, low, moderate, above moderate, bonus, ADU total]; null leaves them blank. */
const approval = (project, job, id, type, issued, units) => {
  const row = Object.fromEntries(header.map((name) => [name, '']));
  const [eli, vli, low, moderate, above, bonus, adu] = units ?? [];
  Object.assign(row, { DEVELOPMENT_ID: `DEV-${project}`, PROJECT_ID: `PRJ-${project}`, JOB_ID: job, APPROVAL_ID: id, APPROVAL_TYPE: type, APPROVAL_STATUS: 'Issued',
    APPROVAL_CREATE_DATE: issued, APPROVAL_ISSUE_DATE: issued });
  if (units) Object.assign(row, { APPROVAL_DU_EXTREMELY_LOW: eli, APPROVAL_DU_VERY_LOW: vli, APPROVAL_DU_LOW: low, APPROVAL_DU_MODERATE: moderate, APPROVAL_DU_ABOVE_MODERATE: above,
    APPROVAL_DU_FUTURE_DEMO: 0, APPROVAL_DU_BONUS: bonus, APPROVAL_ADU_TOTAL: adu });
  return header.map((name) => row[name]);
};
const quote = (value) => (value === '' ? '' : `"${value}"`);
const csv = (rows) => `${[header, ...rows].map((row) => row.map(quote).join(',')).join('\n')}\n`;
const tables = {
  // Every building permit on a job counted in 2024 has its tiers blank. JOB-100005 has a permit
  // here and one in 2025, so it is counted in 2024.
  approvals_issued_2024: { uri: 'issued_2024', retrievedAt: '2026-10-01T00:00:00Z', rows: [
    approval('0900001', 'JOB-240001', 'PMT-2400001', 'Building Permit', '2024-03-04', null),
    approval('0900002', 'JOB-240002', 'PMT-2400002', 'Combination Building Permit', '2024-06-11', null),
    approval('0900005', 'JOB-100005', 'PMT-2400005', 'Building Permit', '2024-12-20', null)] },
  // An earlier copy of 2025, replaced by the later one.
  approvals_prior_2025: { uri: 'issued_2025', retrievedAt: '2026-09-01T00:00:00Z', rows: [
    approval('0900009', 'JOB-100009', 'PMT-3000009', 'Building Permit', '2025-02-01', [50, 50, 50, 50, 50, 50, 50])] },
  approvals_issued_2025: { uri: 'issued_2025', retrievedAt: '2026-10-07T00:00:00Z', rows: [
    // A discretionary job: its units recur on the Coastal and the Site Development approval, as
    // PRJ-0696585 / JOB-060768 does in the published file, and it has no building permit.
    approval('0696585', 'JOB-060768', 'PMT-3000001', 'Coastal Development', '2025-04-02', [0, 0, 0, 0, 224, 0, 0]),
    approval('0696585', 'JOB-060768', 'PMT-3000002', 'Site  Development Permit', '2025-04-02', [0, 0, 0, 0, 224, 0, 0]),
    // Two building permits on one job, each repeating the job's units.
    approval('0900003', 'JOB-100001', 'PMT-3000003', 'Building Permit', '2025-05-06', [2, 3, 4, 5, 10, 6, 1]),
    approval('0900003', 'JOB-100001', 'PMT-3000004', 'Building Permit', '2025-05-07', [2, 3, 4, 5, 10, 6, 1]),
    approval('0900004', 'JOB-100002', 'PMT-3000005', ' Combination Building Permit ', '2025-07-08', [0, 0, 0, 0, 1, 0, 1]),
    // Non-housing work: every tier recorded as 0.
    approval('0900010', 'JOB-100010', 'PMT-3000010', 'Building Permit', '2025-10-01', [0, 0, 0, 0, 0, 0, 0]),
    // No tier recorded.
    approval('0900006', 'JOB-100003', 'PMT-3000006', 'Combination Building Permit', '2025-08-09', null),
    approval('0900005', 'JOB-100005', 'PMT-3000007', 'Building Permit', '2025-01-10', null),
    // No job recorded: in no total.
    approval('0900007', '', 'PMT-3000008', 'Combination Building Permit', '2025-09-10', [0, 0, 0, 0, 7, 0, 0]),
    approval('0900008', 'JOB-100008', 'PMT-3000009', 'Electrical Pmt', '2025-09-11', null)] },
  // Same header, other stages: the same approvals recur in them and are not read.
  approvals_created_2025: { uri: 'created_2025', retrievedAt: '2026-10-07T00:00:00Z', rows: [
    approval('0900003', 'JOB-100001', 'PMT-3000003', 'Building Permit', '2025-05-06', [2, 3, 4, 5, 10, 6, 1])] },
  approvals_closed_2025: { uri: 'closed_2025', retrievedAt: '2026-10-07T00:00:00Z', rows: [
    approval('0900003', 'JOB-100001', 'PMT-3000003', 'Building Permit', '2025-05-06', [2, 3, 4, 5, 10, 6, 1])] },
};
const dataset = (name) => {
  const [stage, year] = tables[name].uri.split('_');
  return { snapshotId: name, datasetKind: 'REGIONAL_DATA', displayName: `Development approvals ${stage} in ${year}`, tableName: `analysis_${name}`, importedRows: tables[name].rows.length, rejectedRows: 0,
    sourceUri: `https://seshat.datasd.org/development_permits/approvals_${stage}_${year}_datasd.csv`, sourceSha256: 'e'.repeat(64), retrievedAt: tables[name].retrievedAt,
    observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} };
};
const schema = {
  datasets: Object.keys(tables).map(dataset), templates: [],
  columns: Object.keys(tables).flatMap((name) => header.map((column) => ({ tableName: `analysis_${name}`, columnName: column.toLowerCase(), logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null }))),
};
const latest = schema.datasets.find((item) => item.snapshotId === 'approvals_issued_2025');
export const fixture = { csv: Object.fromEntries(Object.entries(tables).map(([name, table]) => [name, csv(table.rows)])), schema, queries: housingQueries(schema, latest) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/housing-leads.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
