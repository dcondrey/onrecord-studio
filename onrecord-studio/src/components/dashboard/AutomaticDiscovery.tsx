import { useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import { useFirstRunDataStore } from '../../store/useFirstRunDataStore';
import type { AnalyticalDatasetSummary as CityDatasetSummary, AnalyticalSchema, CityDataset } from '../../types';
import { sleeperBeatCounts, sleeperBeatRowsSql } from '../../utils/callLeads';
import PlaceMap, { type PlaceBoundary } from '../charts/figures/PlaceMap';
import { quoteIdentifier } from '../../utils/dataDiscovery';
import { dailyCountsOf, type DailyCounts } from '../../utils/serviceAccountability';
import CalendarHeat from '../charts/figures/CalendarHeat';
import LeadFigure from '../charts/figures/LeadFigure';
import type { ChartSources } from '../charts/frame/SourceCaption';
import { metadataObservations, profileObservations, rankObservations, recurringObservations, type DiscoveryObservation } from '../../utils/automaticDiscovery';
import type { DataGap } from '../../utils/dataGaps';
import { focusSourceComposer } from '../../utils/sanDiego';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import FamilyBadge from '../ui/FamilyBadge';
import Panel, { NestedCard, PanelBody, PanelHeader } from '../ui/Panel';
import Skeleton from '../ui/Skeleton';
import StatusDot from '../ui/statusDot';
import DataMaintenance from './DataMaintenance';
import InsightCard from './InsightCard';
import RecordGaps from './RecordGaps';
import { recordGaps } from '../../utils/recordGaps';

/** The companion tables a calls-for-service or stop-records import can be missing, by name. */
const COMPANION_TITLE: Partial<Record<CityDataset, string>> = {
  DISPOSITION_CODES: 'Disposition code list for calls for service',
  BEAT_CODES: 'Beat list for calls for service',
  CALL_TYPES: 'Call type list for calls for service',
  RIPA_STOP_REASONS: 'Reasons for each stop (RIPA)',
  RIPA_STOP_RESULTS: 'Results of each stop (RIPA)',
  RIPA_PROPERTY_SEIZURE_BASIS: 'Basis for each property seizure (RIPA)',
};

const ERROR_LABEL: Record<string, string> = {
  schema: 'Table checks',
  candidates: 'Acquired tables',
  patterns: 'Document patterns',
  maintenance: 'Storage upkeep',
  leads: 'Leads',
  discovery: 'Discovery',
};

/** The daily-count query each lead family's tables run, and how its calendar names what it counts. */
const DAILY_QUERY: Partial<Record<NonNullable<DiscoveryObservation['family']>, { id: string; title: string; noun: string }>> = {
  POLICE_CALLS: { id: 'calls_daily', title: 'SLEEPER calls by day', noun: 'SLEEPER calls' },
  ABATEMENT_LOGS: { id: 'abatement_daily', title: 'Abatements by day', noun: 'abatements' },
  PARKING_CITATIONS: { id: 'parking_daily', title: 'Oversized-vehicle and 72-hour citations by day', noun: 'citations' },
};

/** The latest imported table of police beat polygons: a beat code beside each feature's geometry. */
function beatBoundaryTable(schema: AnalyticalSchema | null | undefined): { table: CityDatasetSummary; crs: boolean } | null {
  if (!schema) return null;
  const has = (table: CityDatasetSummary, name: string) => schema.columns.some((column) => column.tableName === table.tableName && column.columnName === name);
  const table = schema.datasets.filter((dataset) => has(dataset, 'beat') && has(dataset, '_geometry'))
    .sort((a, b) => b.retrievedAt.localeCompare(a.retrievedAt) || a.snapshotId.localeCompare(b.snapshotId))[0];
  return table ? { table, crs: has(table, '_crs') } : null;
}

type BoundaryState = { status: 'loading' } | { status: 'ready'; boundaries: PlaceBoundary[] } | { status: 'failed'; message: string };

/**
 * SLEEPER calls per beat over the beat polygons. The polygons are read only when the map is
 * opened, and a read that resolves after the beat table changed is discarded: it would draw one
 * file's boundaries under another's counts.
 */
export function BeatMap({ boundary, counts, noBeat, citywide, sources, onActivate }: {
  boundary: { table: CityDatasetSummary; crs: boolean };
  counts: { beat: string; calls: number }[];
  noBeat: number;
  /** The beats query's own citywide total, the denominator the ranked lead states. */
  citywide: number;
  sources: ChartSources;
  onActivate?(beat: string): void;
}) {
  const [state, setState] = useState<BoundaryState>({ status: 'loading' });
  const gate = useLatestRequestGate();
  const { table, crs } = boundary;
  useEffect(() => {
    const request = gate.begin();
    setState({ status: 'loading' });
    invokeCommand('run_analytical_sql', { sql: `SELECT CAST(beat AS VARCHAR) AS beat, _geometry${crs ? ', _crs' : ''} FROM ${quoteIdentifier(table.tableName)} WHERE _geometry IS NOT NULL`, limit: 5000 })
      .then((result) => {
        if (!gate.isCurrent(request)) return;
        if (result.truncated) {
          setState({ status: 'failed', message: 'The beat file has more shapes than one read returns, so no partial map is drawn.' });
          return;
        }
        const at = (name: string) => result.columns.indexOf(name);
        setState({ status: 'ready', boundaries: result.rows.map((row) => ({ key: String(row[at('beat')]).trim(), _geometry: row[at('_geometry')], _crs: crs ? row[at('_crs')] : undefined })) });
      })
      .catch((reason: unknown) => {
        if (!gate.isCurrent(request)) return;
        setState({ status: 'failed', message: formatNativeError(reason) });
      });
    return gate.invalidate;
  }, [gate, table.tableName, table.snapshotId, crs]);
  const top = counts[0];
  return (
    <PlaceMap
      title="SLEEPER calls by police beat, on the beat map"
      headline={top && citywide ? `Beat ${top.beat}: ${top.calls.toLocaleString('en-US')} of ${citywide.toLocaleString('en-US')} SLEEPER calls` : null}
      noun="SLEEPER calls"
      places={counts.map((item) => ({ key: item.beat, label: `Beat ${item.beat}`, count: item.calls }))}
      unplaced={noBeat}
      total={citywide}
      onActivate={onActivate}
      boundaries={state.status === 'ready' ? state.boundaries : null}
      error={state.status === 'failed' ? state.message : null}
      sources={{ ...sources, tables: [...sources.tables, table.displayName] }}
      size="card"
    />
  );
}

/** Two gaps for one table differ only in the companion they need, so the title names it. */
function gapTitle(gap: DataGap): string {
  const companion = gap.fetch && (gap.key.startsWith('calls:') || gap.key.startsWith('stops:')) ? COMPANION_TITLE[gap.fetch.dataset] : undefined;
  return companion ?? gap.title;
}

/** The controller records `String(error)`; the reader needs the message without its class name. */
function readableError(message: string): string {
  const marker = message.indexOf('Error: ');
  const prefix = marker >= 0 ? message.slice(0, marker) : '';
  const text = marker >= 0 && !prefix.includes(' ') ? message.slice(marker + 'Error: '.length) : message;
  return text.trim().endsWith('.') ? text.trim().slice(0, -1) : text.trim();
}

type DiscoveryState = NonNullable<ReturnType<typeof useAutomaticDiscovery>['state']>;

/** Every observation the completed checks support: table metadata, each profile, and the leads read across tables. */
export function discoveryObservations(state: DiscoveryState): DiscoveryObservation[] {
  const datasets = state.schema?.datasets ?? [];
  return [...datasets.flatMap((dataset) => [
    ...metadataObservations(dataset),
    ...(state.profiles[dataset.snapshotId] ? profileObservations(state.schema!, dataset, state.profiles[dataset.snapshotId]) : []),
  ]), ...(state.schema ? recurringObservations(state.schema, state.profiles) : [])];
}

/**
 * A lead's figure names every table its query read: its own first, then each one the SQL quotes or
 * the lead says it read beside it.
 */
export function discoveryLeadSources(item: DiscoveryObservation, dataset: CityDatasetSummary, datasets: readonly CityDatasetSummary[]): ChartSources {
  return {
    tables: [dataset, ...datasets.filter((other) => other.snapshotId !== dataset.snapshotId
      && (item.reads?.includes(other.snapshotId) || Boolean(item.query?.sql.includes(quoteIdentifier(other.tableName)))))].map((table) => table.displayName),
    snapshot: { name: dataset.displayName, id: dataset.snapshotId, sha256: dataset.sourceSha256 },
    retrievedAt: dataset.retrievedAt,
  };
}

export default function AutomaticDiscovery({ embedded = false }: {
  /** Inside a canvas module that already names it: no panel chrome or title of its own. */
  embedded?: boolean;
} = {}) {
  const { controller, state } = useAutomaticDiscovery();
  const [showAll, setShowAll] = useState(false);
  // Maps whose disclosure is open; a closed map reads no boundaries.
  const [openMaps, setOpenMaps] = useState<ReadonlySet<string>>(new Set());
  const store = useAppStore;
  const inAnalysis = useAppStore((app) => app.workspaceView === 'analysis');
  const fetchingCoreData = useFirstRunDataStore((first) => first.running);
  if (!state || !controller) return null;
  const datasets = state.schema?.datasets ?? [];
  // A schema that has not loaded, or failed to load, is not an empty library: offering the core
  // fetch then would re-download tables the library may already hold.
  const emptyLibrary = state.schema?.datasets.length === 0;
  const observations = discoveryObservations(state);
  // Facts about a table (its coverage, gaps, record shares) are not findings about its subject;
  // they stay available but folded away beneath the leads.
  const leads = observations.filter((item) => item.kind === 'lead');
  const ranked = rankObservations(leads, showAll ? 30 : 6);
  const notes = rankObservations(observations.filter((item) => item.kind !== 'lead'), 30);
  // Byte-identical extracted text is a corpus/ingestion fact (the same document released more
  // than once), not an observation about the investigation's subject; it stays off this feed.
  const standoutPatterns = state.patterns.filter((pattern) => pattern.kind !== 'EXACT_DUPLICATE');
  // Collapsed, leads and patterns together fill six cards: whole rows at two columns and at three.
  const shownPatterns = standoutPatterns.slice(0, showAll ? 8 : Math.max(0, 6 - ranked.length));
  const hiddenCount = (showAll ? 0 : leads.length - ranked.length + standoutPatterns.length - shownPatterns.length);
  const gaps = controller.gaps();
  const records = state.schema ? recordGaps(state.schema, leads, state.profiles) : [];
  const automatic = Boolean(state.maintenance?.policy.enabled && state.maintenance.policy.autoRefresh);
  const profileErrors = Object.values(state.profiles).flatMap((profile) => profile.results.filter((entry) => entry.error));
  const checked = Object.values(state.profiles).filter((profile) => profile.results.some((entry) => entry.result)).length;
  const checking = !state.checkedAt || Boolean(state.activeSnapshot);
  const stage = (url: string) => { store.getState().setPortalUrlDraft(url); focusSourceComposer(); };
  const openAnalysis = () => store.getState().setWorkspaceView('analysis');
  const getCoreData = () => { void useFirstRunDataStore.getState().run(controller); store.getState().setWorkspaceView('overview'); };
  const leadSources = (item: DiscoveryObservation, dataset: CityDatasetSummary) => discoveryLeadSources(item, dataset, datasets);
  // A family's daily counts are drawn once per table, under the first of its leads shown.
  const calendarOwner = new Map<string, { title: string; noun: string; daily: DailyCounts }>();
  const claimed = new Set<string>();
  for (const item of ranked) {
    const daily = item.family ? DAILY_QUERY[item.family] : undefined;
    const results = state.profiles[item.snapshotId]?.results;
    if (!daily || !results || claimed.has(`${item.snapshotId}:${daily.id}`)) continue;
    const counts = dailyCountsOf(results, daily.id);
    if (!counts) continue;
    claimed.add(`${item.snapshotId}:${daily.id}`);
    calendarOwner.set(item.id, { title: daily.title, noun: daily.noun, daily: counts });
  }
  const boundary = beatBoundaryTable(state.schema);
  // The beats lead maps every beat's count, read from the same result as its ranked list.
  const beatMapOf = (item: DiscoveryObservation) => {
    if (!boundary || !item.id.endsWith(':calls_sleeper_beats')) return null;
    const counts = sleeperBeatCounts(state.profiles[item.snapshotId]?.results ?? []);
    const sql = item.query?.sql;
    // A beat's circle opens that beat's calls, selected exactly as the lead's query selects them.
    const onActivate = sql ? (beat: string) => {
      const rows = sleeperBeatRowsSql(sql, beat);
      if (rows) { controller.inspect(rows); openAnalysis(); }
    } : undefined;
    return counts ? { boundary, counts: counts.beats, noBeat: counts.noBeat, citywide: counts.citywide, onActivate } : null;
  };
  const card = (item: DiscoveryObservation) => {
    const dataset = datasets.find((candidate) => candidate.snapshotId === item.snapshotId)!;
    const spending = item.family === 'SPENDING_RECORDS' || dataset.datasetKind === 'OPERATING_BUDGET' || dataset.datasetKind === 'BUDGET_DIMENSION';
    return (
      <InsightCard
        key={item.id}
        nested
        badges={
          item.kind === 'lead' ? <FamilyBadge family={spending ? 'Spending' : 'Records request'} />
            : item.kind === 'pattern' ? <FamilyBadge family="Pattern" />
              : item.kind === 'quality' ? <Badge tone="neutral">Data quality</Badge>
                : <FamilyBadge family="Data gap" />
        }
        headline={item.title}
        body={<>
          {item.figure ? (
            <>
              <LeadFigure figure={item.figure} headline={null} size="card" sources={leadSources(item, dataset)} onActivate={item.query ? () => { controller.inspect(item.query!.sql); openAnalysis(); } : undefined} />
              <details className="text-body">
                <summary className="focus-ring w-fit cursor-pointer rounded text-caption text-muted hover:text-fg">Figures as text</summary>
                <p className="mt-1">{item.detail}</p>
              </details>
            </>
          ) : <p>{item.detail}</p>}
          {beatMapOf(item) && (
            <details className="text-body" onToggle={(event) => {
              const open = event.currentTarget.open;
              setOpenMaps((current) => {
                const next = new Set(current);
                if (open) next.add(item.id);
                else next.delete(item.id);
                return next;
              });
            }}>
              <summary className="focus-ring w-fit cursor-pointer rounded text-caption text-muted hover:text-fg">Map</summary>
              {openMaps.has(item.id) && <div className="mt-2"><BeatMap {...beatMapOf(item)!} sources={leadSources(item, dataset)} /></div>}
            </details>
          )}
          {calendarOwner.get(item.id) && (
            <details className="text-body">
              <summary className="focus-ring w-fit cursor-pointer rounded text-caption text-muted hover:text-fg">By day</summary>
              <div className="mt-2">
                <CalendarHeat {...calendarOwner.get(item.id)!} sources={leadSources(item, dataset)} size="card" />
              </div>
            </details>
          )}
          <p className="text-caption text-lead">Investigate: {item.question}</p>
          <details className="text-caption text-muted">
            <summary className="focus-ring w-fit cursor-pointer rounded hover:text-fg">Evidence and method</summary>
            <div className="mt-1 space-y-0.5 break-all font-mono text-code">
              <p className="select-all">Snapshot {dataset.snapshotId}</p>
              <p className="select-all">SHA-256 {dataset.sourceSha256}</p>
            </div>
            {item.query && <p className="mt-1">{item.query.method}</p>}
          </details>
        </>}
        subject={<span title={dataset.sourceUri ?? 'Local export'}>{dataset.displayName}{dataset.partialDataset ? ' · Partial or unverified' : ''}</span>}
        actions={item.query && <Button size="sm" onClick={() => { controller.inspect(item.query!.sql); openAnalysis(); }}>Explore this finding</Button>}
      />
    );
  };
  const body = (
    <>
        {(state.paused || (checking && datasets.length > 0)) && (
          <p role="status" className="text-caption tabular-nums text-muted">
            {state.paused ? 'Automatic checks are waiting for your current analysis.'
              : `Checked ${checked.toLocaleString()} of ${datasets.length.toLocaleString()} ${datasets.length === 1 ? 'table' : 'tables'}`}
          </p>
        )}
        {Object.keys(state.errors).length > 0 && (
          // The cause is stated once, by the page's own error; here only its consequence, with the
          // transport text folded away for a bug report.
          <div role="status" className="text-caption text-muted">
            <p>
              {Object.keys(state.errors).map((key) => ERROR_LABEL[key] ?? 'Table checks').join(', ')} could not run. Earlier results may be out of date.
            </p>
            <details className="mt-1">
              <summary className="focus-ring w-fit cursor-pointer rounded text-caption text-muted hover:text-fg">Technical details</summary>
              {Object.entries(state.errors).map(([key, error]) => (
                <p key={key} className="mt-1 break-words font-mono text-code">{readableError(error)}</p>
              ))}
            </details>
          </div>
        )}
        {profileErrors.length > 0 && (
          <details className="text-body text-warning">
            <summary className="focus-ring w-fit cursor-pointer rounded tabular-nums">{profileErrors.length} {profileErrors.length === 1 ? 'check needs' : 'checks need'} another attempt</summary>
            {profileErrors.map((entry, i) => <p key={i} className="mt-1">{entry.query.title}: {entry.error}</p>)}
            <p className="mt-1 text-muted">Other checks continue. Failed checks retry after a delay.</p>
          </details>
        )}
        {!state.checkedAt ? (
          <div className="grid items-stretch gap-3 md:grid-cols-2 2xl:grid-cols-3">
            <Skeleton variant="card" label="Reading your library" />
            {[1, 2].map((slot) => <div key={slot} aria-hidden="true"><Skeleton variant="card" label="" /></div>)}
          </div>
        ) : (ranked.length > 0 || shownPatterns.length > 0) && (
          <div className="space-y-2">
            <div className="grid items-stretch gap-3 md:grid-cols-2 2xl:grid-cols-3">
              {ranked.map(card)}
              {shownPatterns.map((pattern) => (
                <InsightCard
                  key={pattern.patternId}
                  nested
                  badges={<FamilyBadge family="Pattern" />}
                  headline={pattern.title}
                  body={<p>{pattern.explanation}</p>}
                  subject={`${pattern.evidence.length} cited passage${pattern.evidence.length === 1 ? '' : 's'}`}
                  actions={<Button size="sm" onClick={() => { const actions = store.getState(); actions.setActivePortal(null); actions.setPatternEvidenceSelection({ patternId: pattern.patternId, portalKey: pattern.evidence[0]?.portalKey ?? null, documentId: pattern.evidence[0]?.documentId ?? null, characterStart: pattern.evidence[0]?.characterStart ?? null }); actions.setWorkspaceView('patterns'); }}>Inspect document evidence</Button>}
                />
              ))}
            </div>
            {shownPatterns.length > 0 && <p className="text-caption text-muted">Shared language or entities across documents do not establish a causal connection.</p>}
          </div>
        )}
        {state.checkedAt && (showAll || hiddenCount > 0) && (
          <Button size="sm" variant="link" className="px-0" onClick={() => setShowAll((value) => !value)}>
            {showAll ? 'Show fewer' : `Show ${hiddenCount.toLocaleString()} more`}
          </Button>
        )}
        {gaps.length > 0 && (
          <section aria-labelledby="data-gaps-heading" className="space-y-2">
            <h4 id="data-gaps-heading" className="text-heading text-fg">Data gaps</h4>
            <ul className="divide-y divide-line rounded-lg border border-line">
              {gaps.map((gap) => {
                const status = state.gapStatus[gap.key];
                const working = status === 'fetching';
                const done = status === 'fetched';
                const failed = Boolean(status) && !working && !done;
                return (
                  <li key={gap.key} className="flex flex-wrap items-start gap-x-4 gap-y-2 p-3">
                    <div className="min-w-0 flex-1 space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <FamilyBadge family="Data gap" />
                        <span className="text-control text-fg">{gapTitle(gap)}</span>
                      </div>
                      <p className="text-body text-muted">{gap.detail}</p>
                      <p role="status" className="flex items-center gap-1.5 text-caption text-muted empty:hidden">
                        {working && <><StatusDot state="active" pulse label="In progress" />{gap.collect ? 'Starting the collection…' : 'Fetching…'}</>}
                        {done && (gap.collect
                          ? <><StatusDot state="active" label="Started" />Collection started; the source is listed in the sidebar.</>
                          : checking
                            ? <><StatusDot state="active" pulse label="Checks running" />Fetched; the checks are running again.</>
                            : <><StatusDot state="active" label="Done" />Fetched and checked.</>)}
                        {failed && <span className="text-danger">The last attempt failed: {readableError(status)}.</span>}
                        {!status && automatic && gap.fetch && 'It will be fetched automatically.'}
                      </p>
                    </div>
                    {!working && !done && (
                      <Button size="sm" onClick={() => controller.fill(gap.key)}>{gap.collect ? 'Collect now' : 'Fetch now'}</Button>
                    )}
                  </li>
                );
              })}
            </ul>
          </section>
        )}
        <RecordGaps gaps={records} />
        {state.candidates.length > 0 && (
          <p className="text-body tabular-nums text-fg-secondary">
            {state.candidates.length.toLocaleString()} acquired {state.candidates.length === 1 ? 'table is' : 'tables are'} available for analysis.
            {!inAnalysis && <>{' '}<Button variant="link" size="sm" className="h-auto px-0 align-baseline" onClick={openAnalysis}>Review acquired tables</Button></>}
          </p>
        )}
        {!datasets.length && !standoutPatterns.length && (
          <div className="grid gap-3 sm:grid-cols-2">
            <NestedCard className="flex flex-col gap-2">
              <h4 className="text-heading text-fg">Where does funding reach the community?</h4>
              <p className="text-body text-muted">Start with operating-budget records. Separate adopted allocations from proposals, then compare with actuals for matching years.</p>
              {emptyLibrary && <Button size="sm" className="mt-auto w-fit" disabled={fetchingCoreData} onClick={getCoreData}>Get San Diego’s core public data</Button>}
            </NestedCard>
            <NestedCard className="flex flex-col gap-2">
              <h4 className="text-heading text-fg">Do housing outcomes match the reported investment?</h4>
              <p className="text-body text-muted">Bring in performance reports and preserve each measure’s population and period. The app will surface connections as records arrive.</p>
              <Button size="sm" className="mt-auto w-fit" onClick={() => stage('https://www.rtfhsd.org/reports-data/')}>Choose housing reports</Button>
            </NestedCard>
            <p className="text-caption text-muted sm:col-span-2">These are starting questions, not findings about data that has not been imported.</p>
          </div>
        )}
        {datasets.length > 0 && !ranked.length && !state.activeSnapshot && state.checkedAt && (
          <p className="text-body text-muted">The completed checks found nothing to raise in these tables. Service-report checks need a table with a service type, a close date, and an opened date or recorded age.</p>
        )}
        {notes.length > 0 && (
          <details className="text-body text-muted">
            <summary className="focus-ring w-fit cursor-pointer rounded tabular-nums hover:text-fg">{notes.length} {notes.length === 1 ? 'note' : 'notes'} about the imported tables themselves</summary>
            <div className="mt-3 grid items-stretch gap-3 md:grid-cols-2 2xl:grid-cols-3">{notes.map(card)}</div>
          </details>
        )}
        <DataMaintenance />
    </>
  );
  if (embedded) {
    return (
      <div className="space-y-5">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="max-w-prose text-caption text-muted">What your tables and indexed documents show that is worth raising, each with the records behind it.</p>
          <Button size="sm" onClick={controller.refresh}>Refresh</Button>
        </div>
        {body}
      </div>
    );
  }
  return (
    <Panel aria-labelledby="discoveries-heading">
      <PanelHeader
        titleId="discoveries-heading"
        title="What stands out"
        description="What your tables and indexed documents show that is worth raising, each with the records behind it."
        actions={<Button size="sm" onClick={controller.refresh}>Refresh</Button>}
      />
      <PanelBody className="space-y-5">{body}</PanelBody>
    </Panel>
  );
}
