import { useEffect, useMemo, useState } from 'react';
import DifferenceDots from '../charts/figures/DifferenceDots';
import { budgetComparisonLineQuery, readBudgetComparison } from '../../utils/budgetComparison';
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
import { formatNativeError } from '../../ipc';
import Button from '../ui/Button';
import EmptyState from '../ui/EmptyState';
import { Field, Input, Select } from '../ui/Field';
import InlineError from '../ui/InlineError';
import PageHeader from '../ui/PageHeader';
import UiPanel, { PanelBody, PanelHeader } from '../ui/Panel';
import BudgetComparison from './BudgetComparison';
import DataDiscovery from './DataDiscovery';
import DataPackImport from './DataPackImport';
import FacilityCoveragePanel from './FacilityCoveragePanel';
import EncampmentOrdinancePanel from './EncampmentOrdinancePanel';
import { useAppStore } from '../../store/useAppStore';
import AutomaticDiscovery from './AutomaticDiscovery';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';

const DATASET_KIND_LABELS: Record<AnalyticalDatasetKind, string> = {
  REGIONAL_DATA: 'General regional data',
  RIPA_STOPS: 'Police stop records (RIPA)',
  RIPA_SEARCH_BASIS: 'Police stop search basis (RIPA)',
  CALLS_FOR_SERVICE: 'Police calls for service',
  CALLS_CODEBOOK: 'Calls for service code list',
  NIBRS_OFFENSES: 'Reported offenses (NIBRS)',
  OPERATING_BUDGET: 'Operating budget',
  BUDGET_DIMENSION: 'Budget reference table',
};

const datasetKinds = Object.keys(DATASET_KIND_LABELS) as AnalyticalDatasetKind[];

/** The commands that report on the embedding model and index; a failure of either is not "Checking". */
type StatusRead = 'pending' | 'ready' | 'failed';

