import type { AnalyticalDatasetSummary, AnalyticalSchema, ServiceLeadInput, SqlQueryResult } from '../types';
import { buildDiscoveryPlan, discoveryDefaults, quoteIdentifier, type DiscoveryQuery, type DiscoverySelection } from './dataDiscovery';
import { abatementCompanion, abatementLeads, abatementQueries, reportsAroundAbatements } from './abatementLeads';
import { belongingsLeads, belongingsQueries } from './belongingsLeads';
import { stopCompanions, stopLeads, stopQueries } from './stopLeads';
import { chargeLeads, chargeQueries } from './chargeLeads';
import { disclosureLeads, disclosureQueries } from './disclosureLeads';
import { capacityCompanions, capacityLeads, capacityQueries } from './capacityLeads';
import { spendingCompanion, spendingLeads, spendingQueries } from './spendingLeads';
import { backlogChangeLeads, budgetTable, recurringServiceLeads, serviceAccountabilityLeads, serviceAccountabilityQueries, serviceCompanions, serviceRequestRoles, type LeadScope } from './serviceAccountability';

export interface DiscoveryObservation {
  id: string;
  snapshotId: string;
  /** A `lead` says something about the subject of the records; the other kinds describe the table itself. */
  kind: 'lead' | 'coverage' | 'quality' | 'pattern';
  priority: number;
  title: string;
  detail: string;
  question: string;
  query?: DiscoveryQuery;
  selection?: DiscoverySelection;
  /** Set on a lead: the published records it was counted from. */
  family?: ServiceLeadInput['family'];
  /** Set on a lead built from other tables' results as well: the tables its query does not name. */
  reads?: string[];
  /** Set on a lead whose result names a period or places a follow-up record should cover. */
  scope?: LeadScope;
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
  // A check that reads other tables is stale once those tables change, not only this one.
  const companions = serviceCompanions(schema, dataset);
  const capacity = capacityCompanions(schema, dataset);
  const columns = schema.columns.filter((column) => column.tableName === dataset.tableName);
  return JSON.stringify([dataset, columns, companions ? [companions.earlier, ...companions.outcomes].map((table) => table.snapshotId) : [],
    serviceRequestRoles(columns)?.group ? budgetTable(schema)?.snapshotId ?? null : null,
    Object.values(stopCompanions(schema, dataset) ?? {}).map((table) => table.snapshotId), abatementCompanion(schema, dataset)?.snapshotId ?? null, reportsAroundAbatements(schema, dataset)?.tables.map((table) => table.snapshotId) ?? null,
    capacity ? [capacity.budget, capacity.reports.open, ...capacity.reports.closed].map((table) => table.snapshotId) : [],
    spendingCompanion(schema, dataset)?.snapshotId ?? null]);
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
  return [...plan.queries.filter((query) => (query.id === 'coverage' && selection.period) || (query.id === 'groups' && selection.dimension) || (query.id === 'changes' && selection.period)),
    ...serviceAccountabilityQueries(schema, dataset), ...stopQueries(schema, dataset), ...abatementQueries(schema, dataset), ...belongingsQueries(schema, dataset), ...disclosureQueries(schema, dataset), ...chargeQueries(schema, dataset), ...capacityQueries(schema, dataset),
    ...spendingQueries(schema, dataset)];
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
  const lead = (family: ServiceLeadInput['family']) => (item: ReturnType<typeof stopLeads>[number]): DiscoveryObservation => ({
    id: `${dataset.snapshotId}:${item.key}`, snapshotId: dataset.snapshotId, kind: 'lead', family, priority: item.priority,
    title: item.title, detail: item.detail, question: item.question, query: item.query, scope: item.scope,
  });
  const observations: DiscoveryObservation[] = [...serviceAccountabilityLeads(profile.results).map(lead('SERVICE_REPORTS')), ...stopLeads(profile.results).map(lead('POLICE_STOPS')),
    ...abatementLeads(profile.results).map(lead('ABATEMENT_LOGS')), ...belongingsLeads(profile.results).map(lead('ABATEMENT_LOGS')), ...chargeLeads(profile.results).map(lead('CHARGE_LOGS')), ...disclosureLeads(profile.results).map(lead('DISCLOSURE_FILINGS')),
    ...capacityLeads(profile.results).map(lead('CAPACITY_COMPARISON')), ...spendingLeads(profile.results).map(lead('SPENDING_RECORDS'))];
  const get = (id: string) => profile.results.find((entry) => entry.query.id === id);
  // A column's missing-value rate describes the source table's own completeness, not the subject
  // of the investigation; it stays on the import/quality screen rather than "what stands out".
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
    // A change in RECORD COUNT between periods is at least as often caused by reporting cadence,
    // collection changes, or an incomplete period as by the subject's own activity changing; its
    // own detail text admits as much. It is a state of the source table, not a finding.
    if (rows.some((row) => row.comparison_status === 'Missing intervening periods')) observations.push({ id: `${dataset.snapshotId}:gaps`, snapshotId: dataset.snapshotId, kind: 'quality', priority: 82,
      title: 'The recorded time series has gaps', detail: 'At least one group skips an intervening period. Missing periods are not treated as zero and changes across gaps are withheld.',
      question: 'Can earlier exports fill these periods without duplicating existing records?', query: changes.query,
    });
  }
  return observations;
}

/** Leads that only appear when several imported tables are read together. */
export function recurringObservations(schema: AnalyticalSchema, profiles: Record<string, AutomaticProfile>): DiscoveryObservation[] {
  const tables = schema.datasets.flatMap((dataset) => profiles[dataset.snapshotId] ? [{ snapshotId: dataset.snapshotId, displayName: dataset.displayName, sourceUri: dataset.sourceUri, retrievedAt: dataset.retrievedAt, results: profiles[dataset.snapshotId].results }] : []);
  return [...backlogChangeLeads(tables), ...recurringServiceLeads(tables)].map((lead) => ({
    id: `${lead.snapshotId}:${lead.key}`, snapshotId: lead.snapshotId, kind: 'lead', family: 'SERVICE_REPORTS', priority: lead.priority,
    title: lead.title, detail: lead.detail, question: lead.question, query: lead.query, reads: lead.reads,
  }));
}

/** Every lead to store as a finding. `complete` only when each imported table has been checked
 * without error: a lead missing from an unfinished pass has not stopped holding. */
export function storedLeads(schema: AnalyticalSchema, profiles: Record<string, AutomaticProfile>): { leads: ServiceLeadInput[]; complete: boolean } {
  const observations = [...schema.datasets.flatMap((dataset) => profiles[dataset.snapshotId] ? profileObservations(schema, dataset, profiles[dataset.snapshotId]) : []),
    ...recurringObservations(schema, profiles)].filter((item) => item.kind === 'lead' && item.query && item.family);
  // A lead cites its own table first, then every other imported table its SQL names or its
  // figures were otherwise read from.
  const sources = (item: DiscoveryObservation) => [item.snapshotId, ...schema.datasets.filter((dataset) => dataset.snapshotId !== item.snapshotId
    && (item.reads?.includes(dataset.snapshotId) || item.query!.sql.includes(quoteIdentifier(dataset.tableName)))).map((dataset) => dataset.snapshotId)];
  return {
    leads: observations.map((item) => ({ family: item.family!, key: item.id, snapshotId: item.snapshotId, snapshotIds: sources(item), title: item.title, detail: item.detail, question: item.question, method: item.query!.method, sql: item.query!.sql })),
    complete: schema.datasets.every((dataset) => profiles[dataset.snapshotId]?.results.every((entry) => !entry.error)),
  };
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
