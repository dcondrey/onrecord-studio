import { useEffect, useMemo, useState } from 'react';
import { open, save } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type {
  AnalyticalDatasetKind,
  AnalyticalImportCandidate,
  AnalyticalImportResult,
  AnalyticalSchema,
  CityDataset,
  EmbeddingIndexReport,
  EmbeddingIndexStatus,
  EmbeddingModelStatus,
  HybridSearchResult,
  SqlQueryResult,
  SqlExportResult,
  SqlTemplate,
} from '../../types';
import Icon from '../ui/icons';
import BudgetComparison from './BudgetComparison';
import DataDiscovery from './DataDiscovery';
import DataPackImport from './DataPackImport';
import FacilityCoveragePanel from './FacilityCoveragePanel';
import EncampmentOrdinancePanel from './EncampmentOrdinancePanel';
import { useAppStore } from '../../store/useAppStore';
import AutomaticDiscovery from './AutomaticDiscovery';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';

const datasetKinds: AnalyticalDatasetKind[] = [
  'REGIONAL_DATA',
  'RIPA_STOPS',
  'RIPA_SEARCH_BASIS',
  'CALLS_FOR_SERVICE',
  'CALLS_CODEBOOK',
  'NIBRS_OFFENSES',
  'OPERATING_BUDGET',
  'BUDGET_DIMENSION',
];

/** `yearly` marks the tables the publisher splits by year; the fetch refuses a year for any other. */
const cityDatasets: { value: CityDataset; label: string; yearly?: true }[] = [
  { value: 'NIBRS_OFFENSES', label: 'SDPD NIBRS offenses (by year)', yearly: true },
  { value: 'CALLS_FOR_SERVICE', label: 'SDPD calls for service (by year)', yearly: true },
  { value: 'GET_IT_DONE_ENCAMPMENTS', label: 'Get It Done encampment reports (closed, by year)', yearly: true },
  { value: 'GET_IT_DONE_OPEN', label: 'Get It Done open reports (all services, today)' },
  { value: 'GET_IT_DONE_OUTCOMES', label: 'Get It Done closed reports (those seen open, by year)', yearly: true },
  { value: 'SCHOOLS', label: 'SANDAG schools' },
  { value: 'TRANSIT_STOPS', label: 'SANDAG transit stops (GTFS)' },
  { value: 'OPERATING_BUDGET', label: 'City operating budget (all years)' },
  { value: 'RIPA_STOPS', label: 'SDPD stop records (RIPA): persons stopped' },
  { value: 'RIPA_STOP_REASONS', label: 'SDPD stop records (RIPA): reasons for the stop' },
  { value: 'RIPA_STOP_RESULTS', label: 'SDPD stop records (RIPA): results of the stop' },
  { value: 'RIPA_PROPERTY_SEIZURE_BASIS', label: 'SDPD stop records (RIPA): basis for property seizure' },
  { value: 'SHELTER_INVENTORY', label: 'Emergency shelter inventory (UCSD Homelessness Hub, all years)' },
  { value: 'OPERATING_ACTUALS', label: 'City operating actuals (all years)' },
];

