import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { AnalyticalSchema, SqlQueryResult } from '../../types';
import { buildDiscoveryPlan, discoveryDefaults, discoveryDrilldown, discoveryFindings, discoverySeriesModel, discoverySeriesSql, finiteCell,
  groupKey, groupLabel, groupsWithMissingPeriods, isNumericColumn, largestDiscoveryChange, SERIES_PANEL_LIMIT,
  type DiscoveryFilter, type DiscoveryPlan, type DiscoveryReport, type DiscoverySelection } from '../../utils/dataDiscovery';
import { useDiscoveryController } from './AutomaticDiscoveryProvider';
import { downloadFile } from './downloadFile';
import BarTable, { type BarTableRow } from '../charts/figures/BarTable';
import SeriesSmallMultiples, { type SeriesLine } from '../charts/figures/SeriesSmallMultiples';
import { formatValue } from '../charts/core/format';
import { absent, measured } from '../charts/core/nulls';
import type { ChartSources } from '../charts/frame/SourceCaption';

export default function DataDiscovery({ schema, onInspect }: { schema: AnalyticalSchema; onInspect: (sql: string) => void }) {
  const automatic = useDiscoveryController();
  const [selection, setSelection] = useState<DiscoverySelection>(() => discoveryDefaults(schema, schema.datasets[0]?.snapshotId ?? ''));
  const [report, setReport] = useState<DiscoveryReport | null>(null);
  const [progress, setProgress] = useState('');
  const [error, setError] = useState('');
  const [running, setRunning] = useState(false);
  const cancelled = useRef(false);
  const runVersion = useRef(0);
  useEffect(() => () => { runVersion.current += 1; cancelled.current = true; }, []);
  const dataset = schema.datasets.find((item) => item.snapshotId === selection.snapshotId);
  const columns = schema.columns.filter((item) => item.tableName === dataset?.tableName);
  const patch = (value: Partial<DiscoverySelection>) => setSelection((current) => ({ ...current, ...value }));
  const stale = report && JSON.stringify(report.plan.selection) !== JSON.stringify(selection);
  const [series, setSeries] = useState<SeriesState | null>(null);
  const seriesGate = useLatestRequestGate();

  // The 'changes' check keeps only its top 100 rows by change, so a truncated result cannot draw
  // whole series. One more query reads every period of the plotted groups. It resolves after the
  // run has finished, so a later run (or leaving) must discard it: the gate is its identity.
  const loadSeries = async (finished: DiscoveryReport) => {
    const changes = finished.results.find((item) => item.query.id === 'changes')?.result;
    const groups = finished.results.find((item) => item.query.id === 'groups')?.result;
    if (!changes?.truncated || !groups) return;
    const order = groups.rows.map((row) => row[groups.columns.indexOf('group_value')]).slice(0, SERIES_PANEL_LIMIT);
    const token = seriesGate.begin();
    setSeries({ status: 'loading' });
    const release = automatic?.hold();
    try {
      const sql = discoverySeriesSql(schema, finished.plan.selection, order);
      if (!sql) throw new Error('This snapshot has no time field to draw.');
      const result = await invokeCommand('run_analytical_sql', { sql, limit: SERIES_ROW_LIMIT });
      if (!seriesGate.isCurrent(token)) return;
      setSeries({ status: 'ready', result });
    } catch (reason) {
      if (!seriesGate.isCurrent(token)) return;
      setSeries({ status: 'error', message: String(reason) });
    } finally {
      release?.();
    }
  };

  const analyze = async () => {
    seriesGate.invalidate();
    setSeries(null);
    setError('');
    cancelled.current = false;
    const version = ++runVersion.current;
    const release = automatic?.hold();
    try {
      const plan = buildDiscoveryPlan(schema, selection);
      setRunning(true);
      const next: DiscoveryReport = { version: 1, createdAt: new Date().toISOString(), plan, results: [] };
      setReport({ ...next });
      // Queries share the repository's single connection. Sequential execution also lets the
      // reader stop between scans and keeps one failed analysis from hiding other evidence.
      for (const query of plan.queries) {
        if (cancelled.current || version !== runVersion.current) break;
        setProgress(`${next.results.length + 1}/${plan.queries.length}: ${query.title}`);
        try {
          const result = await invokeCommand('run_analytical_sql', { sql: query.sql, limit: 100 });
          next.results = [...next.results, { query, result }];
        } catch (reason) {
          next.results = [...next.results, { query, error: String(reason) }];
        }
        if (version === runVersion.current) setReport({ ...next });
      }
      if (version === runVersion.current) setProgress(cancelled.current ? 'Stopped after the current query. The report contains completed checks only.' : 'Analysis complete.');
      if (version === runVersion.current && !cancelled.current) void loadSeries(next);
    } catch (reason) {
      if (version === runVersion.current) setError(String(reason));
    } finally {
      release?.();
      if (version === runVersion.current) setRunning(false);
    }
  };
  const exportReport = () => {
    if (!report) return;
    downloadFile(`onrecord-discovery-${report.plan.dataset.snapshotId.slice(0, 16)}.json`, JSON.stringify({ ...report, findings: discoveryFindings(report) }, null, 2), 'application/json');
  };
  const changesShown = Boolean(report?.results.some((item) => item.query.id === 'changes' && item.result));
  return <div className="space-y-3 text-label leading-4 text-fg-secondary">
    <p className="leading-relaxed text-muted">Explore coverage, group differences, changes over time, unusual values, and repeated records. Each check runs across the imported snapshot and retains its query for review.</p>
    {!schema.datasets.length && <p>Import an acquired table to begin discovery. Document excerpts are searchable below.</p>}
    <fieldset disabled={running} className="space-y-2 disabled:opacity-70">
      <label className="block">Discovery snapshot<select className="analysis-input mt-1 w-full" aria-label="Discovery snapshot" value={selection.snapshotId} onChange={(event) => setSelection(discoveryDefaults(schema, event.target.value))}>
        <option value="">Choose an imported snapshot</option>
        {schema.datasets.map((item) => <option key={item.snapshotId} value={item.snapshotId}>{item.displayName} · {item.retrievedAt} · {item.snapshotId.slice(0, 8)}</option>)}
      </select></label>
      <div className="grid gap-2 sm:grid-cols-2">
        <label>Measure<select className="analysis-input mt-1 w-full" aria-label="Discovery measure" value={selection.measure} onChange={(event) => patch({ measure: event.target.value, ...(!event.target.value ? { aggregation: 'count' as const } : {}) })}>
          <option value="">Source record count</option>{columns.filter(isNumericColumn).map((item) => <option key={item.columnName}>{item.columnName}</option>)}
        </select></label>
        <label>Calculation<select className="analysis-input mt-1 w-full" aria-label="Discovery calculation" value={selection.aggregation} onChange={(event) => patch({ aggregation: event.target.value as DiscoverySelection['aggregation'] })}>
          <option value="count">Count records</option><option value="sum" disabled={!selection.measure}>Sum reported values</option><option value="mean" disabled={!selection.measure}>Mean reported values</option>
        </select></label>
        <label>Compare groups<select className="analysis-input mt-1 w-full" aria-label="Discovery groups" value={selection.dimension} onChange={(event) => patch({ dimension: event.target.value })}>
          <option value="">All records together</option>{columns.map((item) => <option key={item.columnName}>{item.columnName}</option>)}
        </select></label>
        <label>Time field<select className="analysis-input mt-1 w-full" aria-label="Discovery time field" value={selection.period} onChange={(event) => patch({ period: event.target.value })}>
          <option value="">No time comparison</option>{columns.map((item) => <option key={item.columnName}>{item.columnName}</option>)}
        </select></label>
      </div>
      {selection.period && <label className="block">Period format<select className="analysis-input ml-2" aria-label="Discovery period format" value={selection.periodMode} onChange={(event) => patch({ periodMode: event.target.value as DiscoverySelection['periodMode'] })}>
        <option value="year_code">Year / fiscal-year code</option><option value="year">Date → year</option><option value="month">Date → month</option><option value="day">Date → day</option>
      </select></label>}
      {selection.filters.map((filter, index) => {
        const updateFilter = (value: Partial<DiscoveryFilter>) => patch({ filters: selection.filters.map((item, i) => i === index ? { ...item, ...value } : item) });
        return <div key={index} className="flex flex-wrap gap-2">
          <select className="analysis-input min-w-0 flex-1" aria-label={`Filter ${index + 1} column`} value={filter.column} onChange={(event) => updateFilter({ column: event.target.value })}>{columns.map((item) => <option key={item.columnName}>{item.columnName}</option>)}</select>
          <select className="analysis-input" aria-label={`Filter ${index + 1} operator`} value={filter.operator} onChange={(event) => updateFilter({ operator: event.target.value as DiscoveryFilter['operator'] })}><option value="equals">equals exactly</option><option value="contains">contains text</option><option value="missing">is missing</option></select>
          {filter.operator !== 'missing' && <input className="analysis-input min-w-0 flex-1" aria-label={`Filter ${index + 1} value`} value={filter.value} onChange={(event) => updateFilter({ value: event.target.value })} />}
          <button type="button" className="analysis-button" aria-label={`Remove filter ${index + 1}`} onClick={() => patch({ filters: selection.filters.filter((_, i) => i !== index) })}>Remove</button>
        </div>;
      })}
      <button type="button" className="analysis-button" disabled={!columns.length || selection.filters.length >= 8} onClick={() => patch({ filters: [...selection.filters, { column: columns[0].columnName, operator: 'equals', value: '' }] })}>Add scope filter</button>
    </fieldset>
    {selection.aggregation !== 'count' && <p className="text-warning">Confirm units and row meaning. Separate budget phases, account classes, overlapping totals, and populations before interpreting this calculation.</p>}
    <div className="flex flex-wrap gap-2">
      <button type="button" className="analysis-button" disabled={running || !dataset} onClick={() => void analyze()}>Discover patterns</button>
      {running && <button type="button" className="analysis-button" onClick={() => { cancelled.current = true; setProgress('Stopping after the current query…'); }}>Stop after current query</button>}
      {report && <button type="button" className="analysis-button" disabled={running} onClick={exportReport}>Export discovery report</button>}
    </div>
    {progress && <p role="status">{progress}</p>}{error && <p role="alert" className="text-danger">{error}</p>}
    {report && <div className="space-y-3 border-t border-line pt-3">
      {stale && <p role="status" className="text-warning">Controls changed. Results below still describe the previous analysis; run discovery to update them.</p>}
      <p className="break-all text-muted">Results: {report.plan.dataset.displayName} · {report.plan.dataset.sourceUri ?? 'Local export'}<br />Snapshot {report.plan.dataset.snapshotId} · SHA-256 {report.plan.dataset.sourceSha256}<br />{report.createdAt} · {report.results.length}/{report.plan.queries.length} checks returned</p>
      {/* The changes figure states the gap count in its headline; the bullet would repeat it. */}
      <ul className="list-disc space-y-1 pl-4">{discoveryFindings(report).filter((finding) => !(changesShown && finding.includes('time comparisons have gaps'))).map((finding) => <li key={finding}>{finding}</li>)}</ul>
      <details><summary className="text-warning">Coverage and interpretation</summary><ul className="mt-2 list-disc space-y-1 pl-4 text-muted">{report.plan.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul></details>
      {report.results.map(({ query, result, error: queryError }) => <details key={query.id} open={query.id === 'groups' || query.id === 'changes'} className="rounded border border-line p-2">
        <summary className="font-semibold">{query.title}{result ? ` · ${result.rowCount}${result.truncated ? '+' : ''} results` : ' · failed'}</summary>
        <p className="my-2 leading-relaxed text-muted">{query.method}</p>
        <button type="button" className="analysis-button" onClick={() => onInspect(query.sql)}>Open query in SQL</button>
        {queryError && <p role="alert" className="mt-2 text-danger">{queryError}</p>}
        {result && query.id === 'groups' && <div className="mt-3" style={stale ? { opacity: 0.5 } : undefined}><GroupsFigure result={result} plan={report.plan} sources={sourcesOf(report.plan)} onInspect={onInspect} /></div>}
        {result && query.id === 'quality' && <div className="mt-3" style={stale ? { opacity: 0.5 } : undefined}><QualityFigure result={result} sources={sourcesOf(report.plan)} /></div>}
        {result && query.id === 'changes' && <div className="mt-3" style={stale ? { opacity: 0.5 } : undefined}><ChangesFigure changes={result} groups={report.results.find((item) => item.query.id === 'groups')?.result} series={series} plan={report.plan} sources={sourcesOf(report.plan)} onInspect={onInspect} /></div>}
        {result && (query.id === 'changes'
          ? <details className="mt-2"><summary className="cursor-pointer text-fg-secondary">Rows</summary><DiscoveryTable result={result} /></details>
          : <DiscoveryTable result={result} onRow={query.id === 'groups' ? (row) => onInspect(discoveryDrilldown(report.plan, row[result.columns.indexOf('group_value')])) : undefined} />)}
      </details>)}
    </div>}
  </div>;
}

const SERIES_ROW_LIMIT = 2000;
const GROUP_ROWS_SHOWN = 15;

type SeriesState = { status: 'loading' } | { status: 'ready'; result: SqlQueryResult } | { status: 'error'; message: string };

function sourcesOf(plan: DiscoveryPlan): ChartSources {
  return {
    tables: [plan.dataset.tableName],
    snapshot: { name: plan.dataset.displayName, id: plan.dataset.snapshotId, sha256: plan.dataset.sourceSha256 },
  };
}

const compact = (value: number, signed = false) => formatValue(value, { kind: 'count', compact: true, signed });
const exact = (value: number) => value.toLocaleString('en-US', { maximumFractionDigits: 2 });
const columnOf = (result: SqlQueryResult) => (name: string) => result.columns.indexOf(name);

/** 'Differences by group': observed value per group, its share of records, and its missing values. */
function GroupsFigure({ result, plan, sources, onInspect }: { result: SqlQueryResult; plan: DiscoveryPlan; sources: ChartSources; onInspect(sql: string): void }) {
  const at = columnOf(result);
  const shown = result.rows.slice(0, GROUP_ROWS_SHOWN);
  const anyMissing = shown.some((row) => (finiteCell(row[at('missing_values')]) ?? 0) > 0);
  const byKey = new Map(shown.map((row) => [groupKey(row[at('group_value')]), row]));
  const top = result.rows.reduce<unknown[] | null>((best, row) => (best === null || (finiteCell(row[at('record_share_percent')]) ?? -1) > (finiteCell(best[at('record_share_percent')]) ?? -1) ? row : best), null);
  const topShare = top ? finiteCell(top[at('record_share_percent')]) : null;
  const rows: BarTableRow[] = shown.map((row) => {
    const value = finiteCell(row[at('observed_value')]);
    const share = finiteCell(row[at('record_share_percent')]);
    const missing = finiteCell(row[at('missing_values')]) ?? 0;
    const sourceRows = finiteCell(row[at('source_rows')]);
    const label = groupLabel(row[at('group_value')]);
    const tooltip = { rows: [
      { key: 'value', value: value === null ? 'not measured' : exact(value), label: 'observed value' },
      { key: 'rows', value: sourceRows === null ? '—' : exact(sourceRows), label: 'source rows' },
      { key: 'missing', value: exact(missing), label: 'values missing' },
    ] };
    return {
      key: groupKey(row[at('group_value')]),
      name: label,
      label: `${label}: ${value === null ? 'not measured' : exact(value)}, ${share === null ? 'share not computed' : `${share}% of records`}${missing > 0 ? `, ${missing} values missing` : ''}`,
      header: <span className={row[at('group_value')] === null ? 'text-muted' : 'text-fg'}>{label}</span>,
      cells: {
        value: value === null
          ? { value: absent('not-recorded', 'No reported values in this group'), text: 'not measured', tooltip }
          : { value: measured(value), text: compact(value), tooltip },
        share: share === null
          ? { value: absent('not-recorded', 'No rows in scope'), text: '—', tooltip }
          : { value: measured(share / 100), text: `${share}%`, tooltip },
        ...(anyMissing ? { missing: { value: measured(missing), text: missing > 0 ? `${exact(missing)} missing` : '0', tooltip } } : {}),
      },
    };
  });
  return (
    <BarTable
      title={plan.selection.dimension ? `Observed value by ${plan.selection.dimension}` : 'Observed value'}
      headline={top && topShare !== null ? `${groupLabel(top[at('group_value')])} accounts for ${topShare}% of records.` : null}
      sources={sources}
      summary={`${result.rowCount}${result.truncated ? '+' : ''} groups; the first ${shown.length} are drawn. Missing values are drawn hatched, outside the observed value.`}
      status={result.rowCount === 0 ? 'empty' : 'ready'}
      emptyText="No matching results for this check."
      rowHeader={plan.selection.dimension || 'Group'}
      layout="stacked"
      hatchLabel="Values missing (outside the measure)"
      columns={[
        { key: 'value', label: plan.selection.aggregation === 'count' ? 'Records' : plan.selection.aggregation === 'sum' ? 'Sum' : 'Mean', scale: 'own-max', barWidth: 88 },
        { key: 'share', label: 'Share of records', scale: 'own-0-100', barWidth: 48 },
        ...(anyMissing ? [{ key: 'missing', label: 'Missing', note: 'outside the measure', scale: 'own-max' as const, hatched: true, barWidth: 36 }] : []),
      ]}
      rows={rows}
      onRowActivate={(key) => { const row = byKey.get(key); if (row) onInspect(discoveryDrilldown(plan, row[at('group_value')])); }}
      twin={{
        caption: 'Groups as drawn',
        columns: [{ key: 'group', label: 'Group' }, { key: 'value', label: 'Observed value', align: 'end' }, { key: 'share', label: 'Record share %', align: 'end' }, { key: 'missing', label: 'Missing values', align: 'end' }, { key: 'rows', label: 'Source rows', align: 'end' }],
        rows: shown.map((row) => ({
          group: groupLabel(row[at('group_value')]),
          value: finiteCell(row[at('observed_value')]) ?? absent('not-recorded', 'no reported values'),
          share: finiteCell(row[at('record_share_percent')]),
          missing: finiteCell(row[at('missing_values')]),
          rows: finiteCell(row[at('source_rows')]),
        })),
      }}
      footer={result.rowCount > shown.length && <p className="text-caption text-muted">{`${(result.rowCount - shown.length).toLocaleString('en-US')}${result.truncated ? '+' : ''} more groups in the rows below.`}</p>}
    />
  );
}

/** 'Column quality': each column's reported share, with the missing share hatched after it. */
function QualityFigure({ result, sources }: { result: SqlQueryResult; sources: ChartSources }) {
  const at = columnOf(result);
  const measuredRows = result.rows.map((row) => {
    const total = finiteCell(row[at('source_rows')]) ?? 0;
    const missing = finiteCell(row[at('missing_rows')]) ?? 0;
    return { row, total, missing, missingShare: total > 0 ? missing / total : null };
  }).sort((a, b) => (b.missingShare ?? -1) - (a.missingShare ?? -1));
  const halfEmpty = measuredRows.filter((item) => (item.missingShare ?? 0) > 0.5).length;
  const rows: BarTableRow[] = measuredRows.map(({ row, total, missing, missingShare }) => {
    const name = String(row[at('column_name')]);
    const reported = missingShare === null ? null : 1 - missingShare;
    return {
      key: name,
      name,
      label: `${name}: ${reported === null ? 'no rows in scope' : `${formatValue(reported, { kind: 'percent', digits: 0 })} reported, ${exact(missing)} of ${exact(total)} missing`}`,
      header: <span className="flex flex-wrap items-baseline gap-x-2"><span className="font-mono text-code text-fg">{name}</span><span className="text-caption text-muted">{String(row[at('logical_type')])}</span></span>,
      cells: {
        reported: reported === null
          ? { value: absent('not-recorded', 'No rows in scope'), text: 'no rows' }
          : {
            value: measured(reported),
            remainder: missingShare ?? 0,
            text: `${formatValue(reported, { kind: 'percent', digits: 0 })} reported`,
            tooltip: { rows: [
              { key: 'missing', value: exact(missing), label: 'missing' },
              { key: 'rows', value: exact(total), label: 'source rows' },
            ], note: String(row[at('logical_type')]) },
          },
      },
    };
  });
  return (
    <BarTable
      title="Column completeness"
      headline={`${halfEmpty} of ${measuredRows.length} columns are more than half empty.`}
      sources={sources}
      summary={`${measuredRows.length} columns, sorted by missing share. Filled is the reported share; the hatched remainder is the missing share.`}
      status={result.rowCount === 0 ? 'empty' : 'ready'}
      rowHeader="Column"
      columns={[{ key: 'reported', label: 'Reported', note: 'hatched: missing', scale: 'own-0-100', barWidth: 120 }]}
      hatchLabel="Missing share"
      rows={rows}
      twin={{
        caption: 'Column completeness',
        columns: [{ key: 'column', label: 'Column' }, { key: 'type', label: 'Type' }, { key: 'missing', label: 'Missing', align: 'end' }, { key: 'rows', label: 'Source rows', align: 'end' }],
        rows: measuredRows.map(({ row, total, missing }) => ({ column: String(row[at('column_name')]), type: String(row[at('logical_type')]), missing, rows: total })),
      }}
    />
  );
}

/** 'Largest changes over time' as small multiples: one panel per group, lines broken at gaps. */
function ChangesFigure({ changes, groups, series, plan, sources, onInspect }: {
  changes: SqlQueryResult;
  groups: SqlQueryResult | undefined;
  series: SeriesState | null;
  plan: DiscoveryPlan;
  sources: ChartSources;
  onInspect(sql: string): void;
}) {
  const order = groups ? groups.rows.map((row) => row[groups.columns.indexOf('group_value')]) : null;
  const drawn = changes.truncated ? (series?.status === 'ready' ? series.result : null) : changes;
  const model = drawn ? discoverySeriesModel(drawn, order, plan.selection.periodMode) : null;
  const largest = largestDiscoveryChange(changes);
  const gaps = groupsWithMissingPeriods(changes);
  const mean = plan.selection.aggregation === 'mean';

  let headline: string;
  if (largest) {
    const rise = largest.change >= 0;
    const pct = largest.changePercent === null ? '' : ` (${formatValue(largest.changePercent / 100, { kind: 'percent', signed: true, digits: 0 })})`;
    headline = `Largest change: ${groupLabel(largest.group)} ${rise ? 'rose' : 'fell'} ${compact(Math.abs(largest.change))}${pct} from ${String(largest.from)} to ${String(largest.to)}`;
  } else {
    headline = 'No group has two adjacent periods with reported values to compare';
  }
  if (gaps > 0) headline += `; ${changes.truncated ? 'at least ' : ''}${gaps} ${gaps === 1 ? 'group has' : 'groups have'} missing periods and ${gaps === 1 ? 'is' : 'are'} not compared across them`;
  headline += '.';

  const panelOf = largest && model ? model.groups.find((group) => group.key === groupKey(largest.group)) : undefined;
  const bracket = largest && model && panelOf ? {
    panelKey: panelOf.key,
    lineKey: 'value',
    from: model.slots.findIndex((slot) => slot.key === String(largest.from)),
    to: model.slots.findIndex((slot) => slot.key === String(largest.to)),
    label: `${compact(largest.change, true)}${largest.changePercent === null ? '' : ` (${formatValue(largest.changePercent / 100, { kind: 'percent', signed: true, digits: 0 })})`} ${String(largest.from)}→${String(largest.to)}`,
  } : null;

  const status = changes.rowCount === 0 ? 'empty' : changes.truncated && series?.status === 'error' ? 'error' : !model ? 'loading' : 'ready';
  const lines = (cells: readonly ({ observed: number | null; missingValues: number | null } | null)[], slotGap: (i: number) => boolean): SeriesLine => ({
    key: 'value',
    label: 'Observed value',
    role: 'primary',
    values: cells.map((cell, i) => (cell && cell.observed !== null
      ? measured(cell.observed)
      : absent('not-recorded', slotGap(i) ? 'No rows for any group in this period' : cell ? 'No reported values in this period' : 'No rows for this group in this period'))),
    incomplete: new Set(cells.flatMap((cell, i) => (cell && (cell.missingValues ?? 0) > 0 ? [i] : []))),
  });

  return (
    <SeriesSmallMultiples
      title="Changes over time"
      headline={headline}
      sources={{ ...sources, note: mean ? 'Means: the value axis does not start at zero' : undefined }}
      summary={`${model?.groups.length ?? 0} groups drawn over ${model?.slots.filter((slot) => !slot.gap).length ?? 0} periods. Lines break where a period has no rows; hollow markers are periods with missing values.`}
      status={status}
      error={series?.status === 'error' ? series.message : null}
      emptyText="No matching results for this check."
      slots={model?.slots ?? []}
      panels={(model?.groups ?? []).map((group) => {
        const line = lines(group.cells, (i) => model?.slots[i]?.gap ?? false);
        const observed = group.cells.flatMap((cell, i) => (cell && cell.observed !== null ? [i] : []));
        const last = observed.length ? group.cells[observed[observed.length - 1]]?.observed ?? null : null;
        const missing = line.values.flatMap((value, i) => (value.state !== 'value' && observed.length && i > observed[0] && i < observed[observed.length - 1] ? [model?.slots[i]?.title ?? model?.slots[i]?.label ?? ''] : []));
        return {
          key: group.key,
          title: group.label,
          value: last === null ? undefined : compact(last),
          lines: [line],
          summary: `${group.label}: ${observed.length} periods${observed.length ? `, ${model?.slots[observed[0]].label} to ${model?.slots[observed[observed.length - 1]].label}` : ''}${missing.length ? `, no data at ${missing.join(', ')}` : ''}${bracket?.panelKey === group.key ? `, largest change ${bracket.label}` : ''}.`,
        };
      })}
      kind="count"
      yScale="shared"
      zero={!mean}
      bracket={bracket && bracket.from >= 0 && bracket.to >= 0 ? bracket : null}
      describe={(panelKey, slot) => {
        const cell = model?.groups.find((group) => group.key === panelKey)?.cells[slot] ?? null;
        if (!cell) return { rows: [], note: model?.slots[slot]?.gap ? model.slots[slot].title : 'No rows for this group in this period' };
        return {
          rows: [
            { key: 'value', value: cell.observed === null ? 'not measured' : exact(cell.observed), label: 'observed' },
            { key: 'rows', value: cell.sourceRows === null ? '—' : exact(cell.sourceRows), label: 'source rows' },
            { key: 'missing', value: cell.missingValues === null ? '—' : exact(cell.missingValues), label: 'values missing' },
          ],
          note: cell.change !== null ? `${compact(cell.change, true)} vs the previous period` : cell.status ?? undefined,
        };
      }}
      onPanelActivate={(panelKey) => {
        const group = model?.groups.find((candidate) => candidate.key === panelKey);
        if (group) onInspect(discoveryDrilldown(plan, group.value));
      }}
      table={{
        caption: 'Values per group and period',
        columns: [{ key: 'group', label: 'Group' }, { key: 'period', label: 'Period' }, { key: 'value', label: 'Observed value', align: 'end' }, { key: 'status', label: 'Comparison' }],
        rows: (model?.groups ?? []).flatMap((group) => (model?.slots ?? []).map((slot, i) => {
          const cell = group.cells[i];
          return { group: group.label, period: slot.gap ? slot.title ?? '' : slot.label, value: cell?.observed ?? absent('not-recorded', cell ? 'no reported values' : 'no rows in this period'), status: cell?.status ?? null };
        })),
      }}
      footer={model && model.otherGroups > 0 ? <p className="text-caption text-muted">{`Other groups (${model.otherGroups}): see rows.`}</p> : changes.truncated && drawn?.truncated ? <p className="text-caption text-warning">{`The series query also reached its ${SERIES_ROW_LIMIT.toLocaleString('en-US')}-row limit; later periods may be missing from the panels.`}</p> : null}
      storageKey="discovery-changes"
    />
  );
}

function DiscoveryTable({ result, onRow }: { result: SqlQueryResult; onRow?: (row: unknown[]) => void }) {
  return <div className="mt-2">
    {result.truncated && <p className="mb-1 text-warning">First {result.rowCount} results shown. Calculations use all matching records. Open the query to explore or export more.</p>}
    {!result.rowCount ? <p>No matching results for this check.</p> : <div className="max-h-72 overflow-auto"><table className="w-full text-left text-label">
      <thead><tr>{result.columns.map((name) => <th key={name} className="whitespace-nowrap border-b border-line-strong p-1">{name}</th>)}{onRow && <th>Evidence</th>}</tr></thead>
      <tbody>{result.rows.map((row, i) => <tr key={i}>{row.map((cell, j) => <td key={j} title={cell === null ? 'Missing / unavailable' : String(cell)} className="max-w-64 truncate border-b border-line p-1">{cell === null ? '—' : String(cell)}</td>)}{onRow && <td><button type="button" className="analysis-button whitespace-nowrap" onClick={() => onRow(row)}>Explore records</button></td>}</tr>)}</tbody>
    </table></div>}
  </div>;
}
