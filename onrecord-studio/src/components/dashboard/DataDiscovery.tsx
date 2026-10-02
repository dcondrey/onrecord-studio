import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../ipc';
import type { AnalyticalSchema, SqlQueryResult } from '../../types';
import { buildDiscoveryPlan, discoveryDefaults, discoveryDrilldown, discoveryFindings, isNumericColumn,
  type DiscoveryFilter, type DiscoveryReport, type DiscoverySelection } from '../../utils/dataDiscovery';
import { useDiscoveryController } from './AutomaticDiscoveryProvider';

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

  const analyze = async () => {
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
    } catch (reason) {
      if (version === runVersion.current) setError(String(reason));
    } finally {
      release?.();
      if (version === runVersion.current) setRunning(false);
    }
  };
  const exportReport = () => {
    if (!report) return;
    const url = URL.createObjectURL(new Blob([JSON.stringify({ ...report, findings: discoveryFindings(report) }, null, 2)], { type: 'application/json' }));
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `onrecord-discovery-${report.plan.dataset.snapshotId.slice(0, 16)}.json`;
    anchor.click();
    URL.revokeObjectURL(url);
  };
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
      <ul className="list-disc space-y-1 pl-4">{discoveryFindings(report).map((finding) => <li key={finding}>{finding}</li>)}</ul>
      <details><summary className="cursor-pointer text-warning">Coverage and interpretation</summary><ul className="mt-2 list-disc space-y-1 pl-4 text-muted">{report.plan.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul></details>
      {report.results.map(({ query, result, error: queryError }) => <details key={query.id} open={query.id === 'groups' || query.id === 'changes'} className="rounded border border-line p-2">
        <summary className="cursor-pointer font-semibold">{query.title}{result ? ` · ${result.rowCount}${result.truncated ? '+' : ''} results` : ' · failed'}</summary>
        <p className="my-2 leading-relaxed text-muted">{query.method}</p>
        <button type="button" className="analysis-button" onClick={() => onInspect(query.sql)}>Open query in SQL</button>
        {queryError && <p role="alert" className="mt-2 text-danger">{queryError}</p>}
        {result && <DiscoveryTable result={result} onRow={query.id === 'groups' ? (row) => onInspect(discoveryDrilldown(report.plan, row[result.columns.indexOf('group_value')])) : undefined} />}
      </details>)}
    </div>}
  </div>;
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