export default function AnalysisWorkbench() {
  const { controller: discovery, state: automatic } = useAutomaticDiscovery();
  const reviewSearchResult = (result: HybridSearchResult) => {
    const state = useAppStore.getState();
    state.setActivePortal(result.portalKey);
    state.setSearchFilters('', null, null, null);
    state.setSelectedRequest(result.requestId, result.portalKey);
    state.setWorkspaceView('evidence');
  };
  const [modelStatus, setModelStatus] = useState<EmbeddingModelStatus | null>(null);
  const [indexStatus, setIndexStatus] = useState<EmbeddingIndexStatus | null>(null);
  const [indexReport, setIndexReport] = useState<EmbeddingIndexReport | null>(null);
  const [hybridQuery, setHybridQuery] = useState('homelessness spending shelter outcomes contract amendments');
  const [jurisdiction, setJurisdiction] = useState('');
  const [hybridResults, setHybridResults] = useState<HybridSearchResult[]>([]);
  const [schema, setSchema] = useState<AnalyticalSchema | null>(null);
  const [sql, setSql] = useState('');
  const [sqlResult, setSqlResult] = useState<SqlQueryResult | null>(null);
  const [exportResult, setExportResult] = useState<SqlExportResult | null>(null);
  const [datasetKind, setDatasetKind] = useState<AnalyticalDatasetKind>('REGIONAL_DATA');
  const [displayName, setDisplayName] = useState('');
  const [sourcePath, setSourcePath] = useState('');
  const [sourceUri, setSourceUri] = useState('');
  const [worksheet, setWorksheet] = useState('');
  const [observationStart, setObservationStart] = useState('');
  const [observationEnd, setObservationEnd] = useState('');
  const [partialDataset, setPartialDataset] = useState(true);
  const [candidates, setCandidates] = useState<AnalyticalImportCandidate[]>([]);
  const [selectedCandidate, setSelectedCandidate] = useState<AnalyticalImportCandidate | null>(null);
  const [importResult, setImportResult] = useState<AnalyticalImportResult | null>(null);
  const [cityDataset, setCityDataset] = useState<CityDataset>('NIBRS_OFFENSES');
  const [cityYear, setCityYear] = useState(String(new Date().getFullYear() - 1));
  const cityDatasetIsYearly = cityDatasets.some((item) => item.value === cityDataset && item.yearly);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // `sqlResult` is written asynchronously from two independent paths: the automatic-discovery
  // inspection effect below, and the manual Run/Compare buttons (via `runGatedSql`). Both issue a
  // request through the same gate so whichever the user asked for LAST wins, regardless of which
  // resolves first. Every OTHER place that changes `sql` without issuing a request of its own
  // (dataset/template presets, direct textarea edits) must call `sqlGate.invalidate()` so it can't
  // be silently overwritten by an in-flight request's result once it resolves for a query the
  // editor no longer shows.
  const sqlGate = useLatestRequestGate();
  // Guards `hybridResults` the same way: without it, an older search whose IPC round-trip resolves
  // after a newer one would overwrite the newer query's results.
  const hybridSearchGate = useLatestRequestGate();

  const templates = schema?.templates ?? [];
  const groupedColumns = useMemo(() => {
    const groups = new Map<string, NonNullable<typeof schema>['columns']>();
    for (const column of schema?.columns ?? []) {
      groups.set(column.tableName, [...(groups.get(column.tableName) ?? []), column]);
    }
    return [...groups.entries()];
  }, [schema]);

  const refresh = async () => {
    const results = await Promise.allSettled([
      invokeCommand('get_embedding_model_status', {}),
      invokeCommand('get_embedding_index_status', {}),
      invokeCommand('get_analytical_schema', {}),
      invokeCommand('list_analytical_import_candidates', {}),
    ]);
    const [model, index, analytical, acquired] = results;
    if (model.status === 'fulfilled') setModelStatus(model.value);
    if (index.status === 'fulfilled') setIndexStatus(index.value);
    if (analytical.status === 'fulfilled') setSchema(analytical.value);
    if (acquired.status === 'fulfilled') setCandidates(acquired.value);
    const failures = results.filter((result) => result.status === 'rejected');
    if (failures.length) setError(failures.map((result) => String(result.reason)).join('; '));
  };

  useEffect(() => {
    void refresh().catch((reason) => setError(String(reason)));
  }, []);

  const inspection = automatic?.inspection;
  useEffect(() => {
    if (!inspection) return;
    // `active` tracks only whether *this effect instance* is still mounted, so it governs the
    // loading indicator: that must clear when this specific fetch settles, regardless of whether
    // anything downstream (a preset, a template, an edit, a manual run) has since told `sqlGate`
    // not to trust this fetch's *data* any more. Conflating the two left the indicator stuck on
    // "Loading supporting records" (and the Import button disabled) whenever the user moved on
    // to a different query before this fetch resolved.
    let active = true;
    const request = sqlGate.begin();
    const release = discovery?.hold();
    setSql(inspection.sql);
    setSqlResult(null);
    setStatus('Loading supporting records');
    void invokeCommand('run_analytical_sql', { sql: inspection.sql, limit: 500 }).then((result) => {
      if (sqlGate.isCurrent(request)) setSqlResult(result);
    }).catch((reason) => { if (sqlGate.isCurrent(request)) setError(String(reason)); }).finally(() => { release?.(); if (active) setStatus(null); });
    window.requestAnimationFrame(() => document.getElementById('analytical-sql-editor')?.focus());
    return () => {
      active = false;
      // Only invalidate if nothing has superseded this request yet. A manual run that already
      // began (a newer token) must not be cancelled by this effect's own teardown.
      if (sqlGate.isCurrent(request)) sqlGate.invalidate();
      release?.();
    };
  }, [discovery, inspection, sqlGate]);

  // `isCurrent` lets a gated caller keep a superseded request's error and status off the screen;
  // the newer request owns both from the moment it begins.
  const run = async <T,>(
    label: string,
    action: () => Promise<T>,
    onSuccess?: (value: T) => void,
    isCurrent: () => boolean = () => true,
  ) => {
    setStatus(label);
    setError(null);
    const release = discovery?.hold();
    try {
      const value = await action();
      onSuccess?.(value);
      await refresh();
    } catch (reason) {
      if (isCurrent()) setError(String(reason));
    } finally {
      if (isCurrent()) setStatus(null);
      release?.();
      discovery?.refresh();
    }
  };

  // Manual entry point for the two call sites that write `sqlResult` outside the inspection
  // effect (the Run button, the budget-comparison button). `begin()` synchronously, before the
  // request is issued, so it supersedes an in-flight inspection query (or a previous manual one)
  // the instant the user acts.
  const runGatedSql = (label: string, query: string, limit = 500) => {
    const request = sqlGate.begin();
    return run(
      label,
      () => invokeCommand('run_analytical_sql', { sql: query, limit }),
      (result) => {
        if (sqlGate.isCurrent(request)) setSqlResult(result);
      },
      () => sqlGate.isCurrent(request),
    );
  };

  return (
    <main className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto grid w-full max-w-[1800px] gap-4 p-4 lg:grid-cols-[minmax(360px,0.9fr)_minmax(0,1.3fr)] lg:p-5">
        <div className="lg:col-span-2"><AutomaticDiscovery /></div>
        <section className="min-w-0 space-y-4">
          <Panel title="Semantic Index" icon="sparkles">
            <div className="grid gap-2 text-label text-fg-secondary">
              <Metric label="Model" value={modelStatus ? modelStatus.modelId : 'Checking'} />
              <Metric label="State" value={modelStatus?.ready ? 'Ready' : modelStatus?.installed ? 'Installed' : 'Not installed'} />
              <Metric label="Vectors" value={indexStatus ? `${indexStatus.indexedChunkCount.toLocaleString()} chunks` : 'Checking'} />
              <Metric label="Coverage" value={indexStatus ? `${indexStatus.indexedDocumentCount.toLocaleString()} indexed / ${indexStatus.pendingDocumentCount.toLocaleString()} pending` : 'Checking'} />
              {modelStatus?.artifactManifestSha256 && <Metric label="Manifest" value={modelStatus.artifactManifestSha256.slice(0, 16)} />}
            </div>
            <div className="mt-3 flex flex-wrap gap-2">
              <button className="analysis-button" type="button" onClick={() => void run('Installing model', () => invokeCommand('initialize_embedding_model', {}), setModelStatus)}>
                Initialize model
              </button>
              <button className="analysis-button" type="button" onClick={() => void run('Indexing documents', () => invokeCommand('rebuild_embedding_index', { rebuild: false }), setIndexReport)}>
                Index missing
              </button>
              <button className="analysis-button" type="button" onClick={() => void run('Rebuilding semantic index', () => invokeCommand('rebuild_embedding_index', { rebuild: true }), setIndexReport)}>
                Rebuild
              </button>
              <button className="analysis-button" type="button" onClick={() => void run('Cancelling indexing', () => invokeCommand('cancel_embedding_index', {}))}>
                Cancel
              </button>
            </div>
            {indexReport && (
              <p className="mt-2 text-label text-muted">
                Indexed {indexReport.indexedDocuments.toLocaleString()} documents, {indexReport.indexedChunks.toLocaleString()} chunks; skipped {indexReport.skippedDocuments.toLocaleString()}.
              </p>
            )}
          </Panel>

          <Panel title="Analytical Import" icon="database">
            <p className="mb-3 text-label leading-relaxed text-muted">Choose an acquired table or a local CSV, TSV, JSON, GeoJSON, JSONL, XLSX, XLS, or ODS export. General regional data infers numeric columns while preserving codes and suppression markers.</p>
            <div className="grid grid-cols-[minmax(0,1fr)] gap-2">
              <select aria-label="Acquired table" value={selectedCandidate ? `${selectedCandidate.portalKey}:${selectedCandidate.documentId}` : ''} onChange={(event) => {
                const candidate = candidates.find((item) => `${item.portalKey}:${item.documentId}` === event.target.value) ?? null;
                setSelectedCandidate(candidate);
                setSourcePath(candidate?.localPath ?? '');
                setSourceUri(candidate?.sourceUri ?? '');
                setDisplayName(candidate?.fileName ?? '');
              }} className="analysis-input">
                <option value="">Choose from the latest 500 acquired tables, or browse files</option>
                {candidates.map((candidate) => <option key={`${candidate.portalKey}:${candidate.documentId}`} value={`${candidate.portalKey}:${candidate.documentId}`}>{candidate.fileName} · {candidate.sourceUri ?? candidate.portalKey}</option>)}
              </select>
              <button type="button" className="analysis-button w-fit" onClick={() => void run('Refreshing acquired tables', refresh)}>Refresh acquired tables</button>
              <select aria-label="Dataset type" value={datasetKind} onChange={(event) => setDatasetKind(event.target.value as AnalyticalDatasetKind)} className="analysis-input">
                {datasetKinds.map((kind) => <option key={kind} value={kind}>{kind.split('_').join(' ')}</option>)}
              </select>
              <input aria-label="Dataset name" className="analysis-input" value={displayName} onChange={(event) => setDisplayName(event.target.value)} placeholder="Dataset name" />
              <div className="flex gap-2">
                <input aria-label="Table file path" className="analysis-input min-w-0 flex-1" value={sourcePath} onChange={(event) => { setSourcePath(event.target.value); setSelectedCandidate(null); }} placeholder="/path/to/local.csv" />
                <button type="button" className="analysis-button" onClick={() => void run('Choosing table', async () => {
                  const path = await open({ multiple: false, filters: [{ name: 'Tables', extensions: ['csv', 'tsv', 'json', 'geojson', 'jsonl', 'ndjson', 'xlsx', 'xls', 'ods'] }] });
                  if (typeof path === 'string') { setSourcePath(path); setSelectedCandidate(null); setSourceUri(''); setDisplayName(path.split(/[\\/]/).pop() ?? 'Dataset'); }
                })}>Browse files</button>
              </div>
              <input aria-label="Original source URL" className="analysis-input" value={sourceUri} onChange={(event) => setSourceUri(event.target.value)} placeholder="Original source URL (including FTP or SFTP export paths)" />
              <input aria-label="Worksheet name" className="analysis-input" value={worksheet} onChange={(event) => setWorksheet(event.target.value)} placeholder="Worksheet name (required for a workbook with several sheets)" />
              <div className="grid grid-cols-2 gap-2">
                <label className="text-label leading-4 text-muted">Observation start<input aria-label="Observation start" type="date" className="analysis-input mt-1 w-full" value={observationStart} onChange={(event) => setObservationStart(event.target.value)} /></label>
                <label className="text-label leading-4 text-muted">Observation end<input aria-label="Observation end" type="date" className="analysis-input mt-1 w-full" value={observationEnd} onChange={(event) => setObservationEnd(event.target.value)} /></label>
              </div>
              <label className="flex items-center gap-2 text-label leading-4 text-fg-secondary"><input type="checkbox" checked={partialDataset} onChange={(event) => setPartialDataset(event.target.checked)} />Partial export or completeness not established</label>
              <button className="analysis-button w-fit" type="button" disabled={Boolean(status) || !sourcePath.trim() || !displayName.trim()} onClick={() => void run('Importing table', () => invokeCommand('import_analytical_dataset', {
                input: {
                  datasetKind,
                  displayName,
                  sourceUri: sourceUri.trim() || null,
                  localPath: sourcePath,
                  observationPeriodStart: observationStart || null,
                  observationPeriodEnd: observationEnd || null,
                  partialDataset,
                  worksheet: worksheet.trim() || null,
                  sourceFileName: selectedCandidate?.fileName ?? null,
                  expectedSourceSha256: selectedCandidate?.sourceSha256 ?? null,
                },
              }), setImportResult)}>
                Import table
              </button>
            </div>
            <div className="mt-4 grid grid-cols-[minmax(0,1fr)] gap-2 border-t border-line pt-3">
              <p className="text-label leading-relaxed text-muted">Fetch a published City or SANDAG table directly. A Get It Done closed-year file is larger than one import: it is kept either for the Encampment service or for the reports already seen in an imported open copy, and the snapshot records the publisher file’s digest beside the subset’s.</p>
              <div className="flex gap-2">
                <select aria-label="City dataset" value={cityDataset} onChange={(event) => setCityDataset(event.target.value as CityDataset)} className="analysis-input min-w-0 flex-1">
                  {cityDatasets.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
                </select>
                {cityDatasetIsYearly && <input aria-label="City dataset year" type="number" className="analysis-input w-24" value={cityYear} onChange={(event) => setCityYear(event.target.value)} />}
              </div>
              <button className="analysis-button w-fit" type="button" disabled={Boolean(status) || (cityDatasetIsYearly && !/^\d{4}$/.test(cityYear))} onClick={() => void run('Fetching City dataset', () => invokeCommand('fetch_city_dataset', {
                dataset: cityDataset,
                year: cityDatasetIsYearly ? Number(cityYear) : null,
              }), setImportResult)}>
                Fetch and import
              </button>
            </div>
            {importResult && (
              <div className="mt-2 text-label leading-4 text-muted" role="status">
                <p>{importResult.importedRows.toLocaleString()} rows imported to {importResult.tableName}; {importResult.rejectedRows.toLocaleString()} rejected.{importResult.unconvertedCells > 0 && ` ${importResult.unconvertedCells.toLocaleString()} cells did not convert and are kept as written in the row ledger.`}{importResult.datasetKind !== 'REGIONAL_DATA' && ` ${importResult.duplicateKeyRows.toLocaleString()} duplicate keys.`}</p>
                {importResult.schemaWarnings.map((warning, index) => <p key={index} className="mt-1 text-warning">{warning}</p>)}
              </div>
            )}
          </Panel>
          <DataPackImport onImported={() => void refresh().catch((reason) => setError(String(reason)))} />
        </section>

        <section className="space-y-4">
          {schema && <Panel title="Data discovery" icon="sparkles">
            <DataDiscovery schema={schema} onInspect={(query) => {
              setSql(query);
              setSqlResult(null);
              // No request is fired here, but an in-flight inspection or manual run must not
              // repopulate `sqlResult` for the query the user just queued instead of ran.
              sqlGate.invalidate();
              window.requestAnimationFrame(() => document.getElementById('analytical-sql-editor')?.focus());
            }} />
          </Panel>}
          <Panel title="Hybrid Retrieval" icon="search">
            <p className="mb-2 text-label leading-4 text-muted">Search extracted text across reports, budgets, minutes, and articles. Exact search requires every word in the document; excerpts show the closest matching passage. Semantic matches are added when the model is ready. Text indexing covers the first four million characters per document.</p>
            <div className="grid gap-2 md:grid-cols-[minmax(0,1fr)_220px_auto]">
              <input aria-label="Document evidence query" className="analysis-input" value={hybridQuery} onChange={(event) => setHybridQuery(event.target.value)} />
              <select aria-label="Source jurisdiction" className="analysis-input" value={jurisdiction} onChange={(event) => setJurisdiction(event.target.value)}>
                <option value="">All source jurisdictions</option>
                <option>City of San Diego</option>
                <option>San Diego County</option>
                <option>San Diego County Sheriff</option>
                <option>San Diego County District Attorney</option>
                <option>San Diego Housing Commission</option>
                <option>SANDAG / SanGIS</option>
                <option>California</option>
                <option>United States</option>
                <option>City of Escondido</option>
                <option>San Diego Unified School District</option>
                <option>Poway Unified School District</option>
                <option>San Diego Metropolitan Transit System</option>
                <option>San Diego County Office of Education</option>
                <option>San Diego Air Pollution Control District</option>
                <option>UC San Diego</option>
                <option>San Diego State University</option>
                <option>San Diego regional organizations</option>
                <option>Research and policy organizations</option>
                <option>News publishers</option>
                <option>Multi-publisher catalogs</option>
                <option>Independent dashboards</option>
                <option>Unclassified source</option>
              </select>
              <button className="analysis-button" type="button" onClick={() => {
                const request = hybridSearchGate.begin();
                void run(
                  'Searching',
                  () => invokeCommand('hybrid_search', {
                    query: hybridQuery,
                    filter: { jurisdiction: jurisdiction || null },
                    limit: 20,
                  }),
                  (results) => { if (hybridSearchGate.isCurrent(request)) setHybridResults(results); },
                  () => hybridSearchGate.isCurrent(request),
                );
              }}>
                Search
              </button>
            </div>
            <div className="mt-3 divide-y divide-line">
              {hybridResults.map((result) => (
                <div key={result.stableId} className="py-3">
                  <div className="flex flex-wrap items-center gap-2 text-caption text-muted">
                    <span className="font-mono">{result.portalKey} / doc {result.documentId || 'metadata'}</span>
                    <span>{result.agencyName}</span>
                    <span>{result.jurisdiction}</span>
                    <span>{result.retrievalStatus}</span>
                    {result.semanticScore !== null && <span>cos {result.semanticScore.toFixed(3)}</span>}
                  </div>
                  <h3 className="mt-1 text-label leading-4 font-semibold text-fg">{result.fileName}{result.title && ` · ${result.title}`}</h3>
                  <p className="mt-1 whitespace-pre-wrap text-label leading-relaxed text-fg-secondary">{result.excerpt}</p>
                  <div className="mt-1 font-mono text-caption text-muted">source chars {result.characterStart}-{result.characterEnd} · detected personal identifiers masked</div>
                  <button type="button" className="analysis-button mt-2" onClick={() => reviewSearchResult(result)}>Review source record</button>
                </div>
              ))}
              {hybridResults.length === 0 && <p className="py-4 text-label text-muted">Search acquired documents after text extraction. Try a few distinctive terms such as “shelter capacity” or “contract amendment”. A missing match does not establish that the source lacks the information.</p>}
            </div>
          </Panel>

          {schema && <Panel title="Budget versus actuals" icon="database">
            <BudgetComparison schema={schema} onCompare={(query) => {
              setSql(query);
              void runGatedSql('Comparing budget and actuals', query);
            }} />
          </Panel>}

          {schema && <Panel title="Facility coverage" icon="database">
            <FacilityCoveragePanel schema={schema} />
          </Panel>}

          <EncampmentOrdinancePanel />

          <Panel title="SQL Analysis" icon="database">
            <div className="grid gap-3 xl:grid-cols-[280px_minmax(0,1fr)]">
              <div className="space-y-3">
                <div>
                  <div className="analysis-label">Datasets</div>
                  <div className="mt-1 max-h-40 overflow-y-auto rounded border border-line">
                    {(schema?.datasets ?? []).map((dataset) => (
                      <button key={dataset.snapshotId} type="button" onClick={() => { setSql(`SELECT * FROM ${dataset.tableName} LIMIT 50`); setSqlResult(null); sqlGate.invalidate(); }} className="block w-full border-b border-control px-2 py-1.5 text-left text-label text-fg-secondary last:border-b-0 hover:bg-surface">
                        <span className="font-semibold">{dataset.displayName}</span>
                        <span className="block font-mono text-caption text-muted">{dataset.tableName}</span>
                        <span className="block text-muted">{dataset.importedRows.toLocaleString()} rows · {dataset.rejectedRows.toLocaleString()} rejected · {dataset.partialDataset ? 'Partial / unverified coverage' : 'Declared complete'}</span>
                        <span className="block break-all text-muted">{dataset.sourceUri ?? 'Local export'} · SHA-256 {dataset.sourceSha256.slice(0, 16)}</span>
                        {(dataset.observationPeriodStart || dataset.observationPeriodEnd) && <span className="block text-muted">Observed {dataset.observationPeriodStart ?? '?'} – {dataset.observationPeriodEnd ?? '?'}</span>}
                      </button>
                    ))}
                  </div>
                </div>
                <TemplateList templates={templates} onChoose={(query) => { setSql(query); setSqlResult(null); sqlGate.invalidate(); }} />
              </div>
              <div className="min-w-0">
                {/* Unlike the dataset/template presets and DataDiscovery's onInspect (which jump to an
                    entirely different canned query and clear the old table immediately), a manual edit
                    is iterative refinement of the query the visible result already belongs to, so the
                    table is left showing until the user reruns it — standard SQL-console behavior. It
                    still invalidates the gate so a still-in-flight older request can't silently attach
                    its result to this now-different, unrun text. */}
                <textarea id="analytical-sql-editor" aria-label="Analytical SQL" className="analysis-sql" value={sql} onChange={(event) => { setSql(event.target.value); sqlGate.invalidate(); }} placeholder="SELECT ... FROM analysis_*" />
                <div className="mt-2 flex gap-2">
                  <button className="analysis-button" type="button" disabled={!sql.trim()} onClick={() => void runGatedSql('Running SQL', sql)}>
                    Run
                  </button>
                  <button className="analysis-button" type="button" disabled={!sql.trim()} onClick={() => void run('Exporting SQL', async () => {
                    const outputPath = await save({ defaultPath: 'onrecord-analysis-export.csv', filters: [{ name: 'CSV', extensions: ['csv'] }] });
                    if (outputPath) setExportResult(await invokeCommand('export_analytical_sql_csv', { sql, outputPath, limit: 10000 }));
                  })}>
                    Export CSV
                  </button>
                </div>
                {sqlResult && <SqlTable result={sqlResult} />}
                {exportResult && <p className="mt-2 break-all text-label leading-4 text-muted">Exported {exportResult.rowCount.toLocaleString()} rows to {exportResult.outputPath}. Export limit: 10,000 rows.</p>}
              </div>
            </div>
          </Panel>

          {groupedColumns.length > 0 && (
            <Panel title="Schema Browser" icon="sliders">
              <div className="grid gap-2 md:grid-cols-2">
                {groupedColumns.map(([table, columns]) => (
                  <div key={table} className="rounded border border-line p-2">
                    <div className="font-mono text-label text-accent">{table}</div>
                    <div className="mt-1 flex flex-wrap gap-1">
                      {columns.map((column) => <span key={`${table}-${column.columnName}`} className="rounded bg-surface px-1.5 py-0.5 text-caption text-fg-secondary">{column.columnName}:{column.logicalType}</span>)}
                    </div>
                  </div>
                ))}
              </div>
            </Panel>
          )}
        </section>
      </div>
      {(status || error) && (
        <div className="fixed bottom-4 right-4 z-50 rounded border border-line bg-canvas px-3 py-2 text-label text-fg shadow-xl">
          {error ? <span className="text-danger">{error}</span> : status}
        </div>
      )}
    </main>
  );
}

function Panel({ title, icon, children }: { title: string; icon: Parameters<typeof Icon>[0]['name']; children: React.ReactNode }) {
  return (
    <section className="app-panel p-4">
      <div className="mb-3 flex items-center gap-2">
        <Icon name={icon} className="h-4 w-4 text-accent" />
        <h2 className="text-sm font-semibold text-fg">{title}</h2>
      </div>
      {children}
    </section>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return <div className="flex justify-between gap-3 border-b border-line pb-1"><span className="text-muted">{label}</span><span className="min-w-0 truncate text-right font-mono">{value}</span></div>;
}

function TemplateList({ templates, onChoose }: { templates: SqlTemplate[]; onChoose: (sql: string) => void }) {
  return (
    <div>
      <div className="analysis-label">Templates</div>
      <div className="mt-1 space-y-1">
        {templates.map((template) => (
          <button key={template.templateId} type="button" onClick={() => onChoose(template.sql)} title={template.denominator} className="block w-full rounded border border-control px-2 py-1.5 text-left text-label text-fg-secondary hover:bg-surface">
            <span className="font-semibold">{template.title}</span>
            <span className="mt-0.5 line-clamp-2 block text-caption text-muted">{template.description}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function SqlTable({ result }: { result: SqlQueryResult }) {
  return (
    <div className="mt-3 overflow-hidden rounded border border-line">
      <div className="flex justify-between border-b border-line px-2 py-1 text-caption text-muted">
        <span>{result.rowCount.toLocaleString()} rows{result.truncated ? ' shown, truncated' : ''}</span>
        <span>{result.elapsedMs} ms</span>
      </div>
      <div className="max-h-80 overflow-auto">
        <table className="min-w-full border-collapse text-left text-label">
          <thead className="sticky top-0 bg-canvas">
            <tr>{result.columns.map((column) => <th key={column} className="border-b border-line px-2 py-1 font-semibold text-fg-secondary">{column}</th>)}</tr>
          </thead>
          <tbody>
            {result.rows.map((row, index) => (
              <tr key={index} className="odd:bg-surface/40">
                {row.map((cell, cellIndex) => <td key={cellIndex} className="max-w-[260px] truncate border-b border-line px-2 py-1 text-fg-secondary">{cell === null ? '' : String(cell)}</td>)}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
