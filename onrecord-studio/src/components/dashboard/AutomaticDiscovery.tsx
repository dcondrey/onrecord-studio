import { useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import { useFirstRunDataStore } from '../../store/useFirstRunDataStore';
import type { CityDataset } from '../../types';
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

export default function AutomaticDiscovery() {
  const { controller, state } = useAutomaticDiscovery();
  const [showAll, setShowAll] = useState(false);
  const store = useAppStore;
  const inAnalysis = useAppStore((app) => app.workspaceView === 'analysis');
  const fetchingCoreData = useFirstRunDataStore((first) => first.running);
  if (!state || !controller) return null;
  const datasets = state.schema?.datasets ?? [];
  // A schema that has not loaded, or failed to load, is not an empty library: offering the core
  // fetch then would re-download tables the library may already hold.
  const emptyLibrary = state.schema?.datasets.length === 0;
  const observations = [...datasets.flatMap((dataset) => [
    ...metadataObservations(dataset),
    ...(state.profiles[dataset.snapshotId] ? profileObservations(state.schema!, dataset, state.profiles[dataset.snapshotId]) : []),
  ]), ...(state.schema ? recurringObservations(state.schema, state.profiles) : [])];
  // Facts about a table (its coverage, gaps, record shares) are not findings about its subject;
  // they stay available but folded away beneath the leads.
  const leads = observations.filter((item) => item.kind === 'lead');
  const ranked = rankObservations(leads, showAll ? 30 : 6);
  const notes = rankObservations(observations.filter((item) => item.kind !== 'lead'), 30);
  // Byte-identical extracted text is a corpus/ingestion fact (the same document released more
  // than once), not an observation about the investigation's subject; it stays off this feed.
  const standoutPatterns = state.patterns.filter((pattern) => pattern.kind !== 'EXACT_DUPLICATE');
  const shownPatterns = standoutPatterns.slice(0, showAll ? 8 : 2);
  const gaps = controller.gaps();
  const records = state.schema ? recordGaps(state.schema, leads, state.profiles) : [];
  const automatic = Boolean(state.maintenance?.policy.enabled && state.maintenance.policy.autoRefresh);
  const profileErrors = Object.values(state.profiles).flatMap((profile) => profile.results.filter((entry) => entry.error));
  const checked = Object.values(state.profiles).filter((profile) => profile.results.some((entry) => entry.result)).length;
  const checking = !state.checkedAt || Boolean(state.activeSnapshot);
  const stage = (url: string) => { store.getState().setPortalUrlDraft(url); focusSourceComposer(); };
  const openAnalysis = () => store.getState().setWorkspaceView('analysis');
  const getCoreData = () => { void useFirstRunDataStore.getState().run(controller); store.getState().setWorkspaceView('overview'); };
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
          <p>{item.detail}</p>
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
  return (
    <Panel aria-labelledby="discoveries-heading">
      <PanelHeader
        titleId="discoveries-heading"
        title="What stands out"
        description="What your tables and indexed documents show that is worth raising, each with the records behind it."
        actions={<Button size="sm" onClick={controller.refresh}>Refresh</Button>}
      />
      <PanelBody className="space-y-5">
        <p role="status" className="text-caption tabular-nums text-muted empty:hidden">
          {state.paused ? 'Automatic checks are waiting for your current analysis.'
            : checking && datasets.length > 0 ? `Checked ${checked.toLocaleString()} of ${datasets.length.toLocaleString()} ${datasets.length === 1 ? 'table' : 'tables'}`
              : ''}
        </p>
        {Object.keys(state.errors).length > 0 && (
          <ul role="alert" className="space-y-1 rounded-lg border border-warning/30 bg-warning/10 px-4 py-3 text-body text-warning">
            {Object.entries(state.errors).map(([key, error]) => (
              <li key={key}>{ERROR_LABEL[key] ?? 'Table check'} could not run: {readableError(error)}. Earlier results may be out of date.</li>
            ))}
          </ul>
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
        {(leads.length > 6 || standoutPatterns.length > 2) && (
          <Button size="sm" variant="ghost" onClick={() => setShowAll((value) => !value)}>{showAll ? 'Show fewer' : 'Show more'}</Button>
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
                        {done ? <Badge tone="success">Fetched</Badge> : <FamilyBadge family="Data gap" />}
                        <span className="text-control text-fg">{gapTitle(gap)}</span>
                      </div>
                      <p className="text-body text-muted">{gap.detail}</p>
                      <p role="status" className="flex items-center gap-1.5 text-caption text-muted empty:hidden">
                        {working && <><StatusDot state="active" pulse label="In progress" />{gap.collect ? 'Starting the collection…' : 'Fetching…'}</>}
                        {done && <><StatusDot state="idle" label="Done" />{gap.collect ? 'Collection started; the source is listed in the sidebar.' : 'Fetched; the checks are running again.'}</>}
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
      </PanelBody>
    </Panel>
  );
}
