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
const { callQueries } = compile('callLeads');
const quoted = (value) => (value === '' ? '' : `"${value}"`);
// The year files quote every header and text value in upper case; the lookups start with a BOM
// and end lines with CRLF, and the call-type list has three blank trailing headers.
const yearCsv = (rows) => `${[['INCIDENT_NUM', 'DATE_TIME', 'DAY_OF_WEEK', 'CALL_TYPE', 'DISPOSITION', 'BEAT', 'PRIORITY'].map(quoted).join(','),
  ...rows.map(([incident, at, type, dispo, beat]) => [quoted(incident), quoted(`${at}.000`), 1, quoted(type), quoted(dispo), quoted(beat), quoted('2')].join(','))].join('\n')}\n`;
const lookupCsv = (lines) => `﻿${lines.join('\r\n')}\r\n`;
const year = (number) => `https://seshat.datasd.org/police_calls_for_service/pd_calls_for_service_${number}_datasd.csv`;
const tables = {
  // E25001 is repeated; its later entry (A) is kept. The lower-case sleeper is SLEEPER; 5150V and
  // 415 are not read. E25005 has no disposition, E25006 no beat.
  cfs_2025: { uri: year(2025), retrievedAt: '2026-10-01T00:00:00Z', csv: yearCsv([
    ['E25001', '2025-03-01 10:00:00', 'SLEEPER', 'K', '521'], ['E25001', '2025-03-01 11:00:00', 'SLEEPER', 'A', '521'], ['E25002', '2025-04-01 09:00:00', 'SLEEPER', 'AHR', '521'],
    ['E25003', '2025-05-01 09:00:00', ' sleeper ', 'K', '524'], ['E25004', '2025-06-01 09:00:00', 'SLEEPER', 'CAN', '524'], ['E25005', '2025-07-01 09:00:00', 'SLEEPER', '', '999'],
    ['E25006', '2025-08-01 09:00:00', 'SLEEPER', 'KHR', ''], ['E25007', '2025-08-02 09:00:00', 'CW', 'U', '521'], ['E25008', '2025-08-03 09:00:00', '5150V', 'A', '521'],
    ['E25009', '2025-08-04 09:00:00', '602', 'R', '524'], ['E25010', '2025-08-05 09:00:00', '415', 'A', '521']]) },
  // An earlier copy of the current year: E26001 was later re-coded K, and E26099 left the file.
  cfs_prior_2026: { uri: year(2026), retrievedAt: '2026-09-01T00:00:00Z', csv: yearCsv([
    ['E26001', '2026-01-05 09:00:00', 'SLEEPER', 'A', '521'], ['E26002', '2026-02-01 09:00:00', 'SLEEPER', 'X', '521'], ['E26003', '2026-03-01 09:00:00', 'SLEEPER', 'R', '524'],
    ['E26099', '2026-03-02 09:00:00', 'SLEEPER', 'K', '524']]) },
  cfs_2026: { uri: year(2026), retrievedAt: '2026-10-04T00:00:00Z', csv: yearCsv([
    ['E26001', '2026-01-05 09:00:00', 'SLEEPER', 'K', '521'], ['E26002', '2026-02-01 09:00:00', 'SLEEPER', 'X', '521'], ['E26003', '2026-03-01 09:00:00', 'SLEEPER', 'R', '524'],
    ['E26004', '2026-04-01 09:00:00', 'SLEEPER', 'O', '524'], ['E26005', '2026-05-01 09:00:00', 'SLEEPER', 'DUP', '521'], ['E26006', '2026-06-01 09:00:00', '647F', 'K', '521'],
    ['E26008', '2026-08-01 09:00:00', 'SLEEPER', 'K', '524']]) },
  cfs_types: { uri: 'https://seshat.datasd.org/police_calls_for_service/pd_cfs_calltypes_datasd.csv', retrievedAt: '2026-10-04T00:00:00Z', csv: lookupCsv([
    'call_type,description,,,', '5150,MENTAL CASE,,,', '602,TRESPASSING,,,', '647F,DRUNK - DRUGS/ALCOHOL,,,', 'CW,CHECK THE WELFARE,,,', 'SLEEPER,SLEEPER,,,']) },
  cfs_dispositions: { uri: 'https://seshat.datasd.org/police_calls_for_service/pd_dispo_codes_datasd.csv', retrievedAt: '2026-10-04T00:00:00Z', csv: lookupCsv([
    'dispo_code,description', 'X,DISPATCHED ALARM CALL CANCELLED BEFORE UNIT ONSCENE', 'A,ARREST MADE', 'AHR,"ARREST MADE, HIGH RISK (CRISIS RESPONSE TEAM)"', 'R,REPORT', 'O,OTHER',
    'K,NO REPORT REQUIRED', 'KHR,"NO FURTHER ACTION, HIGH RISK (CRISIS RESPONSE TEAM)"', 'U,UNFOUNDED', 'CAN,CANCEL', 'DUP,"DUPLICATE CALL, XREF\'D"']) },
  cfs_beats: { uri: 'https://seshat.datasd.org/gis_police_beats/pd_beat_codes_list_datasd.csv', retrievedAt: '2026-10-04T00:00:00Z', csv: lookupCsv([
    'beat,neighborhood', '521,East Village', '524,Core-Columbia']) },
};
const headers = { cfs_types: ['call_type', 'description', 'field_3', 'field_4', 'field_5'], cfs_dispositions: ['dispo_code', 'description'], cfs_beats: ['beat', 'neighborhood'] };
const yearHeaders = ['incident_num', 'date_time', 'day_of_week', 'call_type', 'disposition', 'beat', 'priority'];
const dataset = (name) => ({ snapshotId: name, datasetKind: name.startsWith('cfs_20') || name.startsWith('cfs_prior') ? 'CALLS_FOR_SERVICE' : 'CALLS_CODEBOOK', displayName: `Calls fixture, ${name}`,
  tableName: `analysis_${name}`, importedRows: tables[name].csv.trim().split('\n').length - 1, rejectedRows: 0, sourceUri: tables[name].uri, sourceSha256: 'e'.repeat(64), retrievedAt: tables[name].retrievedAt,
  observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} });
const schema = {
  datasets: Object.keys(tables).map(dataset), templates: [],
  columns: Object.keys(tables).flatMap((name) => (headers[name] ?? yearHeaders).map((column) => ({ tableName: `analysis_${name}`, columnName: column, logicalType: 'CATEGORY', originalName: column, nullable: true, dictionary: null }))),
};
const latest = schema.datasets.find((item) => item.snapshotId === 'cfs_2026');
export const fixture = { csv: Object.fromEntries(Object.entries(tables).map(([name, table]) => [name, table.csv])), schema, queries: callQueries(schema, latest) };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/call-leads.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
