// Builds scripts/ui-review-fixtures-real.mjs, the data the UI review harness serves under
// `--data=real`, from the signed San Diego packs on local disk:
//
//   node scripts/build-real-ui-fixtures.mjs [--packs=/Volumes/A/onrecord-packs] [--harvest=<capture>/index.json]
//
// Each pack's parquet is exposed to the DuckDB CLI as the table the app would create on import:
// the publisher's original columns only, named `analysis_<kind>_<24 hex>` and typed by the app's
// import rules (storage/sqlite/analytical.rs `logical_type_for`, storage/tabular.rs `inferred`).
// The app's own discovery code (src/utils/automaticDiscovery.ts and the *Leads.ts builders) then
// produces the exact SQL the browser will send; every statement runs over the full parquet, and
// its result is serialized the way `execute_restricted_sql` returns it. Leads become findings in
// the format analysis/service_leads.rs stores. Row-level samples are ordered deterministically,
// so two runs over the same packs write the same bytes.
//
// `--harvest` adds the SQL a capture asked for and did not find (index.json `sqlMisses`) to
// scripts/ui-review-real-sql.json, which every later build also answers.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { compileUtil } from './compile-util.mjs';
import { FIXTURE_NOW } from './ui-review-fixtures.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const flag = (name) => argv.find((arg) => arg.startsWith(`--${name}=`))?.slice(name.length + 3) ?? null;
const packsRoot = flag('packs') ?? '/Volumes/A/onrecord-packs';
const duckdbBin = process.env.DUCKDB_BIN ?? '/opt/homebrew/bin/duckdb';
const outputFile = path.join(here, 'ui-review-fixtures-real.mjs');
const extraSqlFile = path.join(here, 'ui-review-real-sql.json');
const PACK_DATE = '2026-10-07';
const MAX_ROWS = 501;
const SIZE_LIMIT = 5 * 1024 * 1024;

const sha256 = (value) => createHash('sha256').update(value).digest('hex');
const quoteIdent = (value) => `"${value.replace(/"/g, '""')}"`;
const literal = (value) => `'${String(value).replace(/'/g, "''")}'`;

if (flag('harvest')) {
  const index = JSON.parse(readFileSync(flag('harvest'), 'utf8'));
  const known = existsSync(extraSqlFile) ? JSON.parse(readFileSync(extraSqlFile, 'utf8')) : [];
  const merged = [...new Set([...known, ...(index.sqlMisses ?? []).map((miss) => miss.sql)])].sort();
  writeFileSync(extraSqlFile, `${JSON.stringify(merged, null, 2)}\n`);
  console.log(`${merged.length - known.length} SQL statements harvested into ${path.basename(extraSqlFile)}`);
}

// ---- Packs and the tables the app imports from them ----------------------------------------

const manifests = Object.fromEntries(['money', 'reports', 'enforcement', 'capacity'].map((name) => {
  const directory = path.join(packsRoot, `san-diego-${name}-${PACK_DATE}`);
  const signed = JSON.parse(readFileSync(path.join(directory, 'manifest.json'), 'utf8'));
  return [`san-diego-${name}`, { directory, signed }];
}));

/**
 * One import per original file, as packs/import.rs does it. `kind` and `displayName` are
 * ingestion/queue/city_datasets.rs's for the file; `where` is the row filter its spec applies.
 */