/** One sentence for the reader; the transport messages stay under "Technical details". */
function workspaceErrorSummary(messages: string[]): string {
  return messages.some((message) => message.includes('DATABASE_UNAVAILABLE'))
    ? 'The local evidence store is busy. Try again in a moment.'
    : 'Part of the analysis workspace could not be loaded.';
}

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
  { value: 'CAPITAL_BUDGET', label: 'City capital improvements budget (all years)' },
  { value: 'CAPITAL_ACTUALS', label: 'City capital improvements actuals (all years)' },
  { value: 'CITY_PROPERTIES', label: 'City-owned properties (today)' },
  { value: 'CITY_PARCELS', label: 'City-owned property parcels (today)' },
  { value: 'CITY_LEASES', label: 'City property leases and permits (today)' },
  { value: 'CALL_TYPES', label: 'SDPD call types' },
  { value: 'DISPOSITION_CODES', label: 'SDPD call dispositions' },
  { value: 'BEAT_CODES', label: 'SDPD beats' },
  { value: 'BUDGET_ACCOUNTS', label: 'City budget accounts' },
  { value: 'BUDGET_DEPARTMENTS', label: 'City budget departments' },
  { value: 'BUDGET_FUNDS', label: 'City budget funds' },
  { value: 'PARKING_CITATIONS_FIRST_HALF', label: 'Parking citations, January to June (by year)', yearly: true },
  { value: 'PARKING_CITATIONS_SECOND_HALF', label: 'Parking citations, July to December (by year)', yearly: true },
  { value: 'FIRE_EMS_INCIDENTS', label: 'Fire-Rescue incidents (by year)', yearly: true },
  { value: 'APPROVALS_CREATED', label: 'Development approvals created (by year)', yearly: true },
  { value: 'APPROVALS_ISSUED', label: 'Development approvals issued (by year)', yearly: true },
  { value: 'APPROVALS_CLOSED', label: 'Development approvals closed (by year)', yearly: true },
  { value: 'TRAFFIC_COLLISIONS', label: 'SDPD traffic collisions (all years)' },
  { value: 'TRAFFIC_COLLISION_PEOPLE', label: 'SDPD traffic collisions, people involved (all years)' },
  { value: 'HATE_CRIMES', label: 'SDPD hate crimes' },
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
  const [modelRead, setModelRead] = useState<StatusRead>('pending');
  const [indexRead, setIndexRead] = useState<StatusRead>('pending');
  const [hybridQuery, setHybridQuery] = useState('homelessness spending shelter outcomes contract amendments');
  const [jurisdiction, setJurisdiction] = useState('');
  const [hybridResults, setHybridResults] = useState<HybridSearchResult[]>([]);
  const [schema, setSchema] = useState<AnalyticalSchema | null>(null);
  const [sql, setSql] = useState('');
  const [sqlResult, setSqlResult] = useState<SqlQueryResult | null>(null);
  // The query the shown result answers; the editor text can move on without a rerun.
  const [resultSql, setResultSql] = useState('');
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
    setModelRead(model.status === 'fulfilled' ? 'ready' : 'failed');
    setIndexRead(index.status === 'fulfilled' ? 'ready' : 'failed');
    if (analytical.status === 'fulfilled') setSchema(analytical.value);
    if (acquired.status === 'fulfilled') setCandidates(acquired.value);
    const failures = [...new Set(results.flatMap((result) => (result.status === 'rejected' ? [formatNativeError(result.reason)] : [])))];
    if (failures.length) setError(failures.join('\n'));
  };

  const retryRefresh = () => {
    setError(null);
    void refresh().catch((reason) => setError(formatNativeError(reason)));
  };

  useEffect(() => {
    void refresh().catch((reason) => setError(formatNativeError(reason)));
  }, []);

  const inspection = automatic?.inspection;
  useEffect(() => {
    if (!inspection) return;
    // `active` tracks only whether *this effect instance* is still mounted, so it governs the
    // loading indicator: that must clear when this specific fetch settles, even after `sqlGate`
    // has stopped trusting this fetch's *data*, or the indicator and the Import button stay stuck.
    let active = true;
    const request = sqlGate.begin();
    const release = discovery?.hold();
    setSql(inspection.sql);
    setSqlResult(null);
    setStatus('Loading supporting records');
    void invokeCommand('run_analytical_sql', { sql: inspection.sql, limit: 500 }).then((result) => {
      if (!sqlGate.isCurrent(request)) return;
      setSqlResult(result);
      setResultSql(inspection.sql);
    }).catch((reason) => { if (sqlGate.isCurrent(request)) setError(formatNativeError(reason)); }).finally(() => { release?.(); if (active) setStatus(null); });
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
      if (isCurrent()) setError(formatNativeError(reason));
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
        if (!sqlGate.isCurrent(request)) return;
        setSqlResult(result);
        setResultSql(query);
      },
      () => sqlGate.isCurrent(request),
    );
  };

  const indexing = status === 'Indexing documents' || status === 'Rebuilding semantic index';
  const indexedDocuments = indexStatus?.indexedDocumentCount ?? null;
  const pendingDocuments = indexStatus?.pendingDocumentCount ?? null;
  const indexLine = indexRead === 'failed'
    ? 'Semantic search: index status unavailable'
    : indexStatus
      ? `Semantic search: ${indexStatus.indexedDocumentCount.toLocaleString()} of ${(indexStatus.indexedDocumentCount + indexStatus.pendingDocumentCount).toLocaleString()} documents indexed`
      : null;
  const unavailable = (read: StatusRead) => (read === 'failed' ? 'Unavailable' : 'Checking');

  return (
    <main className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-content space-y-6 px-4 pb-10 pt-6 md:px-6">
        <PageHeader
          title="Analysis"
          summary="Imported tables, document search and SQL over what you have collected."
        />
        {error && <InlineError summary={workspaceErrorSummary(error.split('\n'))} detail={error} onRetry={retryRefresh} />}
        {status && !error && <p role="status" className="text-caption text-muted">{status}</p>}
        <AutomaticDiscovery />
        <div className="grid gap-4 lg:grid-cols-[minmax(360px,0.9fr)_minmax(0,1.3fr)]">
        <section className="min-w-0 space-y-4">
          <UiPanel aria-labelledby="semantic-search-heading">
            <PanelHeader
              titleId="semantic-search-heading"
              title="Semantic search"
              description={indexLine ?? <span className="skeleton inline-block h-3 w-48 rounded align-middle"><span className="sr-only">Checking the index</span></span>}
              actions={
                modelRead === 'ready' && !modelStatus?.installed ? (
                  <Button variant="primary" size="sm" disabled={Boolean(status)} onClick={() => void run('Installing model', () => invokeCommand('initialize_embedding_model', {}), setModelStatus)}>Set up</Button>
                ) : pendingDocuments !== null && pendingDocuments > 0 ? (
                  <Button variant="primary" size="sm" disabled={Boolean(status)} onClick={() => void run('Indexing documents', () => invokeCommand('rebuild_embedding_index', { rebuild: false }), setIndexReport)}>
                    Index {pendingDocuments.toLocaleString()} new document{pendingDocuments === 1 ? '' : 's'}
                  </Button>
                ) : null
              }
            />
            <PanelBody dense>
              {indexReport && (
                <p className="mb-3 text-body tabular-nums text-muted">
                  Indexed {indexReport.indexedDocuments.toLocaleString()} documents, {indexReport.indexedChunks.toLocaleString()} chunks; skipped {indexReport.skippedDocuments.toLocaleString()}.
                </p>
              )}
              <details className="group">
                <summary className="focus-ring w-fit cursor-pointer rounded text-caption text-muted hover:text-fg">Technical details</summary>
                <dl className="mt-2 grid gap-1 text-caption">
                  <Metric label="Model" value={modelStatus ? modelStatus.modelId : unavailable(modelRead)} mono={Boolean(modelStatus)} />
                  <Metric label="State" value={modelStatus ? (modelStatus.ready ? 'Ready' : modelStatus.installed ? 'Installed' : 'Not installed') : unavailable(modelRead)} />
                  <Metric label="Passages" value={indexStatus ? indexStatus.indexedChunkCount.toLocaleString() : unavailable(indexRead)} />
                  <Metric label="Documents" value={indexedDocuments !== null && pendingDocuments !== null ? `${indexedDocuments.toLocaleString()} indexed, ${pendingDocuments.toLocaleString()} pending` : unavailable(indexRead)} />
                  {modelStatus?.artifactManifestSha256 && <Metric label="Manifest" value={modelStatus.artifactManifestSha256} mono />}
                </dl>
                <div className="mt-3 flex flex-wrap gap-2">
                  <Button size="sm" disabled={Boolean(status) || !modelStatus?.installed} onClick={() => void run('Rebuilding semantic index', () => invokeCommand('rebuild_embedding_index', { rebuild: true }), setIndexReport)}>
                    Rebuild index
                  </Button>
                  <Button size="sm" variant="ghost" disabled={!indexing} onClick={() => void run('Cancelling indexing', () => invokeCommand('cancel_embedding_index', {}))}>
                    Cancel indexing
                  </Button>
                </div>
              </details>
            </PanelBody>
          </UiPanel>

          <Panel title="Import a table" titleId="analysis-import-heading">
            <p className="mb-4 text-body text-muted">Choose an acquired table or a local CSV, TSV, JSON, GeoJSON, JSONL, XLSX, XLS or ODS export. General regional data infers numeric columns while preserving codes and suppression markers.</p>
            <div className="grid grid-cols-[minmax(0,1fr)] gap-3">
              <div className="flex items-end gap-2">
                <Select label="Acquired table" aria-label="Acquired table" value={selectedCandidate ? `${selectedCandidate.portalKey}:${selectedCandidate.documentId}` : ''} onChange={(event) => {
                  const candidate = candidates.find((item) => `${item.portalKey}:${item.documentId}` === event.target.value) ?? null;
                  setSelectedCandidate(candidate);
                  setSourcePath(candidate?.localPath ?? '');
                  setSourceUri(candidate?.sourceUri ?? '');
                  setDisplayName(candidate?.fileName ?? '');
                }}>
                  <option value="">Choose from the latest 500 acquired tables</option>
                  {candidates.map((candidate) => <option key={`${candidate.portalKey}:${candidate.documentId}`} value={`${candidate.portalKey}:${candidate.documentId}`}>{candidate.fileName} · {candidate.sourceUri ?? candidate.portalKey}</option>)}
                </Select>
                <Button variant="ghost" iconOnly icon="sync" aria-label="Refresh acquired tables" title="Refresh acquired tables" onClick={() => void run('Refreshing acquired tables', refresh)} />
              </div>
              <Select label="Dataset type" aria-label="Dataset type" value={datasetKind} onChange={(event) => setDatasetKind(event.target.value as AnalyticalDatasetKind)}>
                {datasetKinds.map((kind) => <option key={kind} value={kind}>{DATASET_KIND_LABELS[kind]}</option>)}
              </Select>
              <Field label="Dataset name">
                <Input aria-label="Dataset name" value={displayName} onChange={(event) => setDisplayName(event.target.value)} placeholder="e.g. Shelter beds by year" />
              </Field>
              <div className="flex items-end gap-2">
                <Field label="Table file" className="min-w-0 flex-1">
                  <Input aria-label="Table file path" value={sourcePath} onChange={(event) => { setSourcePath(event.target.value); setSelectedCandidate(null); }} placeholder="/path/to/local.csv" />
                </Field>
                <Button onClick={() => void run('Choosing table', async () => {
                  const path = await open({ multiple: false, filters: [{ name: 'Tables', extensions: ['csv', 'tsv', 'json', 'geojson', 'jsonl', 'ndjson', 'xlsx', 'xls', 'ods'] }] });
                  if (typeof path === 'string') { setSourcePath(path); setSelectedCandidate(null); setSourceUri(''); setDisplayName(path.split(/[\\/]/).pop() ?? 'Dataset'); }
                })}>Browse…</Button>
              </div>
              <Field label="Original source URL" help="Including FTP or SFTP export paths.">
                <Input aria-label="Original source URL" value={sourceUri} onChange={(event) => setSourceUri(event.target.value)} placeholder="https://data.sandiego.gov/…" />
              </Field>
              <Field label="Worksheet" help="Required for a workbook with several sheets.">
                <Input aria-label="Worksheet name" value={worksheet} onChange={(event) => setWorksheet(event.target.value)} placeholder="e.g. FY2025" />
              </Field>
              <div className="grid grid-cols-2 gap-3">
                <Field label="Observation start"><Input aria-label="Observation start" type="date" value={observationStart} onChange={(event) => setObservationStart(event.target.value)} /></Field>
                <Field label="Observation end"><Input aria-label="Observation end" type="date" value={observationEnd} onChange={(event) => setObservationEnd(event.target.value)} /></Field>
              </div>
              <label className="flex items-center gap-2 text-body text-fg-secondary"><input type="checkbox" className="accent-[rgb(var(--accent))]" checked={partialDataset} onChange={(event) => setPartialDataset(event.target.checked)} />Partial export or completeness not established</label>
              <Button variant="primary" className="w-fit" disabled={Boolean(status) || !sourcePath.trim() || !displayName.trim()} onClick={() => void run('Importing table', () => invokeCommand('import_analytical_dataset', {
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
              </Button>
            </div>
            <div className="mt-5 grid grid-cols-[minmax(0,1fr)] gap-3 border-t border-line pt-4">
              <h3 className="text-heading text-fg">Fetch a published table</h3>
              <p className="text-body text-muted">Fetch a City or SANDAG table directly. A Get It Done closed-year file is larger than one import: it is kept either for the Encampment service or for the reports already seen in an imported open copy, and the snapshot records the publisher file’s digest beside the subset’s.</p>
              <div className="flex items-end gap-2">
                <Select label="City dataset" aria-label="City dataset" value={cityDataset} onChange={(event) => setCityDataset(event.target.value as CityDataset)} className="min-w-0">
                  {cityDatasets.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
                </Select>
                {cityDatasetIsYearly && <Input aria-label="City dataset year" type="number" className="w-24 tabular-nums" value={cityYear} onChange={(event) => setCityYear(event.target.value)} />}
              </div>
              <Button className="w-fit" disabled={Boolean(status) || (cityDatasetIsYearly && !/^\d{4}$/.test(cityYear))} onClick={() => void run('Fetching City dataset', () => invokeCommand('fetch_city_dataset', {
                dataset: cityDataset,
                year: cityDatasetIsYearly ? Number(cityYear) : null,
              }), setImportResult)}>
                Fetch and import
              </Button>
            </div>
            {importResult && (
              <div className="mt-3 text-body text-muted" role="status">
                <p className="tabular-nums">{importResult.importedRows.toLocaleString()} rows imported to <code className="font-mono text-code">{importResult.tableName}</code>; {importResult.rejectedRows.toLocaleString()} rejected.{importResult.unconvertedCells > 0 && ` ${importResult.unconvertedCells.toLocaleString()} cells did not convert and are kept as written in the row ledger.`}{importResult.datasetKind !== 'REGIONAL_DATA' && ` ${importResult.duplicateKeyRows.toLocaleString()} duplicate keys.`}</p>
                {importResult.schemaWarnings.map((warning, index) => <p key={index} className="mt-1 text-warning">{warning}</p>)}
              </div>
            )}
          </Panel>
          <DataPackImport onImported={() => void refresh().catch((reason) => setError(formatNativeError(reason)))} />
        </section>

        <section className="space-y-4">
          {schema && schema.datasets.length === 0 && (
            <EmptyState
              icon="database"
              title="Import a table to explore it"
              body="Data discovery summarizes each imported table: its columns, coverage and the questions it can answer."
              actions={<Button onClick={() => document.getElementById('analysis-import-heading')?.scrollIntoView({ block: 'start' })}>Import a table</Button>}
            />
          )}
          {schema && schema.datasets.length > 0 && <Panel title="Data discovery">
            <DataDiscovery schema={schema} onInspect={(query) => {
              setSql(query);
              setSqlResult(null);
              // No request is fired here, but an in-flight inspection or manual run must not
              // repopulate `sqlResult` for the query the user just queued instead of ran.
              sqlGate.invalidate();
              window.requestAnimationFrame(() => document.getElementById('analytical-sql-editor')?.focus());
            }} />
          </Panel>}
          <Panel title="Search documents">
            <p className="mb-3 text-body text-muted">Search extracted text across reports, budgets, minutes, and articles. Exact search requires every word in the document; excerpts show the closest matching passage. Semantic matches are added when the model is ready. Text indexing covers the first four million characters per document.</p>
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
                  <div className="mt-1 text-caption tabular-nums text-muted">Characters {result.characterStart}–{result.characterEnd} · detected personal identifiers masked</div>
                  <button type="button" className="analysis-button mt-2" onClick={() => reviewSearchResult(result)}>Review source record</button>
                </div>
              ))}
              {hybridResults.length === 0 && <p className="py-4 text-label text-muted">Search acquired documents after text extraction. Try a few distinctive terms such as “shelter capacity” or “contract amendment”. A missing match does not establish that the source lacks the information.</p>}
            </div>
          </Panel>

          {schema && <Panel title="Budget versus actuals">
            <BudgetComparison schema={schema} onCompare={(query) => {
              setSql(query);
              void runGatedSql('Comparing budget and actuals', query);
            }} />
          </Panel>}

          {schema && <Panel title="Facility coverage">
            <FacilityCoveragePanel schema={schema} />
          </Panel>}

          <EncampmentOrdinancePanel />

          <Panel title="SQL analysis">
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
            {/* Full panel width: the result column beside the dataset list is too narrow for row labels. */}
            {sqlResult && <BudgetDifference result={sqlResult} onInspect={(line) => { setSql(budgetComparisonLineQuery(resultSql, line)); sqlGate.invalidate(); }} />}
          </Panel>

          {groupedColumns.length > 0 && (
            <Panel title="Schema browser">
              <div className="grid gap-2 md:grid-cols-2">
                {groupedColumns.map(([table, columns]) => (
                  <div key={table} className="rounded border border-line p-2">
                    <div className="font-mono text-code text-fg">{table}</div>
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
      </div>
    </main>
  );
}

function Panel({ title, titleId, children }: { title: string; titleId?: string; children: React.ReactNode }) {
  return (
    <UiPanel aria-labelledby={titleId}>
      <PanelHeader title={title} titleId={titleId} />
      <PanelBody>{children}</PanelBody>
    </UiPanel>
  );
}

function Metric({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex justify-between gap-3 border-b border-line pb-1">
      <dt className="text-muted">{label}</dt>
      <dd className={mono ? 'min-w-0 break-all text-right font-mono text-code text-fg-secondary' : 'min-w-0 text-right tabular-nums text-fg-secondary'}>{value}</dd>
    </div>
  );
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

/** The budget-comparison result as a difference chart, above the exact table; any other result shows the table alone. */
function BudgetDifference({ result, onInspect }: { result: SqlQueryResult; onInspect: (line: { fund: string | null; center: string | null; account: string | null }) => void }) {
  const lines = useMemo(() => readBudgetComparison(result), [result]);
  if (!lines) return null;
  const byKey = new Map(lines.map((line) => [line.key, line] as const));
  const compared = lines.flatMap((line) => (line.difference !== null && line.budget !== null && line.actuals !== null
    ? [{ key: line.key, label: line.label, context: line.departments, from: line.budget, to: line.actuals, difference: line.difference, percent: line.variancePercent }]
    : []));
  const uncompared = lines.filter((line) => line.difference === null || line.budget === null || line.actuals === null)
    .map((line) => ({ key: line.key, label: line.label, status: line.status === 'Matched' ? 'Amounts not readable' : line.status }));
  return (
    <div className="mt-3">
      <DifferenceDots
        title="Actuals minus budget, by account line"
        sources={{ tables: result.provenance.length ? result.provenance : ['run_analytical_sql result'], note: 'Matched lines only; positive means actuals exceed the selected allocation, which is not a finding' }}
        compared={compared}
        uncompared={uncompared}
        fromLabel="budget"
        toLabel="actual"
        truncated={result.truncated}
        onActivate={(key) => { const line = byKey.get(key); if (line) onInspect(line); }}
      />
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
      <div tabIndex={0} role="region" aria-label="Query results" className="focus-ring max-h-80 overflow-auto">
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
