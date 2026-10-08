import type { AnalyticalColumnSummary, AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { formatDate } from '../components/charts/core/format';

export interface DiscoveryFilter { column: string; operator: 'equals' | 'contains' | 'missing'; value: string }
export interface DiscoverySelection {
  snapshotId: string;
  measure: string;
  aggregation: 'count' | 'sum' | 'mean';
  dimension: string;
  period: string;
  periodMode: 'year_code' | 'year' | 'month' | 'day';
  filters: DiscoveryFilter[];
}
export interface DiscoveryQuery { id: string; title: string; method: string; sql: string }
export interface DiscoveryPlan {
  dataset: AnalyticalDatasetSummary;
  selection: DiscoverySelection;
  warnings: string[];
  queries: DiscoveryQuery[];
  recordsSql: string;
}
export interface DiscoveryResult { query: DiscoveryQuery; result?: SqlQueryResult; error?: string }
export interface DiscoveryReport { version: 1; createdAt: string; plan: DiscoveryPlan; results: DiscoveryResult[] }

export const quoteIdentifier = (value: string) => `"${value.replace(/"/g, '""')}"`;
const literal = (value: string) => `'${value.replace(/'/g, "''")}'`;
export const isNumericColumn = (column: AnalyticalColumnSummary) => /^(INTEGER|NUMBER|DECIMAL|FLOAT)$/.test(column.logicalType);

export function discoveryDefaults(schema: AnalyticalSchema, snapshotId: string): DiscoverySelection {
  const dataset = schema.datasets.find((item) => item.snapshotId === snapshotId);
  const columns = schema.columns.filter((item) => item.tableName === dataset?.tableName);
  const period = columns.find((item) => /^(report_fy|fiscal_year|year|date|observed|observation_date|report_date)$/.test(item.columnName))
    ?? columns.find((item) => /^(DATE|TIMESTAMP)$/.test(item.logicalType));
  return { snapshotId, measure: '', aggregation: 'count', dimension: '', period: period?.columnName ?? '',
    periodMode: period && /^(report_fy|fiscal_year|year)$/.test(period.columnName) ? 'year_code' : 'month', filters: [] };
}

export function buildDiscoveryPlan(schema: AnalyticalSchema, selection: DiscoverySelection): DiscoveryPlan {
  return planDiscovery(schema, selection, null);
}

/** Marks the series query's SQL; nothing else in a plan orders this way. */
export const SERIES_ORDER = 'ORDER BY group_value NULLS LAST, discovery_period';

/**
 * Every period of the named groups, from the same scope as the 'changes' check: drawn when that
 * check's top 100 rows (ordered by change) do not hold every group × period. The groups filter sits
 * before LAG, which is partitioned by group, so the comparison statuses are unchanged by it.
 */
export function discoverySeriesSql(schema: AnalyticalSchema, selection: DiscoverySelection, groups: readonly unknown[]): string | null {
  return planDiscovery(schema, selection, groups).queries.find((query) => query.id === 'changes')?.sql ?? null;
}

function planDiscovery(schema: AnalyticalSchema, selection: DiscoverySelection, seriesGroups: readonly unknown[] | null): DiscoveryPlan {
  const dataset = schema.datasets.find((item) => item.snapshotId === selection.snapshotId);
  if (!dataset) throw new Error('Choose an imported snapshot.');
  const columns = schema.columns.filter((item) => item.tableName === dataset.tableName);
  const column = (name: string) => {
    const found = columns.find((item) => item.columnName === name);
    if (!found) throw new Error(`Column ${name} is not in this snapshot.`);
    return quoteIdentifier(found.columnName);
  };
  if (!['count', 'sum', 'mean'].includes(selection.aggregation)) throw new Error('Choose count, sum, or mean.');
  if (!['year_code', 'year', 'month', 'day'].includes(selection.periodMode)) throw new Error('Choose a period format.');
  if (selection.aggregation !== 'count' && !selection.measure) throw new Error('Choose a numeric measure.');
  if (selection.measure && !columns.some((item) => item.columnName === selection.measure && isNumericColumn(item))) {
    throw new Error('The measure must be numeric. Codes and suppression markers are preserved as text.');
  }
  if (selection.filters.length > 8) throw new Error('Use at most eight filters.');
  const predicates = selection.filters.map((filter) => {
    const field = column(filter.column);
    if (filter.operator === 'missing') return `${field} IS NULL`;
    if (filter.operator === 'equals') return `CAST(${field} AS VARCHAR) = ${literal(filter.value)}`;
    if (filter.operator === 'contains') return `CONTAINS(LOWER(CAST(${field} AS VARCHAR)), LOWER(${literal(filter.value)}))`;
    throw new Error('Unknown filter operator.');
  });
  const from = `FROM ${quoteIdentifier(dataset.tableName)}${predicates.length ? ` WHERE ${predicates.join(' AND ')}` : ''}`;
  const measure = selection.measure ? quoteIdentifier(`source: ${selection.measure}`) : null;
  const group = selection.dimension ? `CAST(${column(selection.dimension)} AS VARCHAR)` : "'All rows'";
  let period = 'NULL';
  if (selection.period) {
    const field = column(selection.period);
    period = selection.periodMode === 'year_code'
      ? `CASE WHEN REGEXP_FULL_MATCH(TRIM(CAST(${field} AS VARCHAR)), '[0-9]{2}|[0-9]{4}') THEN TRY_CAST(${field} AS INTEGER) END`
      : `CAST(DATE_TRUNC('${selection.periodMode}', TRY_CAST(${field} AS DATE)) AS DATE)`;
  }
  // Namespace original fields so a source column named q1, peer_values, or discovery_group
  // cannot silently replace one of the computed analysis fields.
  const base = `SELECT ${columns.map((item) => `${column(item.columnName)} AS ${quoteIdentifier(`source: ${item.columnName}`)}`).join(', ')}, ${group} AS discovery_group, ${period} AS discovery_period ${from}`;
  // Never infer that a numeric column is additive. Counting source records is the default.
  const value = selection.aggregation === 'count' ? 'COUNT(*)' : `${selection.aggregation === 'sum' ? 'SUM' : 'AVG'}(${measure})`;
  const missing = selection.aggregation === 'count' ? '0' : `COUNT(*) - COUNT(${measure})`;
  const warnings = [
    'Counts describe source records, not necessarily unique people, projects, or events. Repeated totals and overlapping geographies require review.',
    'Comparisons use one selected snapshot and the same filters. Source definitions, units, and collection methods still determine whether a comparison is meaningful.',
  ];
  if (dataset.partialDataset) warnings.push('This snapshot is partial or its completeness has not been established. Results describe only the imported records.');
  if (dataset.rejectedRows) warnings.push(`${dataset.rejectedRows.toLocaleString()} rejected source rows are excluded from calculations.`);
  const unconverted = [...new Set([selection.measure, selection.dimension, selection.period].filter((name): name is string => Boolean(name)))]
    .reduce((total, name) => total + (dataset.unconvertedByColumn[name] ?? 0), 0);
  if (unconverted) warnings.push(`${unconverted.toLocaleString()} source cells in the selected columns did not convert to their column type. They are missing here but not empty in the source; review them in the row ledger.`);
  if (selection.aggregation === 'sum') warnings.push('Sum adds reported values. Filter to one budget phase and compatible account class, unit, and population; exclude overlapping totals before interpreting it.');
  if (selection.aggregation === 'mean') warnings.push('Mean gives every nonmissing source row equal weight. It is not a population-weighted rate.');
  if (selection.periodMode === 'year_code' && selection.period) warnings.push('Year codes retain their source convention. Two-digit fiscal codes are not expanded to calendar years; do not mix two- and four-digit encodings.');
  if (columns.length > 64) warnings.push(`Column quality profiles cover the first 64 of ${columns.length} columns. Other analyses use all rows and the selected fields.`);
  const queries: DiscoveryQuery[] = [{
    id: 'coverage', title: 'Scope and missing data',
    method: 'Full filtered snapshot. Missing and unparseable periods remain in the coverage count but are excluded from time comparisons.',
    sql: `WITH analysis_scope AS (${base}) SELECT COUNT(*) AS source_rows${measure ? `, COUNT(${measure}) AS reported_values, COUNT(*) - COUNT(${measure}) AS missing_values, MIN(${measure}) AS minimum, MEDIAN(${measure}) AS median, MAX(${measure}) AS maximum` : ''}${selection.period ? ', COUNT(*) - COUNT(discovery_period) AS unusable_period_rows, MIN(discovery_period) AS first_period, MAX(discovery_period) AS last_period' : ''} FROM analysis_scope`,
  }, {
    id: 'quality', title: 'Column quality',
    method: 'Null counts, distinct reported values, and range over every filtered row. Text suppression markers remain distinct values; a nonmissing cell is not necessarily a usable measurement.',
    sql: columns.slice(0, 64).map((item) => `SELECT ${literal(item.columnName)} AS column_name, ${literal(item.logicalType)} AS logical_type, COUNT(*) AS source_rows, COUNT(*) - COUNT(${column(item.columnName)}) AS missing_rows, COUNT(DISTINCT ${column(item.columnName)}) AS distinct_values, CAST(MIN(${column(item.columnName)}) AS VARCHAR) AS minimum, CAST(MAX(${column(item.columnName)}) AS VARCHAR) AS maximum ${from}`).join('\nUNION ALL\n'),
  }];
  queries.push({
    id: 'groups', title: selection.dimension ? `Differences by ${selection.dimension}` : 'Selected measure',
    method: `${selection.aggregation === 'count' ? 'Count of records' : selection.aggregation === 'sum' ? 'Sum of reported values' : 'Unweighted mean of reported values'} per group across all selected periods. Record share uses ALL filtered rows as denominator, including missing groups. Missing measurements are shown separately; observed values may be incomplete.`,
    sql: `WITH analysis_scope AS (${base}), analysis_groups AS (
  SELECT discovery_group AS group_value, COUNT(*) AS source_rows, ${missing} AS missing_values, ${value} AS observed_value
  FROM analysis_scope GROUP BY discovery_group
) SELECT *, ROUND(100.0 * source_rows / NULLIF(SUM(source_rows) OVER (), 0), 2) AS record_share_percent
FROM analysis_groups ORDER BY ABS(observed_value) DESC NULLS LAST, group_value NULLS LAST`,
  });
  if (selection.period) {
    const named = (seriesGroups ?? []).filter((group) => group !== null && group !== undefined).map((group) => literal(String(group)));
    // IN never matches NULL, so a blank group is named separately.
    const blank = (seriesGroups ?? []).some((group) => group === null || group === undefined);
    const groupFilter = seriesGroups
      ? ` AND (${[named.length ? `discovery_group IN (${named.join(', ')})` : '', blank ? 'discovery_group IS NULL' : ''].filter(Boolean).join(' OR ') || 'FALSE'})`
      : '';
    const step = selection.periodMode === 'year_code' ? 'discovery_period - previous_period' : `DATE_DIFF('${selection.periodMode}', previous_period, discovery_period)`;
    queries.push({
      id: 'changes', title: 'Largest changes over time',
      method: 'Adjacent observed periods within the same group. Missing intervening periods and missing measurements suppress differences. A zero baseline suppresses percentage change. Partial period coverage and changing populations require review.',
      sql: `WITH analysis_scope AS (${base}), analysis_periods AS (
 SELECT discovery_group AS group_value, discovery_period, COUNT(*) AS source_rows, ${missing} AS missing_values, ${value} AS observed_value
 FROM analysis_scope WHERE discovery_period IS NOT NULL${groupFilter} GROUP BY discovery_group, discovery_period
), analysis_previous AS (
 SELECT *, LAG(discovery_period) OVER w AS previous_period, LAG(observed_value) OVER w AS previous_value,
 LAG(source_rows) OVER w AS previous_rows, LAG(missing_values) OVER w AS previous_missing_values
 FROM analysis_periods WINDOW w AS (PARTITION BY group_value ORDER BY discovery_period)
), analysis_changes AS (
 SELECT *, CASE WHEN previous_period IS NULL THEN 'No baseline' WHEN ${step} <> 1 THEN 'Missing intervening periods'
 WHEN missing_values > 0 OR previous_missing_values > 0 THEN 'Incomplete measurements' ELSE 'Adjacent periods with reported values' END AS comparison_status
 FROM analysis_previous
)
SELECT *, CASE WHEN comparison_status = 'Adjacent periods with reported values' THEN observed_value - previous_value END AS change,
 CASE WHEN comparison_status = 'Adjacent periods with reported values' THEN ROUND(100.0 * (observed_value - previous_value) / NULLIF(ABS(previous_value), 0), 2) END AS change_percent
FROM analysis_changes ${seriesGroups ? SERIES_ORDER : 'ORDER BY ABS(change) DESC NULLS LAST, group_value NULLS LAST, discovery_period'}`,
    });
  }
  if (measure) queries.push({
    id: 'outliers', title: 'Unusual source values',
    method: 'Values outside Q1 − 1.5 × IQR or Q3 + 1.5 × IQR within the selected group and period. At least eight reported values and a positive IQR are required. These are review leads, not evidence of error or wrongdoing. Quartiles use observed values (discrete quantiles).',
    sql: `WITH analysis_scope AS (${base}), analysis_distribution AS (
 SELECT *, COUNT(${measure}) OVER w AS peer_values, QUANTILE_DISC(${measure}, 0.25) OVER w AS q1,
 QUANTILE_DISC(${measure}, 0.75) OVER w AS q3
 FROM analysis_scope WINDOW w AS (PARTITION BY discovery_group, discovery_period)
), analysis_unusual AS (
 SELECT *, q1 - 1.5 * (q3 - q1) AS lower_fence, q3 + 1.5 * (q3 - q1) AS upper_fence
 FROM analysis_distribution WHERE peer_values >= 8 AND q3 > q1${selection.period ? ' AND discovery_period IS NOT NULL' : ''}
)
SELECT *, COUNT(*) OVER () AS total_flagged_rows FROM analysis_unusual
WHERE ${measure} < lower_fence OR ${measure} > upper_fence
ORDER BY ABS(${measure}) DESC`,
  });
  queries.push({
    id: 'duplicates', title: 'Repeated source rows',
    method: 'Exact row-hash repeats across the entire snapshot, including rows outside the selected filters. They are not removed automatically: repeated records can be legitimate. Original row numbers and source cells are retained for review.',
    sql: `SELECT row_hash, COUNT(*) AS occurrences, MIN(row_number) AS first_source_row, MAX(row_number) AS last_source_row,
 MIN(source_payload_json) AS source_cells FROM analytical_rows
WHERE snapshot_id = ${literal(dataset.snapshotId)} AND malformed = 0
GROUP BY row_hash HAVING COUNT(*) > 1 ORDER BY occurrences DESC, first_source_row`,
  });
  return { dataset: { ...dataset }, selection: structuredClone(selection), warnings, queries, recordsSql: `SELECT * ${from}` };
}

export function discoveryDrilldown(plan: DiscoveryPlan, groupValue: unknown): string {
  const field = plan.selection.dimension;
  if (!field) return plan.recordsSql;
  return `SELECT * FROM (${plan.recordsSql}) AS analysis_records WHERE ${groupValue === null ? `${quoteIdentifier(field)} IS NULL` : `CAST(${quoteIdentifier(field)} AS VARCHAR) = ${literal(String(groupValue))}`}`;
}

export function discoveryFindings(report: DiscoveryReport): string[] {
  const coverage = report.results.find((item) => item.query.id === 'coverage')?.result;
  if (!coverage?.rows.length) return [];
  const cell = (name: string) => coverage.rows[0][coverage.columns.indexOf(name)];
  const findings = [`${String(cell('source_rows'))} records match this analysis.`];
  if (Number(cell('missing_values')) > 0) findings.push(`${String(cell('missing_values'))} records have no value for ${report.plan.selection.measure}.`);
  if (Number(cell('unusable_period_rows')) > 0) findings.push(`${String(cell('unusable_period_rows'))} records cannot be placed in the selected time series.`);
  if (cell('last_period') != null) findings.push(`Latest observed period in this scope: ${String(cell('last_period'))}. Import time does not establish data freshness.`);
  const changes = report.results.find((item) => item.query.id === 'changes')?.result;
  if (changes) {
    const gapIndex = changes.columns.indexOf('comparison_status');
    const gaps = changes.rows.filter((row) => row[gapIndex] === 'Missing intervening periods').length;
    if (gaps) findings.push(`${gaps}${changes.truncated ? ' or more' : ''} displayed time comparisons have gaps; their changes are withheld.`);
  }
  const outliers = report.results.find((item) => item.query.id === 'outliers')?.result;
  if (outliers?.rows.length) findings.push(`${String(outliers.rows[0][outliers.columns.indexOf('total_flagged_rows')])} source values fall outside their peer-group IQR fences. Review units and context before drawing conclusions.`);
  return findings;
}

export const SERIES_PANEL_LIMIT = 8;
const MISSING_INTERVENING = 'Missing intervening periods';
const ADJACENT = 'Adjacent periods with reported values';

export interface SeriesCell {
  observed: number | null;
  sourceRows: number | null;
  missingValues: number | null;
  status: string | null;
  change: number | null;
  changePercent: number | null;
}
export interface DiscoverySeriesSlot { key: string; label: string; title?: string; gap: boolean; position?: number }
export interface DiscoverySeriesGroup { key: string; value: unknown; label: string; cells: (SeriesCell | null)[] }
export interface DiscoverySeriesModel { slots: DiscoverySeriesSlot[]; groups: DiscoverySeriesGroup[]; otherGroups: number }

/** A numeric cell as a number. SUM of a DECIMAL can arrive as a string; anything else is absent. */
export const finiteCell = (value: unknown): number | null => {
  if (value === null || value === undefined || value === '') return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
};
export const groupLabel = (value: unknown) => (value === null || value === undefined ? '(blank in source)' : String(value));
export const groupKey = (value: unknown) => (value === null || value === undefined ? '\u0000blank' : `v:${String(value)}`);
const periodKey = (value: unknown) => String(value);

function isoParts(value: unknown): [number, number, number] | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(String(value));
  return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null;
}

