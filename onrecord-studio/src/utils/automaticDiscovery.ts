import type { AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { buildDiscoveryPlan, discoveryDefaults, type DiscoveryQuery, type DiscoverySelection } from './dataDiscovery';

export interface DiscoveryObservation {
  id: string;
  snapshotId: string;
  kind: 'coverage' | 'quality' | 'pattern';
  priority: number;
  title: string;
  detail: string;
  question: string;
  query?: DiscoveryQuery;
  selection?: DiscoverySelection;
}
export interface AutomaticProfile {
  fingerprint: string;
  checkedAt: string;
  results: { query: DiscoveryQuery; result?: SqlQueryResult; error?: string }[];
}

const timeNames = /^(report_fy|fiscal_year|year|date|observed|observation_date|report_date|created_date|incident_date)$/;
const dimensionNames = /^(city|jurisdiction|dept_name|department|agency|agency_name|program|program_name|community|district|zip|zip_code|status|category|type|budget_cycle|phase)$/;
const number = (value: unknown): number | null => {
  if (value == null || value === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};
const display = (value: unknown) => value == null ? 'missing values' : String(value);
const records = (result: SqlQueryResult) => result.rows.map((row) => Object.fromEntries(result.columns.map((name, i) => [name, row[i]])));

export function snapshotFingerprint(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): string {
  return JSON.stringify([dataset, schema.columns.filter((column) => column.tableName === dataset.tableName)]);
}

export function automaticProfileQuery(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery {
  const columns = schema.columns.filter((column) => column.tableName === dataset.tableName);
  const ordered = [...columns].sort((a, b) => {
    const rank = (name: string) => timeNames.test(name) ? 0 : dimensionNames.test(name) ? 1 : 2;
    return rank(a.columnName) - rank(b.columnName);
  }).slice(0, 12);
  const plan = buildDiscoveryPlan({ ...schema, columns: ordered }, { ...discoveryDefaults(schema, dataset.snapshotId), period: '' });
  return { ...plan.queries.find((query) => query.id === 'quality')!, method: 'Full-row profile of up to 12 prioritized columns. Time and grouping fields are checked first; source record counts are not counts of unique people or events.' };
}

export function automaticFollowups(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary, profile: SqlQueryResult): DiscoveryQuery[] {
  const rows = records(profile);
  const eligible = rows.filter((row) => dimensionNames.test(String(row.column_name)) && (number(row.distinct_values) ?? 0) >= 2 && (number(row.distinct_values) ?? 0) <= 100);
  const selection = { ...discoveryDefaults(schema, dataset.snapshotId), dimension: eligible[0] ? String(eligible[0].column_name) : '' };
  // One consistent counting unit, no inferred sums, rates, or cross-publisher joins.
  const plan = buildDiscoveryPlan(schema, selection);
  return plan.queries.filter((query) => (query.id === 'coverage' && selection.period) || (query.id === 'groups' && selection.dimension) || (query.id === 'changes' && selection.period));
}

export function metadataObservations(dataset: AnalyticalDatasetSummary): DiscoveryObservation[] {
  const base = { snapshotId: dataset.snapshotId, kind: 'coverage' as const };
  const observations: DiscoveryObservation[] = [];
  if (dataset.rejectedRows > 0) observations.push({ ...base, kind: 'quality', id: `${dataset.snapshotId}:rejected`, priority: 90,
    title: `${dataset.rejectedRows.toLocaleString()} source rows need repair`,
    detail: `${dataset.importedRows.toLocaleString()} rows were imported; rejected rows remain in the source ledger. Automatic repairs never replace suppression markers with zero.`,
    question: 'Are rejected records concentrated in a particular period or provider?',
    query: { id: 'rejected', title: 'Rejected source records', method: 'Original source cells and parser errors.', sql: `SELECT row_number, source_payload_json, validation_errors_json FROM analytical_rows WHERE snapshot_id = '${dataset.snapshotId.replace(/'/g, "''")}' AND malformed = 1 ORDER BY row_number` },
  });
  if (dataset.partialDataset) observations.push({ ...base, id: `${dataset.snapshotId}:partial`, priority: 35,
    title: 'Coverage is partial or unverified', detail: 'These results describe the imported records. Additional pages, worksheets, or collection periods may exist.',
    question: 'Does the publisher offer a complete export or additional periods?',
  });
  let metadata: Record<string, unknown> = {};
  try { const parsed: unknown = JSON.parse(dataset.validationSummary); if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) metadata = parsed as Record<string, unknown>; } catch { /* Older snapshots may lack machine-readable diagnostics. */ }
  if (Array.isArray(metadata.warnings)) {
    const repairs = metadata.warnings.filter((warning): warning is string => typeof warning === 'string' && /repair|delimiter|UTF-16/i.test(warning));
    if (repairs.length) observations.push({ ...base, kind: 'quality', id: `${dataset.snapshotId}:repairs`, priority: 65,
      title: 'Import issues repaired automatically', detail: repairs.join(' '), question: 'Do the normalized columns match the publisher’s definitions?' });
  }
  return observations;
}

export function profileObservations(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary, profile: AutomaticProfile): DiscoveryObservation[] {
  const observations: DiscoveryObservation[] = [];
  const get = (id: string) => profile.results.find((entry) => entry.query.id === id);
  const quality = get('quality');
  if (quality?.result) {
    const missing = records(quality.result).filter((row) => (number(row.missing_rows) ?? 0) > 0 && (number(row.source_rows) ?? 0) > 0)
      .sort((a, b) => Number(b.missing_rows) - Number(a.missing_rows)).slice(0, 2);
    for (const row of missing) {
      const percentage = 100 * Number(row.missing_rows) / Number(row.source_rows);
      observations.push({ id: `${dataset.snapshotId}:missing:${row.column_name}`, snapshotId: dataset.snapshotId, kind: 'quality', priority: percentage >= 20 ? 85 : 50,
        title: `${percentage.toFixed(1)}% missing in ${row.column_name}`, detail: `${Number(row.missing_rows).toLocaleString()} of ${Number(row.source_rows).toLocaleString()} imported rows have no value in this field. Missing is not zero.`,
        question: 'Is missingness caused by reporting coverage, suppression, or a source-format issue?', query: quality.query,
      });
    }
  }
  const coverage = get('coverage');
  if (coverage?.result?.rows.length) {
    const row = records(coverage.result)[0];
    const defaults = discoveryDefaults(schema, dataset.snapshotId);
    if (row.last_period != null) observations.push({ id: `${dataset.snapshotId}:latest`, snapshotId: dataset.snapshotId, kind: 'coverage', priority: 75,
      title: `Latest observed ${defaults.periodMode === 'year_code' ? 'year code' : 'period'}: ${display(row.last_period)}`,
      detail: `The ${defaults.period} field covers ${display(row.first_period)} through ${display(row.last_period)}. Imported ${dataset.retrievedAt.slice(0, 10)}. Year codes retain the publisher’s convention.`,
      question: 'Is there a newer release, or is this an intentionally historical dataset?', query: coverage.query,
    });
    if ((number(row.unusable_period_rows) ?? 0) > 0) observations.push({ id: `${dataset.snapshotId}:dates`, snapshotId: dataset.snapshotId, kind: 'quality', priority: 80,
      title: `${display(row.unusable_period_rows)} records have unusable dates`, detail: 'Missing or unrecognized periods are excluded from time comparisons. The automatic time-field choice may need review.',
      question: 'Does this source use another date field or a different period convention?', query: coverage.query,
    });
  }
  const groups = get('groups');
  if (groups?.result?.rows.length) {
    const row = records(groups.result)[0];
    if ((number(row.source_rows) ?? 0) >= 5) observations.push({ id: `${dataset.snapshotId}:group`, snapshotId: dataset.snapshotId, kind: 'pattern', priority: 70,
      title: `${display(row.group_value)} accounts for ${display(row.record_share_percent)}% of records`,
      detail: `${display(row.source_rows)} source rows across all imported periods. This describes record coverage, not a population rate or service outcome.`,
      question: 'Does this concentration reflect activity, a reporting boundary, or uneven collection?', query: groups.query,
    });
  }
  const changes = get('changes');
  if (changes?.result) {
    const rows = records(changes.result);
    const change = rows.find((row) => Math.abs(number(row.change) ?? 0) >= 20 && Math.abs(number(row.change_percent) ?? 0) >= 25);
    if (change) observations.push({ id: `${dataset.snapshotId}:change`, snapshotId: dataset.snapshotId, kind: 'pattern', priority: 88,
      title: `Recorded rows changed ${Number(change.change_percent).toFixed(1)}% for ${display(change.group_value)}`,
      detail: `${display(change.previous_value)} → ${display(change.observed_value)} records, ${display(change.previous_period)} → ${display(change.discovery_period)}. Changes in collection and incomplete periods can explain this.`,
      question: 'Did activity change, or did the source’s coverage, definitions, or reporting cadence change?', query: changes.query,
    });
    if (rows.some((row) => row.comparison_status === 'Missing intervening periods')) observations.push({ id: `${dataset.snapshotId}:gaps`, snapshotId: dataset.snapshotId, kind: 'quality', priority: 82,
      title: 'The recorded time series has gaps', detail: 'At least one group skips an intervening period. Missing periods are not treated as zero and changes across gaps are withheld.',
      question: 'Can earlier exports fill these periods without duplicating existing records?', query: changes.query,
    });
  }
  return observations;
}

export function rankObservations(observations: DiscoveryObservation[], limit = 8): DiscoveryObservation[] {
  // Give several datasets a voice before adding a second observation about the same one.
  const sorted = [...observations].sort((a, b) => b.priority - a.priority || a.id.localeCompare(b.id));
  const selected: DiscoveryObservation[] = [];
  for (const perSource of [1, 2, 3]) for (const item of sorted) {
    if (selected.length >= limit) return selected;
    if (!selected.some((prior) => prior.id === item.id) && selected.filter((prior) => prior.snapshotId === item.snapshotId).length < perSource) selected.push(item);
  }
  return selected;
}