const IMPORTS = [
  { pack: 'san-diego-money', table: 'budget_adopted', file: 'budget_operating_datasd', kind: 'OPERATING_BUDGET', displayName: 'City operating budget (all published years)' },
  { pack: 'san-diego-money', table: 'budget_actuals', file: 'actuals_operating_datasd', kind: 'REGIONAL_DATA', displayName: 'City operating actuals (all published years)' },
  { pack: 'san-diego-money', table: 'budget_departments', file: 'budget_reference_depts_datasd', kind: 'BUDGET_DIMENSION', displayName: 'City budget departments' },
  { pack: 'san-diego-money', table: 'city_leases', file: 'city_property_leases_datasd', kind: 'REGIONAL_DATA', displayName: 'City-owned properties, leases and permits' },
  ...[2024, 2025, 2026].map((year) => ({ pack: 'san-diego-reports', table: 'police_calls', file: `pd_calls_for_service_${year}_datasd`, kind: 'CALLS_FOR_SERVICE', displayName: `SDPD calls for service ${year}` })),
  { pack: 'san-diego-reports', table: 'call_types', file: 'pd_cfs_calltypes_datasd', kind: 'CALLS_CODEBOOK', displayName: 'SDPD call types' },
  { pack: 'san-diego-reports', table: 'call_dispositions', file: 'pd_dispo_codes_datasd', kind: 'CALLS_CODEBOOK', displayName: 'SDPD call dispositions' },
  { pack: 'san-diego-reports', table: 'police_beats', file: 'pd_beat_codes_list_datasd', kind: 'REGIONAL_DATA', displayName: 'SDPD beats' },
  ...[2024, 2025].map((year) => ({ pack: 'san-diego-reports', table: 'service_reports', file: `get_it_done_requests_closed_${year}_datasd`, kind: 'REGIONAL_DATA', displayName: `Get It Done closed reports ${year}, service_name = Encampment`, where: "service_name = 'Encampment'" })),
  { pack: 'san-diego-reports', table: 'service_reports', file: 'get_it_done_requests_open_datasd', kind: 'REGIONAL_DATA', displayName: 'Get It Done open reports (all services)' },
  { pack: 'san-diego-enforcement', table: 'police_stops', file: 'ripa_stops_datasd', kind: 'RIPA_STOPS', displayName: 'SDPD stop records (RIPA), persons stopped' },
  { pack: 'san-diego-enforcement', table: 'police_stop_reasons', file: 'ripa_stop_reason_datasd', kind: 'REGIONAL_DATA', displayName: 'SDPD stop records (RIPA), reasons for the stop' },
  { pack: 'san-diego-enforcement', table: 'police_stop_results', file: 'ripa_stop_result_datasd', kind: 'REGIONAL_DATA', displayName: 'SDPD stop records (RIPA), results of the stop' },
  { pack: 'san-diego-enforcement', table: 'police_offenses', file: 'pd_nibrs_2025_datasd', kind: 'NIBRS_OFFENSES', displayName: 'SDPD NIBRS offenses 2025' },
  ...[1, 2].map((part) => ({ pack: 'san-diego-enforcement', table: 'parking_citations', file: `parking_citations_2025_part${part}_datasd`, kind: 'REGIONAL_DATA', displayName: `Parking citations 2025, ${part === 1 ? 'January to June' : 'July to December'}` })),
  ...['created', 'issued', 'closed'].map((stage) => ({ pack: 'san-diego-capacity', table: `approvals_${stage}`, file: `approvals_${stage}_2025_datasd`, kind: 'REGIONAL_DATA', displayName: `Development approvals ${stage} in 2025` })),
  { pack: 'san-diego-capacity', table: 'approvals_issued', file: 'approvals_issued_2024_datasd', kind: 'REGIONAL_DATA', displayName: 'Development approvals issued in 2024' },
];

/** The pack conformance columns appended after the publisher's own; the app imports the original CSV, which has none of them. */
const PACK_COLUMNS = new Set(['record_id', 'publisher_record_id', 'source_id', 'source_url', 'retrieved_at', 'source_sha256', 'source_row', 'pack_version', 'occurred_at', 'occurred_on', 'calendar_year', 'fiscal_year', 'time_basis']);

// ---- DuckDB CLI -------------------------------------------------------------------------------

