import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { compileUtil } from './compile-util.mjs';

const [database, output] = process.argv.slice(2);
if (!database || !output) throw new Error('Usage: node scripts/audit-saved-discovery.mjs <copied-core.db> <report.json>');
const duck = (sql) => {
  const result = execFileSync(process.env.DUCKDB_BIN ?? 'duckdb', ['-readonly', database, '-json', '-cmd', 'SET threads=1', '-c', sql], { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 });
  return result.trim() ? JSON.parse(result) : [];
};
const datasets = duck(`SELECT snapshot_id AS "snapshotId", dataset_kind AS "datasetKind", display_name AS "displayName", table_name AS "tableName", imported_rows AS "importedRows", rejected_rows AS "rejectedRows", source_uri AS "sourceUri", source_sha256 AS "sourceSha256", retrieved_at AS "retrievedAt", observation_period_start AS "observationPeriodStart", observation_period_end AS "observationPeriodEnd", partial_dataset <> 0 AS "partialDataset", validation_summary_json AS "validationSummary" FROM analytical_snapshots ORDER BY retrieved_at DESC, snapshot_id`).map((dataset) => ({ ...dataset, unconvertedByColumn: JSON.parse(dataset.validationSummary).unconvertedByColumn ?? {} }));
const columns = duck(`SELECT table_name AS "tableName", column_name AS "columnName", logical_type AS "logicalType", original_name AS "originalName", nullable <> 0 AS nullable, dictionary FROM analytical_columns ORDER BY snapshot_id, ordinal`);
const schema = { datasets, columns, templates: [] };
const { automaticProfileQuery, automaticFollowups, storedLeads } = compileUtil('automaticDiscovery');
const profiles = {};
for (const dataset of datasets) {
  const queue = [automaticProfileQuery(schema, dataset)];
  const results = [];
  while (queue.length) {
    const query = queue.shift();
    try {
      const body = query.sql.trim().replace(/;+$/, '');
      const shape = duck(`DESCRIBE SELECT * FROM (${body}) AS audit_query`);
      const names = shape.map((column) => column.column_name);
      const aliases = shape.map((_, index) => `c${index}`);
      const projection = shape.map((column, index) => /^(DECIMAL|HUGEINT|UHUGEINT)/.test(column.column_type) ? `CAST(c${index} AS VARCHAR) AS c${index}` : `c${index}`).join(', ');
      const rows = duck(`SELECT ${projection} FROM (${body}) AS audit_query(${aliases.join(', ')}) LIMIT 101`).map((row) => aliases.map((alias) => row[alias] ?? null));
      const result = { columns: names, rows: rows.slice(0, 100), rowCount: Math.min(rows.length, 100), truncated: rows.length > 100, elapsedMs: 0, provenance: [dataset.snapshotId] };
      results.push({ query, result });
      if (query.id === 'quality') queue.push(...automaticFollowups(schema, dataset, result));
    } catch (error) {
      results.push({ query, error: String(error.stderr ?? error.message) });
    }
  }
  profiles[dataset.snapshotId] = { fingerprint: dataset.snapshotId, checkedAt: new Date().toISOString(), results };
  console.log(`${dataset.displayName}: ${results.length} checks, ${results.filter((entry) => entry.error).length} failed`);
}
const result = storedLeads(schema, profiles);
writeFileSync(output, `${JSON.stringify({ capturedAt: new Date().toISOString(), limitation: 'Read-only replay of current frontend query builders on a copied database. Does not exercise native SQL validation, source lifecycle filtering, finding persistence, or the installed application.', schema, profiles, ...result }, null, 2)}\n`);
console.log(`${result.leads.length} leads; complete=${result.complete}`);
if (!result.complete) process.exitCode = 1;