/**
 * A period as a reader names it: a DATE period arrives as 'YYYY-MM-DD' (DATE_TRUNC to its unit),
 * so year mode reads '2024', month mode 'Jan 2024', day mode 'Jan 1, 2024'. A year code keeps its
 * source convention.
 */
export function formatPeriod(value: unknown, mode: DiscoverySelection['periodMode']): string {
  if (value === null || value === undefined) return '(no period)';
  if (mode === 'year_code') return String(value);
  const parts = isoParts(value);
  if (!parts) return String(value);
  return formatDate(Date.UTC(parts[0], parts[1] - 1, parts[2]), mode);
}

/** Periods from `a` to `b` in the plan's unit (as the 'changes' check counts them), or null. */
function periodStep(a: unknown, b: unknown, mode: DiscoverySelection['periodMode']): number | null {
  if (mode === 'year_code') {
    const [x, y] = [finiteCell(a), finiteCell(b)];
    return x === null || y === null ? null : y - x;
  }
  const [x, y] = [isoParts(a), isoParts(b)];
  if (!x || !y) return null;
  if (mode === 'year') return y[0] - x[0];
  if (mode === 'month') return (y[0] - x[0]) * 12 + y[1] - x[1];
  return Math.round((Date.UTC(y[0], y[1] - 1, y[2]) - Date.UTC(x[0], x[1] - 1, x[2])) / 86_400_000);
}