const work = mkdtempSync(path.join(os.tmpdir(), 'onrecord-real-fixtures-'));
const database = path.join(work, 'packs.duckdb');
const duck = (sql) => {
  // One thread: some of the app's SQL aggregates with an unordered STRING_AGG, whose order otherwise varies per run.
  const output = execFileSync(duckdbBin, [database, '-json', '-cmd', 'SET threads=1', '-c', sql], { encoding: 'utf8', maxBuffer: 1 << 30, stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  return output ? JSON.parse(output) : [];
};

/** app: storage/sqlite/analytical.rs `logical_type_for`, for every kind but regional data. */
function documentedType(kind, header) {
  if (kind === 'OPERATING_BUDGET') return header === 'amount' ? 'NUMBER' : header.endsWith('_number') ? 'IDENTIFIER' : 'CATEGORY';
  if (['latitude', 'longitude', 'lat', 'lon', 'lng'].includes(header)) return 'FLOAT';
  if (header.includes('date') && header.includes('time')) return 'LOCAL_TIMESTAMP';
  if (header.includes('date') || header === 'occured_on') return 'DATE';
  if (header.includes('time')) return 'TIMESTAMP';
  if (header.includes('amount') || header.includes('budget')) return 'DECIMAL';
  if (['uid', 'id', 'pid', 'incident_num', 'case_number', 'nibrs_uniq'].includes(header)) return 'IDENTIFIER';
  return 'CATEGORY';
}
/** app: storage/tabular.rs `inferred`; the pack builder's parquet type stands in for the value profile. */
function inferredType(header, parquetType) {
  if (header === 'id' || header.endsWith('_id') || header.endsWith('code') || header.includes('fips') || header.includes('geoid') || header.includes('zip') || header.includes('tract') || header.endsWith('_number')) return 'IDENTIFIER';
  if (/^(BIGINT|INTEGER|SMALLINT|TINYINT|HUGEINT)$/.test(parquetType)) return 'INTEGER';
  if (parquetType.startsWith('DECIMAL')) return /(^|_)(lat|lng|lon|latitude|longitude)$/.test(header) ? 'FLOAT' : 'NUMBER';
  if (parquetType === 'DOUBLE' || parquetType === 'FLOAT') return 'FLOAT';
  if (parquetType === 'DATE') return 'DATE';
  return 'CATEGORY';
}
/** app: storage/sqlite/analytical.rs `duckdb_type`. */
const duckdbType = (logical) => ({ INTEGER: 'BIGINT', NUMBER: 'DECIMAL(38,10)', FLOAT: 'DOUBLE', DATE: 'DATE', TIMESTAMP: 'TIMESTAMP', LOCAL_TIMESTAMP: 'TIMESTAMP', DECIMAL: 'DECIMAL(18,2)' })[logical] ?? 'TEXT';
const isText = (logical) => duckdbType(logical) === 'TEXT';

const datasets = [];
const columns = [];
const manifestFile = (pack, relative) => manifests[pack].signed.manifest.files.find((file) => file.path === relative);

for (const spec of IMPORTS) {
  const { directory } = manifests[spec.pack];
  const parquet = path.join(directory, 'parquet', spec.table, `${spec.file}.parquet`);
  const original = manifestFile(spec.pack, `originals/${spec.table}/${spec.file}.csv`);
  if (!existsSync(parquet) || !original) throw new Error(`${spec.pack} has no ${spec.table}/${spec.file}`);
  const identity = sha256(JSON.stringify(['regional-tables-v4', original.sha256, original.sourceUrl, null, null, null, original.partial]));
  const kind = spec.kind.toLowerCase();
  const tableName = `analysis_${kind}_${identity.slice(0, 24)}`;
  const snapshotId = `${kind.replace(/_/g, '-')}-${identity.slice(0, 24)}`;
  const source = `read_parquet(${literal(parquet)})${spec.where ? ` WHERE ${spec.where}` : ''}`;
  const described = duck(`DESCRIBE SELECT * FROM read_parquet(${literal(parquet)})`).filter((column) => !PACK_COLUMNS.has(column.column_name));
  const documented = spec.kind !== 'REGIONAL_DATA';
  const planned = described.map(({ column_name: name, column_type: parquetType }) => ({ name, parquetType, logical: documented ? documentedType(spec.kind, name) : inferredType(name, parquetType) }));
  // A documented type no value converts to is the wrong hint for this file: the column stays text.
  const typed = planned.filter((column) => !isText(column.logical));
  const conversions = typed.length ? duck(`SELECT ${typed.map((column, index) => `COUNT(${quoteIdent(column.name)}) AS n${index}, COUNT(TRY_CAST(${quoteIdent(column.name)} AS ${duckdbType(column.logical)})) AS c${index}`).join(', ')} FROM ${source}`)[0] : {};
  const unconvertedByColumn = {};
  typed.forEach((column, index) => {
    const present = Number(conversions[`n${index}`]);
    const converted = Number(conversions[`c${index}`]);
    if (present > 0 && converted === 0) column.logical = 'CATEGORY';
    else if (present > converted) unconvertedByColumn[column.name] = present - converted;
  });
  const select = planned.map((column) => {
    const name = quoteIdent(column.name);
    if (isText(column.logical)) return `CAST(${name} AS TEXT) AS ${name}`;
    return `TRY_CAST(${name} AS ${duckdbType(column.logical)}) AS ${name}`;
  });
  duck(`CREATE VIEW ${quoteIdent(tableName)} AS SELECT ${select.join(', ')} FROM ${source}`);
  const importedRows = Number(duck(`SELECT COUNT(*) AS n FROM ${quoteIdent(tableName)}`)[0].n);
  datasets.push({
    snapshotId, datasetKind: spec.kind, displayName: spec.displayName, tableName, importedRows, rejectedRows: 0,
    sourceUri: original.sourceUrl, sourceSha256: original.sha256, retrievedAt: original.retrievedAt.replace(/\.\d+Z$/, 'Z'),
    observationPeriodStart: null, observationPeriodEnd: null, partialDataset: Boolean(original.partial || spec.where),
    validationSummary: JSON.stringify({ unconvertedByColumn, warnings: [], packOrigin: { pack: spec.pack, path: original.path } }), unconvertedByColumn,
  });
  planned.forEach((column) => columns.push({ tableName, columnName: column.name, logicalType: column.logical, originalName: column.name, nullable: true, dictionary: null }));
  console.log(`${spec.displayName}: ${importedRows.toLocaleString('en-US')} rows -> ${tableName}`);
}

// app: storage/sqlite/analytical.rs `analytical_schema`, which orders by retrieval, newest first.
datasets.sort((a, b) => b.retrievedAt.localeCompare(a.retrievedAt) || a.tableName.localeCompare(b.tableName));
columns.sort((a, b) => a.tableName.localeCompare(b.tableName));
const templates = datasets.flatMap((dataset) => {
  const own = columns.filter((column) => column.tableName === dataset.tableName);
  const has = (name) => own.some((column) => column.columnName === name);
  const table = quoteIdent(dataset.tableName);
  return [
    ...(has('report_fy') && !has('budget_cycle') ? [{ templateId: `${dataset.snapshotId}-fiscal-year-coverage`, title: `Fiscal-year coverage: ${dataset.displayName}`, description: 'Inspect the fiscal-year codes actually represented in this snapshot. A recent publication date does not establish recent observations.', sql: `SELECT report_fy, COUNT(*) AS imported_rows FROM ${table} GROUP BY 1 ORDER BY 1`, denominator: 'Rows in the selected snapshot, grouped by its own fiscal-year code.' }] : []),
    ...(has('report_fy') && has('budget_cycle') && own.some((column) => column.columnName === 'amount' && ['NUMBER', 'INTEGER', 'DECIMAL'].includes(column.logicalType)) ? [{ templateId: `${dataset.snapshotId}-budget-coverage`, title: `Budget year and cycle coverage: ${dataset.displayName}`, description: 'Inspect fiscal years, reported budget cycles, and missing amounts before selecting appropriations. Blank cycles remain unspecified.', sql: `SELECT report_fy, COALESCE(NULLIF(LOWER(TRIM(budget_cycle)), ''), 'UNSPECIFIED') AS reported_cycle, COUNT(*) AS allocation_rows, COUNT(*) - COUNT(amount) AS missing_amounts FROM ${table} GROUP BY 1, 2 ORDER BY 1, 2`, denominator: 'Imported budget rows by the publisher\'s fiscal-year and cycle fields. Missing status is not assumed to be adopted.' }] : []),
    { templateId: `${dataset.snapshotId}-profile`, title: `Profile ${dataset.displayName}`, description: 'Missing values, distinct values, and minimum/maximum for the first 64 columns; text bounds are lexical.', sql: own.slice(0, 64).map((column) => `SELECT '${column.columnName.replace(/'/g, "''")}' AS column_name, COUNT(*) AS rows, COUNT(*) - COUNT(${quoteIdent(column.columnName)}) AS missing, COUNT(DISTINCT ${quoteIdent(column.columnName)}) AS distinct_values, CAST(MIN(${quoteIdent(column.columnName)}) AS VARCHAR) AS minimum, CAST(MAX(${quoteIdent(column.columnName)}) AS VARCHAR) AS maximum FROM ${table}`).join(' UNION ALL '), denominator: 'Imported rows in this snapshot; rejected rows and partial coverage remain in the snapshot ledger.' },
    { templateId: `${dataset.snapshotId}-count`, title: `Count ${dataset.displayName}`, description: 'Count imported rows without assuming that rows represent unique people, events, or properties.', sql: `SELECT COUNT(*) AS imported_rows FROM ${table}`, denominator: 'One row of the selected snapshot.' },
  ];
});
const schema = { datasets, columns, templates };

// ---- Restricted SQL, serialized as execute_restricted_sql returns it ---------------------------

/**
 * app: storage/sqlite/analytical.rs `duckdb_value_to_json`. DECIMAL and HUGEINT arrive as strings,
 * DATE as YYYY-MM-DD, TIMESTAMP as the duckdb-rs `ValueRef` Debug text the native command emits.
 */
function serialize(type, name) {
  if (/^(DECIMAL|HUGEINT|UHUGEINT)/.test(type) || type === 'DATE') return `CAST(${name} AS VARCHAR)`;
  if (type === 'TIMESTAMP') return `CASE WHEN ${name} IS NULL THEN NULL ELSE 'Timestamp(Microsecond, ' || epoch_us(${name}) || ')' END`;
  if (/^(BOOLEAN|TINYINT|SMALLINT|INTEGER|BIGINT|UTINYINT|USMALLINT|UINTEGER|UBIGINT|FLOAT|DOUBLE|VARCHAR)$/.test(type)) return name;
  return `CAST(${name} AS VARCHAR)`;
}
const referenced = (sql) => datasets.filter((dataset) => sql.includes(dataset.tableName)).map((dataset) => `${dataset.tableName}@${dataset.snapshotId}`);
/** Stored with up to 501 rows; the handler slices to each call's limit and sets `truncated`. */
const SQL = {};
function execute(sql) {
  if (SQL[sql]) return SQL[sql];
  const body = sql.trim().replace(/;+$/, '');
  const started = process.hrtime.bigint();
  try {
    const shape = duck(`DESCRIBE SELECT * FROM (${body}) AS onrecord_user_query`);
    const aliases = shape.map((_, index) => `c${index}`);
    const rows = duck(`SELECT ${shape.map((column, index) => `${serialize(column.column_type, aliases[index])} AS ${aliases[index]}`).join(', ')} FROM (SELECT * FROM (${body}) AS onrecord_user_query LIMIT ${MAX_ROWS}) AS t(${aliases.join(', ')})`)
      .map((row) => aliases.map((alias) => row[alias] ?? null));
    SQL[sql] = { columns: shape.map((column) => column.column_name), rows, elapsedMs: Math.max(1, Math.round(Number(process.hrtime.bigint() - started) / 1e6)), provenance: referenced(body) };
  } catch (error) {
    const message = String(error.stderr ?? error.message).split('\n').filter(Boolean).slice(0, 3).join(' ');
    SQL[sql] = { error: `SQL could not be prepared: ${message}` };
  }
  return SQL[sql];
}
const asResult = (entry, limit) => ({ columns: entry.columns, rows: entry.rows.slice(0, limit), rowCount: Math.min(entry.rows.length, limit), truncated: entry.rows.length > limit, elapsedMs: entry.elapsedMs, provenance: entry.provenance });

// ---- The app's own discovery, as DiscoveryController runs it (limit 100 per query) --------------

const { automaticProfileQuery, automaticFollowups, storedLeads, profileObservations } = compileUtil('automaticDiscovery');
const { budgetComparisonCandidates, buildBudgetComparison } = compileUtil('budgetComparison');
const profiles = {};
for (const dataset of datasets) {
  const queue = [automaticProfileQuery(schema, dataset)];
  const results = [];
  while (queue.length) {
    const query = queue.shift();
    const entry = execute(query.sql);
    if (entry.error) { results.push({ query, error: entry.error }); continue; }
    const result = asResult(entry, 100);
    results.push({ query, result });
    if (query.id === 'quality') queue.push(...automaticFollowups(schema, dataset, result));
  }
  profiles[dataset.snapshotId] = { fingerprint: dataset.snapshotId, checkedAt: new Date(FIXTURE_NOW).toISOString(), results };
  const failed = results.filter((entry) => entry.error);
  console.log(`discovery ${dataset.displayName}: ${results.length} queries${failed.length ? `, ${failed.length} failed: ${failed[0].error.slice(0, 160)}` : ''}`);
}
const { leads } = storedLeads(schema, profiles);
const observations = datasets.flatMap((dataset) => profileObservations(schema, dataset, profiles[dataset.snapshotId]));
console.log(`${leads.length} leads, ${observations.length} observations`);

/** app: analysis/service_leads.rs `to_findings`. */
const FAMILY_KIND = { SERVICE_REPORTS: 'SERVICE_REPORT_LEAD', POLICE_STOPS: 'POLICE_STOP_LEAD', ABATEMENT_LOGS: 'ABATEMENT_LOG_LEAD', CHARGE_LOGS: 'CHARGE_LOG_LEAD', DISCLOSURE_FILINGS: 'DISCLOSURE_FILING_LEAD', CAPACITY_COMPARISON: 'CAPACITY_COMPARISON_LEAD', SPENDING_RECORDS: 'SPENDING_RECORD_LEAD', POLICE_CALLS: 'POLICE_CALL_LEAD', PARKING_CITATIONS: 'PARKING_CITATION_LEAD', HOUSING_APPROVALS: 'HOUSING_APPROVAL_LEAD' };
const sourceLine = (position, dataset) => `${position + 1}. '${dataset.displayName}' (${dataset.sourceUri ?? 'no source address recorded'}), retrieved ${dataset.retrievedAt.slice(0, 10)}, sha256 ${dataset.sourceSha256}`;
const now = new Date(FIXTURE_NOW).toISOString();
const LEAD_FINDINGS = leads.map((lead) => {
  const sources = lead.snapshotIds.map((id) => datasets.find((dataset) => dataset.snapshotId === id));
  const kind = FAMILY_KIND[lead.family];
  return {
    id: sha256(['kind', kind, 'detector', 'service_report_leads', sources[0].snapshotId, sources[0].sourceSha256, lead.key, lead.title, lead.detail].join('\u001f')),
    kind, severity: 'NOTABLE', confidence: { kind: 'UNASSESSED' },
    subject: { portalKey: null, agencyId: null, requestId: null, documentId: null, entityId: null },
    headline: lead.title.trim(),
    explanation: `${lead.detail.trim()} Method: ${lead.method.trim()} Worth asking: ${lead.question.trim()}\n\nTables read:\n${sources.map((dataset, position) => sourceLine(position, dataset)).join('\n')}\n\nSQL:\n${lead.sql.trim()}`,
    citations: [], discoveredAt: now, lastSeenAt: now, dismissedAt: null, sourceDetector: 'service_report_leads',
  };
});

// ---- Other surfaces the packs hold -------------------------------------------------------------

const tableOf = (file, where = (dataset) => dataset.displayName.includes(file)) => datasets.find(where);
const budget = tableOf('City operating budget');
const actuals = tableOf('City operating actuals');
const leases = tableOf('leases and permits');
const encampments2025 = tableOf('Get It Done closed reports 2025');
const value = (sql) => duck(sql);

const BUDGET_COMPARISON = { budgetSnapshotId: budget.snapshotId, actualsSnapshotId: actuals.snapshotId, fiscalYear: '23', cycle: 'adopted', accountClass: 'expense' };
if (!budgetComparisonCandidates(schema, 'budget').some((dataset) => dataset.snapshotId === budget.snapshotId)) throw new Error('The budget table is not a comparison candidate');
const budgetComparisonSql = buildBudgetComparison(schema, BUDGET_COMPARISON);
/** A row-level sample a reader would open from the SQL panel: long public text, real addresses, nulls. */
const PREVIEW_SQL = `SELECT service_request_id, date_requested, date_closed, status, street_address, zipcode, council_district, comm_plan_name, case_age_days, case_origin, referred, public_description FROM ${quoteIdent(encampments2025.tableName)} ORDER BY date_requested DESC, service_request_id LIMIT 500`;
const extraSql = existsSync(extraSqlFile) ? JSON.parse(readFileSync(extraSqlFile, 'utf8')) : [];
for (const sql of [budgetComparisonSql, PREVIEW_SQL, ...templates.map((template) => template.sql), ...extraSql]) execute(sql);
const missing = Object.entries(SQL).filter(([, entry]) => entry.error);
if (missing.length) console.log(`${missing.length} statements fail in DuckDB (served as errors): ${missing.slice(0, 3).map(([sql, entry]) => `${sql.slice(0, 60)}… ${entry.error.slice(0, 120)}`).join(' | ')}`);

const FINANCE = {
  datasets: [budget, actuals].map((dataset) => ({ snapshotId: dataset.snapshotId, name: dataset.displayName, tableName: dataset.tableName, sourceUrl: dataset.sourceUri, sourceSha256: dataset.sourceSha256, columns: columns.filter((column) => column.tableName === dataset.tableName).map((column) => column.columnName), partial: dataset.partialDataset })),
  fiscalYears: value(`SELECT DISTINCT TRY_CAST(report_fy AS INTEGER) AS fy FROM ${quoteIdent(budget.tableName)} WHERE TRY_CAST(report_fy AS INTEGER) IS NOT NULL ORDER BY 1`).map((row) => 2000 + Number(row.fy)),
  funds: value(`SELECT DISTINCT fund_number FROM ${quoteIdent(budget.tableName)} WHERE fund_number IS NOT NULL ORDER BY 1`).map((row) => row.fund_number),
  departments: value(`SELECT DISTINCT dept_name FROM ${quoteIdent(budget.tableName)} WHERE dept_name IS NOT NULL ORDER BY 1`).map((row) => row.dept_name),
};
// Companies only: the lessee name column also holds private individuals.
const PAYEES = value(`SELECT TRIM(lessee_company) AS vendor, COUNT(*) AS n FROM ${quoteIdent(leases.tableName)} WHERE NULLIF(TRIM(lessee_company), '') IS NOT NULL GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 40`)
  .map((row) => ({ vendorValue: row.vendor, rows: Number(row.n), datasetNames: [leases.displayName] }));
const BLOCKS = value(`SELECT street_address, COUNT(*) AS n FROM ${quoteIdent(encampments2025.tableName)} WHERE street_address IS NOT NULL GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 6`).map((row) => ({ address: row.street_address, reports: Number(row.n) }));

/** packs/import.rs `summarize`: originals only, grouped by table. */
const PACKS = Object.entries(manifests).map(([pack, { signed }]) => {
  const manifest = signed.manifest;
  const tables = [];
  for (const file of manifest.files.filter((entry) => entry.format === 'ORIGINAL')) {
    const summary = { path: file.path, year: file.year ?? null, rows: file.rows, bytes: file.bytes, partial: file.partial, sourceUrl: file.sourceUrl, retrievedAt: file.retrievedAt, datasetKind: IMPORTS.find((spec) => file.path === `originals/${spec.table}/${spec.file}.csv`)?.kind ?? null, licence: file.licence ?? null };
    const table = tables.find((entry) => entry.table === file.table);
    if (table) table.files.push(summary);
    else tables.push({ table: file.table, files: [summary], unchangedChecks: 0, lastCheckedAt: null, changeRecords: 0 });
  }
  return { pack, snapshots: false, packVersion: manifest.packVersion, schemaVersion: manifest.schemaVersion, builtAt: manifest.builtAt, signerKeyId: signed.signature.signerKeyId, licenceId: manifest.licence.id, licenceUrl: manifest.licence.url, disclaimer: manifest.licence.disclaimer, manifestSha256: signed.manifestSha256, tables };
});

// ---- Write -------------------------------------------------------------------------------------

const header = `// Generated by scripts/build-real-ui-fixtures.mjs from the signed San Diego packs (${PACK_DATE}).
// Do not edit by hand; rebuild instead. Every value below was read from the packs' parquet or
// manifests, or computed from them by the app's own query builders.
/* eslint-disable */
`;
const exported = { SCHEMA: schema, SQL, LEAD_FINDINGS, PACKS, FINANCE, PAYEES, BLOCKS, BUDGET_COMPARISON, PREVIEW_SQL };
const body = Object.entries(exported).map(([name, data]) => `export const ${name} = ${JSON.stringify(data)};\n`).join('');
writeFileSync(outputFile, header + body);
rmSync(work, { recursive: true, force: true });
const bytes = Buffer.byteLength(header + body);
console.log(`${path.basename(outputFile)}: ${(bytes / 1024 / 1024).toFixed(2)} MB, ${Object.keys(SQL).length} statements, sha256 ${sha256(header + body).slice(0, 16)}`);
if (bytes > SIZE_LIMIT) {
  console.error(`The generated module exceeds ${SIZE_LIMIT} bytes.`);
  process.exit(1);
}