/**
 * One slot per period any plotted group observed, in period order. Where a group skips periods
 * that no other group fills ('Missing intervening periods' with nothing between), a gap slot is
 * inserted, so the line breaks there instead of joining the two periods. Groups follow
 * `groupOrder` (the 'groups' check's order) and the first SERIES_PANEL_LIMIT are plotted.
 */
export function discoverySeriesModel(result: SqlQueryResult, groupOrder: readonly unknown[] | null, periodMode: DiscoverySelection['periodMode']): DiscoverySeriesModel {
  const at = (name: string) => result.columns.indexOf(name);
  const [g, p, prev, status] = [at('group_value'), at('discovery_period'), at('previous_period'), at('comparison_status')];
  const groups = new Map<string, unknown>();
  for (const row of result.rows) if (!groups.has(groupKey(row[g]))) groups.set(groupKey(row[g]), row[g]);
  const named = new Set((groupOrder ?? []).map(groupKey));
  const ordered = [
    ...(groupOrder ?? []).filter((value) => groups.has(groupKey(value))),
    ...[...groups.values()].filter((value) => !named.has(groupKey(value))),
  ];
  const plotted = ordered.slice(0, SERIES_PANEL_LIMIT);
  const plottedKeys = new Set(plotted.map(groupKey));
  const rows = result.rows.filter((row) => plottedKeys.has(groupKey(row[g])));

  const numeric = rows.every((row) => finiteCell(row[p]) !== null);
  const compare = (a: unknown, b: unknown) => (numeric ? Number(a) - Number(b) : String(a).localeCompare(String(b)));
  const periods = [...new Map(rows.map((row) => [periodKey(row[p]), row[p]])).values()].sort(compare);
  const gapsAfter = new Map<string, { from: unknown; to: unknown }>();
  periods.forEach((period, i) => {
    const next = periods[i + 1];
    const step = next === undefined ? null : periodStep(period, next, periodMode);
    if (step !== null && step > 1) gapsAfter.set(periodKey(period), { from: period, to: next });
  });
  // Periods the step cannot be read from fall back to the query's own verdict.
  for (const row of rows) {
    if (row[status] !== MISSING_INTERVENING || row[prev] === null || row[prev] === undefined) continue;
    if (periodStep(row[prev], row[p], periodMode) !== null) continue;
    const between = periods.some((period) => compare(period, row[prev]) > 0 && compare(period, row[p]) < 0);
    if (!between) gapsAfter.set(periodKey(row[prev]), { from: row[prev], to: row[p] });
  }
  // Where every period's distance from the first can be counted, x follows it: a five-period hole
  // is five periods wide, not one slot.
  const offsets = periods.map((period) => periodStep(periods[0], period, periodMode));
  const positioned = offsets.every((offset) => offset !== null);
  const slots: DiscoverySeriesSlot[] = [];
  periods.forEach((period, i) => {
    const position = positioned ? offsets[i] ?? undefined : undefined;
    slots.push({ key: periodKey(period), label: formatPeriod(period, periodMode), gap: false, position });
    const gap = gapsAfter.get(periodKey(period));
    if (!gap) return;
    const next = positioned ? periodStep(periods[0], gap.to, periodMode) : null;
    slots.push({
      key: `gap:${periodKey(gap.from)}|${periodKey(gap.to)}`,
      label: '',
      title: `no rows between ${formatPeriod(gap.from, periodMode)} and ${formatPeriod(gap.to, periodMode)}`,
      gap: true,
      position: position !== undefined && next !== null ? (position + next) / 2 : undefined,
    });
  });
  const slotIndex = new Map(slots.map((slot, index) => [slot.key, index]));
  const cell = (row: unknown[]): SeriesCell => ({
    observed: finiteCell(row[at('observed_value')]),
    sourceRows: finiteCell(row[at('source_rows')]),
    missingValues: finiteCell(row[at('missing_values')]),
    status: row[status] === null || row[status] === undefined ? null : String(row[status]),
    change: finiteCell(row[at('change')]),
    changePercent: finiteCell(row[at('change_percent')]),
  });
  return {
    slots,
    groups: plotted.map((value) => {
      const cells: (SeriesCell | null)[] = slots.map(() => null);
      for (const row of rows) {
        const index = slotIndex.get(periodKey(row[p]));
        if (groupKey(row[g]) === groupKey(value) && index !== undefined) cells[index] = cell(row);
      }
      return { key: groupKey(value), value, label: groupLabel(value), cells };
    }),
    otherGroups: ordered.length - plotted.length,
  };
}

/** The largest adjacent change across every group: the 'changes' rows are ordered by |change|. */
export function largestDiscoveryChange(changes: SqlQueryResult): { group: unknown; from: unknown; to: unknown; change: number; changePercent: number | null } | null {
  const at = (name: string) => changes.columns.indexOf(name);
  const row = changes.rows.find((candidate) => candidate[at('comparison_status')] === ADJACENT && finiteCell(candidate[at('change')]) !== null);
  if (!row) return null;
  return { group: row[at('group_value')], from: row[at('previous_period')], to: row[at('discovery_period')], change: finiteCell(row[at('change')]) ?? 0, changePercent: finiteCell(row[at('change_percent')]) };
}

/** Groups with any period whose predecessor is not adjacent: their changes are withheld there. */
export function groupsWithMissingPeriods(changes: SqlQueryResult): number {
  const at = (name: string) => changes.columns.indexOf(name);
  return new Set(changes.rows.filter((row) => row[at('comparison_status')] === MISSING_INTERVENING).map((row) => groupKey(row[at('group_value')]))).size;
}
