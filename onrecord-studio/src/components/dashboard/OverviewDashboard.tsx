import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import TemporalMap from '../charts/TemporalMap';
import DepartmentLedger, { ledgerOrder, SMALL_SAMPLE_CLOSURES } from '../charts/figures/DepartmentLedger';
import Dumbbells, { changeText, type DumbbellKind, type DumbbellRow } from '../charts/figures/Dumbbells';
import { formatDateRange, formatValue } from '../charts/core/format';
import { absent, measured, type Measured } from '../charts/core/nulls';
import { useAppStore, type CaseworkTab, type OverviewPerspective } from '../../store/useAppStore';
import { scrollWithin } from '../../utils/scrollWithin';
import MetricsGrid from './MetricsGrid';
import Icon from '../ui/icons';
import Skeleton from '../ui/Skeleton';
import Panel, { PanelHeader } from '../ui/Panel';
import Button from '../ui/Button';
import { Select } from '../ui/Field';
import SegmentedControl from '../ui/SegmentedControl';
import { cn } from '../../utils/cn';
import SanDiegoWorkspace from './SanDiegoWorkspace';
import InvestigationDesk from './InvestigationDesk';
import ResearchWorkspace from './ResearchWorkspace';
import ProfessionalWorkbench from './ProfessionalWorkbench';
import DataMaintenance from './DataMaintenance';
import ResearchSnapshotExport from './ResearchSnapshotExport';
import RegistrySnapshots from './RegistrySnapshots';
import AutomaticDiscovery from './AutomaticDiscovery';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { AgencyPerformanceProfile } from '../../types';
import OverviewWorkbench, { WorkbenchSkeleton } from '../workbench/OverviewWorkbench';
import Masthead, { Dateline } from '../workbench/Masthead';
import EmptyState from '../ui/EmptyState';
import SinceControl from './SinceControl';
import WatchlistPanel from './WatchlistPanel';
import StandingSearchPanel from './StandingSearchPanel';
import SubjectDossier from './SubjectDossier';
import PostIngestBrief from './PostIngestBrief';
import CrossCorpusExplorer from './CrossCorpusExplorer';
import TopicPacks from './TopicPacks';
import StarterSets from './StarterSets';
import FirstRunData from './FirstRunData';
import { useStarterRunStore } from '../../store/useStarterRunStore';
import { INVESTIGATION_TOPICS } from '../../utils/investigations';

const PERSPECTIVES: { value: OverviewPerspective; label: string }[] = [
  { value: 'leads', label: 'Leads' },
  { value: 'following', label: 'Following' },
  { value: 'analytics', label: 'Analytics' },
  { value: 'casework', label: 'Casework' },
  { value: 'tools', label: 'Investigations' },
];

export default function OverviewDashboard() {
  const [topicId, setTopicId] = useState(INVESTIGATION_TOPICS[0].id);
  const topic = INVESTIGATION_TOPICS.find((item) => item.id === topicId)!;
  const portals = useAppStore((state) => state.portals);
  const perspective = useAppStore((state) => state.overviewPerspective);
  const setPerspective = useAppStore((state) => state.setOverviewPerspective);
  const [casework, setCasework] = useState<{ reportId: string; tab: CaseworkTab; opened: number }>({ reportId: '', tab: 'Cases', opened: 0 });
  const requestedCasework = useAppStore((state) => state.requestedCasework);
  const consumeCasework = useAppStore((state) => state.consumeCasework);
  const openCasework = useAppStore((state) => state.openCasework);
  // Casework opened from the case switcher, the inspector or the command bar lands on its tab.
  useEffect(() => {
    if (!requestedCasework) return;
    const request = consumeCasework();
    if (request) setCasework((current) => ({ reportId: request.reportId, tab: request.tab ?? 'Cases', opened: current.opened + 1 }));
  }, [consumeCasework, requestedCasework]);
  const workspaceInitializationStatus = useAppStore((state) => state.workspaceInitializationStatus);
  // Stays up after the first source lands, so the outcome of every source in a run is visible.
  const starterRunShown = useStarterRunStore((state) => Object.keys(state.statuses).length > 0);
  const workspaceInitializationError = useAppStore((state) => state.workspaceInitializationError);
  const profiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const analyticsUpdatedAt = useAppStore((state) => state.analyticsUpdatedAt);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const correlationCount = useAppStore((state) => state.crossAgencyCorrelations.length);
  // With no result at all, one retryable error stands for the whole panel; the comparison and the
  // charts below would each restate the same failure.
  const analyticsUnavailable = analyticsError !== null && profiles.length === 0 && correlationCount === 0;
  const scopedProfiles = useMemo(
    () => profiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey),
    [activePortalKey, profiles],
  );
  const analyticsStatus = useAppStore((state) => state.analyticsStatus);
  const caseScoped = useAppStore((state) => state.activeCaseId !== null && state.scopeToCase);
  const inspectDepartment = useInspectDepartment();
  const portalName = usePortalName();
  const scroller = useRef<HTMLDivElement>(null);
  // Scrolls the overview's own column to the ledger; never the shell around it.
  const openLedger = useCallback(() => {
    setPerspective('analytics');
    requestAnimationFrame(() => {
      const ledger = document.getElementById('deadline-ledger');
      if (ledger && scroller.current) scrollWithin(scroller.current, ledger, 'start');
    });
  }, [setPerspective]);
  const hasSources = portals.length > 0;
  const ledger = (
    <DepartmentLedger
      variant="overview"
      profiles={ledgerOrder(scopedProfiles)}
      scaleProfiles={scopedProfiles}
      status={caseScoped ? 'scoped-out' : profiles.length === 0 && analyticsStatus === 'pending' ? 'loading' : 'ready'}
      scopedOutText="Hidden while a case scope is active: these are all-time totals for every request, not the case's."
      portalName={portalName}
      onInspect={(profile) => inspectDepartment(profile.portalSubdomain, profile.agencyId, true)}
    />
  );
  const loading = workspaceInitializationStatus === 'loading';
  const failed = workspaceInitializationStatus === 'error';
  const showStarter = workspaceInitializationStatus === 'ready' && (portals.length === 0 || starterRunShown);
  const leadsWorkbench = perspective === 'leads' && hasSources && !showStarter && workspaceInitializationStatus === 'ready';
  const fillsView = perspective === 'leads' && (leadsWorkbench || loading);

  return (
    <div ref={scroller} className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <div className={cn('mx-auto flex w-full max-w-content flex-1 flex-col gap-3 px-3 pt-4 md:px-4', fillsView ? 'min-h-[36rem] pb-3' : 'pb-10')}>
        <Masthead
          overline={<Dateline />}
          title="Overview"
          tabs={(
            <SegmentedControl<OverviewPerspective>
              label="Overview perspective"
              mode="tabs"
              size="sm"
              value={perspective}
              onChange={setPerspective}
              panelId={() => 'overview-perspective'}
              options={PERSPECTIVES}
            />
          )}
          actions={<>
            {hasSources && !loading && !showStarter && <SinceControl />}
            {/* With no source, or sources that didn't load, Evidence has nothing to review. */}
            {hasSources && !failed && <Button onClick={() => setWorkspaceView('evidence')}>Review evidence</Button>}
          </>}
        />

        <FirstRunData />
        <PostIngestBrief />
        {perspective === 'leads' && !loading && !failed && (
          <section aria-label="Start an investigation" className="rounded-xl border border-line bg-panel p-2">
            <div className="flex flex-wrap items-center gap-3">
              <label htmlFor="starting-issue" className="text-body font-medium text-fg">Start with an issue</label>
              <Select id="starting-issue" value={topicId} onChange={(event) => setTopicId(event.target.value)}>
                {INVESTIGATION_TOPICS.map((item) => <option key={item.id} value={item.id}>{item.title}</option>)}
              </Select>
              <Button variant="primary" onClick={() => setPerspective('tools')}>Investigate this issue</Button>
              <Button variant="ghost" onClick={() => {
                useAppStore.getState().setActivePortal(null);
                useAppStore.getState().setSearchFilters(topic.searches[0], null, null, null);
                setWorkspaceView('evidence');
              }}>Find relevant records</Button>
            </div>
            <p className="mt-1 text-body text-fg-secondary">{topic.question}</p>
            <p className="text-caption text-muted">1. Choose an issue. 2. Check a lead against its cited records. 3. Draft a records request or add verified evidence to a briefing.</p>
          </section>
        )}
        <div id="overview-perspective" role="tabpanel" aria-label={PERSPECTIVES.find((option) => option.value === perspective)?.label} className={cn('flex flex-col gap-3', fillsView ? 'min-h-0 flex-1' : perspective !== 'leads' && 'motion-fade')}>
          {failed ? (
            <div className="mx-auto w-full max-w-[46rem]"><WorkspaceSourcesUnavailable detail={workspaceInitializationError} /></div>
          ) : <>
            <SubjectDossier />
            {perspective === 'leads' && (
              loading ? <WorkbenchSkeleton />
                : showStarter ? <section className="app-panel mx-auto w-full max-w-[46rem] p-5"><StarterSets /></section>
                  : leadsWorkbench && <OverviewWorkbench ledgerFigure={ledger} onOpenLedger={openLedger} onOpenFollowing={() => setPerspective('following')} />
            )}
            {perspective === 'following' && (
              <section aria-label="Following" className="grid grid-cols-1 items-start gap-4 2xl:grid-cols-2">
                <WatchlistPanel />
                <StandingSearchPanel />
              </section>
            )}
            {perspective === 'analytics' && (loading ? (
              <Panel aria-label="Response analytics"><Skeleton variant="lines" lines={5} label="Loading your saved sources" className="p-5" /></Panel>
            ) : hasSources ? (
              <Panel aria-labelledby="response-analytics-title">
                <PanelHeader
                  titleId="response-analytics-title"
                  title="Response analytics"
                  description={analyticsError && !analyticsUnavailable
                    ? (analyticsUpdatedAt ? `Stale: the last refresh failed. Last updated ${formatRelativeTime(analyticsUpdatedAt)}.` : 'Analytics couldn’t be calculated.')
                    : analyticsUpdatedAt ? `Updated ${formatRelativeTime(analyticsUpdatedAt)}` : 'How quickly and completely agencies answer requests.'}
                />
                <div id="response-analytics-body" className="space-y-4 p-5">
                  {analyticsUnavailable ? <MetricsGrid /> : <>
                    <MetricsGrid />
                    <div id="deadline-ledger" className="scroll-mt-4 rounded-lg border border-line p-4">{ledger}</div>
                    <PeriodComparison activePortalKey={activePortalKey} />
                    <TemporalMap />
                  </>}
                </div>
              </Panel>
            ) : (
              <EmptyState icon="bars" title="No requests to measure yet" body="Response analytics measure how quickly and completely agencies answer the requests in your saved sources. Add a records portal to start." />
            ))}
            {perspective === 'casework' && (
              <ProfessionalWorkbench key={casework.opened} initialReportId={casework.reportId} initialTab={casework.tab} />
            )}
            {perspective === 'tools' && <>
              <ResearchWorkspace initialTopic={topicId} onReview={(reportId) => openCasework(reportId, 'Evidence review')} />
              <Panel aria-labelledby="more-tools-title">
                <PanelHeader titleId="more-tools-title" title="More tools" />
                <div className="divide-y divide-line px-5">
                  <ToolDrawer label="Discoveries across your library"><AutomaticDiscovery /></ToolDrawer>
                  <ToolDrawer label="Topic packs"><TopicPacks /></ToolDrawer>
                  <ToolDrawer label="Shared language across sources"><CrossCorpusExplorer /></ToolDrawer>
                  <ToolDrawer label="Plan an investigation or draft a request"><InvestigationDesk /></ToolDrawer>
                  <ToolDrawer label="Browse sources and add records"><SanDiegoWorkspace /></ToolDrawer>
                  <ToolDrawer label="Data upkeep and storage"><div className="space-y-6"><DataMaintenance /><ResearchSnapshotExport /><RegistrySnapshots /></div></ToolDrawer>
                </div>
              </Panel>
            </>}
          </>}
        </div>
      </div>
    </div>
  );
}

/** Mounted on first open and kept mounted, so a closed drawer keeps its drafts and filters. */
function ToolDrawer({ label, children }: { label: string; children: ReactNode }) {
  const [everOpened, setEverOpened] = useState(false);
  return (
    <details className="group" data-ui-drawer onToggle={(event) => { if (event.currentTarget.open) setEverOpened(true); }}>
      <summary className="focus-ring flex h-12 cursor-pointer list-none items-center gap-2 rounded text-body font-medium text-fg [&::-webkit-details-marker]:hidden">
        {label}
      </summary>
      {everOpened && <div className="pb-5 pt-1">{children}</div>}
    </details>
  );
}

type PeriodMeasure = 'requests' | 'closure' | 'counted' | 'latency';

const PERIOD_MEASURES: { value: PeriodMeasure; label: string }[] = [
  { value: 'counted', label: 'Counted deadline misses' },
  { value: 'closure', label: 'Closure rate' },
  { value: 'latency', label: 'Avg. closure days' },
  { value: 'requests', label: 'Requests' },
];

const NO_REQUESTS = 'no requests in window';
const NONE_MATURED = 'no request in window is old enough for its statutory clock to have run out';

/**
 * Closure has no deadline to mature against, so a closure measure's windows end this many days
 * ago: a request created last week has had no time to close, and counting it would show a decline
 * that is only censoring. The miss rate needs no lag: its denominator is the requests whose
 * longest deadline has passed (`statutoryMaturedCount`, decided by age alone), and younger ones
 * are left out.
 */
export const PERIOD_MATURITY_DAYS = 30;

const maturityLag = (measure: PeriodMeasure) => (measure === 'closure' || measure === 'latency' ? PERIOD_MATURITY_DAYS : 0);

interface PeriodWindows {
  current: AgencyPerformanceProfile[];
  prior: AgencyPerformanceProfile[];
  /** Window bounds, ms: start of prior, start of current, end of current. */
  bounds: [number, number, number];
  /** Days the windows end before today; 0 when they end today. */
  lag: number;
}

/**
 * This interval against the one before it, as dumbbells per department. The two windows are read
 * together behind one gate: a slower response for an interval or portal the reader has since left
 * is discarded, never drawn as the current comparison.
 */
export function PeriodComparison({ activePortalKey }: { activePortalKey: string | null }) {
  const [days, setDays] = useState(30);
  const [measure, setMeasure] = useState<PeriodMeasure>('counted');
  const [windows, setWindows] = useState<PeriodWindows | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const comparisonGate = useLatestRequestGate();
  const inspectDepartment = useInspectDepartment();
  const portalName = usePortalName();
  const lag = maturityLag(measure);

  useEffect(() => {
    const request = comparisonGate.begin();
    const end = Date.now() - lag * 86_400_000;
    const middle = end - days * 86_400_000;
    const start = end - days * 2 * 86_400_000;
    setIsLoading(true);
    setError(null);
    const inScope = (profile: AgencyPerformanceProfile) => !activePortalKey || profile.portalSubdomain === activePortalKey;
    void Promise.all([
      invokeCommand('get_agency_performance_profiles', { config: { created_from: new Date(middle).toISOString(), created_to: new Date(end).toISOString() } }),
      invokeCommand('get_agency_performance_profiles', { config: { created_from: new Date(start).toISOString(), created_to: new Date(middle).toISOString() } }),
    ]).then(([currentProfiles, priorProfiles]) => {
      if (!comparisonGate.isCurrent(request)) return;
      setWindows({ current: currentProfiles.filter(inScope), prior: priorProfiles.filter(inScope), bounds: [start, middle, end], lag });
    }).catch((reason) => {
      if (comparisonGate.isCurrent(request)) setError(formatNativeError(reason));
    }).finally(() => {
      if (comparisonGate.isCurrent(request)) setIsLoading(false);
    });
    return comparisonGate.invalidate;
  }, [activePortalKey, comparisonGate, days, lag]);

  // A read for the other lag is still the last result while the new one loads; it never stands for
  // the selected measure.
  const current = windows && windows.lag === lag ? windows : null;
  const model = current ? periodModel(current, measure, days, portalName, inspectDepartment) : null;
  const status = error ? 'error' : !model ? 'loading' : isLoading ? 'stale' : model.empty ? 'empty' : 'ready';
  const range = current ? { prior: formatDateRange(current.bounds[0], current.bounds[1]), current: formatDateRange(current.bounds[1], current.bounds[2]) } : null;
  return (
    <section className="rounded-lg border border-line p-4" aria-label="Response behavior period comparison">
      <Dumbbells
        title="Period comparison"
        storageKey="period-comparison"
        headline={model?.headline ?? null}
        claim={measure === 'counted' ? 'counted' : undefined}
        sources={{
          tables: ['requests', 'statutory_extensions', 'agencies'],
          note: range
            ? `requests created ${range.current} vs ${range.prior}${lag > 0 ? `; both windows end ${lag} days ago, so no request in them is younger than ${lag} days` : ''}${measure === 'counted' ? '; the rate counts only requests older than the longest statutory deadline (10 days plus a 14-day extension, carried to a court day), so no clock in it is still running' : ''}`
            : undefined,
        }}
        status={status}
        error={error}
        emptyText={lag > 0
          ? `No requests were created in the ${days * 2} days before ${lag} days ago, so there is nothing to compare.`
          : `No requests were created in the last ${days * 2} days, so there is nothing to compare.`}
        kind={model?.kind ?? 'rate'}
        rows={model?.rows ?? []}
        tooFew={model?.tooFew}
        priorLabel={lag > 0 ? (range ? `Created ${range.prior}` : `Earlier ${days} days`) : `Prior ${days} days`}
        currentLabel={lag > 0 ? (range ? `Created ${range.current}` : `Later ${days} days`) : `Last ${days} days`}
        controls={<>
          <SegmentedControl<PeriodMeasure> label="Measure" mode="filter" size="sm" value={measure} onChange={setMeasure} options={PERIOD_MEASURES} />
          <Select size="sm" aria-label="Interval" value={days} onChange={(event) => setDays(Number(event.target.value))} className="w-auto tabular-nums">
            <option value={7}>7 days</option><option value={30}>30 days</option><option value={90}>90 days</option><option value={365}>365 days</option>
          </Select>
        </>}
      />
    </section>
  );
}

interface PeriodSide {
  requests: number;
  closed: number;
  violations: number;
  /** Requests older than any statutory deadline: the only ones a miss rate may count. */
  matured: number;
  /** Counted misses among the matured requests. */
  maturedViolations: number;
  closures: number;
  /** Closure days weighted by the closures each average rests on; null when nothing closed. */
  averageClosureDays: number | null;
}

function summarizePeriod(profiles: readonly AgencyPerformanceProfile[]): PeriodSide {
  const requests = profiles.reduce((sum, profile) => sum + profile.totalRequestsProcessed, 0);
  const closed = profiles.reduce((sum, profile) => sum + profile.totalClosed, 0);
  const violations = profiles.reduce((sum, profile) => sum + profile.statutoryViolationCount, 0);
  const matured = profiles.reduce((sum, profile) => sum + profile.statutoryMaturedCount, 0);
  const maturedViolations = profiles.reduce((sum, profile) => sum + profile.statutoryMaturedViolationCount, 0);
  const measuredProfiles = profiles.filter((profile) => profile.averageClosureDays !== null && profile.validLatencySampleSize > 0);
  const closures = measuredProfiles.reduce((sum, profile) => sum + profile.validLatencySampleSize, 0);
  const latencyTotal = measuredProfiles.reduce((sum, profile) => sum + (profile.averageClosureDays as number) * profile.validLatencySampleSize, 0);
  return { requests, closed, violations, matured, maturedViolations, closures, averageClosureDays: closures > 0 ? latencyTotal / closures : null };
}

/** One window's value of the measure; a window with no requests has no rate, never a 0% rate. */
function sideValue(side: PeriodSide, measure: PeriodMeasure): Measured {
  if (measure === 'requests') return measured(side.requests);
  if (side.requests === 0) return absent('not-recorded', NO_REQUESTS);
  if (measure === 'closure') return measured(side.closed / side.requests);
  if (measure === 'counted') return side.matured === 0 ? absent('not-recorded', NONE_MATURED) : measured(side.maturedViolations / side.matured);
  return side.averageClosureDays === null ? absent('not-recorded', 'no measurable closure in window') : measured(side.averageClosureDays);
}

function sideBasis(side: PeriodSide, measure: PeriodMeasure): string {
  const plural = (n: number, word: string) => `${n.toLocaleString('en-US')} ${word}${n === 1 ? '' : 's'}`;
  if (measure === 'latency') {
    const open = side.requests - side.closed;
    return `${plural(side.closures, 'closure')}${open > 0 ? `; ${open.toLocaleString('en-US')} still open, not measured` : ''}`;
  }
  if (measure === 'counted') {
    const recent = side.requests - side.matured;
    return `${side.maturedViolations.toLocaleString('en-US')} of ${plural(side.matured, 'request')} old enough to judge${recent > 0 ? `; ${recent.toLocaleString('en-US')} too recent, not counted` : ''}`;
  }
  if (measure === 'closure') return `${side.closed.toLocaleString('en-US')} of ${plural(side.requests, 'request')}`;
  return plural(side.requests, 'request');
}

/** Below this many requests (or closures, for latency) in either window a change is not compared. */
const PERIOD_MINIMUM = SMALL_SAMPLE_CLOSURES;

function periodModel(
  { current, prior, bounds, lag }: PeriodWindows,
  measure: PeriodMeasure,
  days: number,
  portalName: (portalKey: string) => string,
  onInspect: (portalKey: string, agencyId: number) => void,
) {
  const kind: DumbbellKind = measure === 'requests' ? 'count' : measure === 'latency' ? 'days' : 'rate';
  const totalCurrent = summarizePeriod(current);
  const totalPrior = summarizePeriod(prior);
  const empty = totalCurrent.requests === 0 && totalPrior.requests === 0;
  const departments = new Map<string, { portalKey: string; agencyId: number; name: string; current: AgencyPerformanceProfile[]; prior: AgencyPerformanceProfile[] }>();
  for (const [side, list] of [['current', current], ['prior', prior]] as const) {
    for (const profile of list) {
      const key = `${profile.portalSubdomain}:${profile.agencyId}`;
      const entry = departments.get(key) ?? { portalKey: profile.portalSubdomain, agencyId: profile.agencyId, name: profile.agencyName, current: [], prior: [] };
      entry[side].push(profile);
      departments.set(key, entry);
    }
  }
  const spansPortals = new Set([...departments.values()].map((entry) => entry.portalKey)).size > 1;
  const rows: DumbbellRow[] = [{
    key: 'total',
    label: 'All departments',
    total: true,
    prior: sideValue(totalPrior, measure),
    current: sideValue(totalCurrent, measure),
    priorBasis: sideBasis(totalPrior, measure),
    currentBasis: sideBasis(totalCurrent, measure),
  }];
  const tooFew: string[] = [];
  for (const [key, entry] of departments) {
    const a = summarizePeriod(entry.prior);
    const b = summarizePeriod(entry.current);
    const label = spansPortals ? `${entry.name} (${portalName(entry.portalKey)})` : entry.name;
    const basisOf = (side: PeriodSide) => (measure === 'latency' ? side.closures : measure === 'counted' ? side.matured : side.requests);
    if (measure !== 'requests' && (basisOf(a) < PERIOD_MINIMUM || basisOf(b) < PERIOD_MINIMUM) && (basisOf(a) > 0 || basisOf(b) > 0)) {
      tooFew.push(label);
      continue;
    }
    rows.push({
      key,
      label,
      prior: sideValue(a, measure),
      current: sideValue(b, measure),
      priorBasis: sideBasis(a, measure),
      currentBasis: sideBasis(b, measure),
      onActivate: () => onInspect(entry.portalKey, entry.agencyId),
    });
  }
  return {
    kind,
    rows,
    empty,
    tooFew: { labels: tooFew.sort((x, y) => x.localeCompare(y)), reason: `fewer than ${PERIOD_MINIMUM} ${measure === 'latency' ? 'closures' : measure === 'counted' ? 'requests old enough to judge' : 'requests'} in a window` },
    headline: periodHeadline(totalPrior, totalCurrent, measure, lag > 0 ? `created ${formatDateRange(bounds[1], bounds[2])}` : `created in the last ${days} days`, days),
  };
}

function periodHeadline(prior: PeriodSide, current: PeriodSide, measure: PeriodMeasure, created: string, days: number): string {
  const requests = `${current.requests.toLocaleString('en-US')} request${current.requests === 1 ? '' : 's'} ${created}`;
  if (measure === 'requests') return `${requests}, against ${prior.requests.toLocaleString('en-US')} in the ${days} days before.`;
  if (current.requests === 0) return `No requests were ${created}, so there is no current rate to compare.`;
  const kind: DumbbellKind = measure === 'latency' ? 'days' : 'rate';
  const now = sideValue(current, measure);
  const before = sideValue(prior, measure);
  const name = measure === 'counted' ? 'Counted deadline-miss rate' : measure === 'closure' ? 'Closure rate' : 'Average closure';
  const show = (value: number) => (kind === 'rate' ? formatValue(value, { kind: 'percent' }) : formatValue(value, { kind: 'days' }));
  // The miss rate rests on the requests past their deadline, never the ones still within their clock.
  const basis = measure === 'counted' ? `${current.matured.toLocaleString('en-US')} of the ${requests} old enough to judge` : requests;
  if (now.state !== 'value') return measure === 'counted' ? `${name}: none of the ${requests} is old enough for its statutory clock to have run out.` : `${name}: no measurable closure among the ${requests}.`;
  if (before.state !== 'value') return `${name} ${show(now.value)} across ${basis}; no comparable prior window.`;
  return `${name} ${show(now.value)} vs ${show(before.value)} (${changeText(before.value, now.value, kind)}) across ${basis}.`;
}

function WorkspaceSourcesUnavailable({ detail }: { detail: string | null }) {
  const retry = useAppStore((state) => state.retryWorkspaceInitialization);
  return (
    <section className="app-panel p-5" aria-labelledby="sources-unavailable-title">
      <div className="flex items-start gap-3">
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-raised text-fg-secondary"><Icon name="database" size="md" /></span>
        <div className="min-w-0">
          <div role="alert">
            <h3 id="sources-unavailable-title" className="text-heading text-fg">Your saved sources didn’t load</h3>
            <p className="mt-1 max-w-prose text-body text-muted">
              The local evidence store is busy or unavailable. Your sources and documents are not affected.
            </p>
          </div>
          {detail && (
            <details className="mt-2">
              <summary className="text-caption text-muted">Technical details</summary>
              <p className="mt-1 break-words font-mono text-code text-fg-secondary">{detail}</p>
            </details>
          )}
          <Button size="sm" className="mt-3" onClick={retry}>Try again</Button>
        </div>
      </div>
    </section>
  );
}

/**
 * The overview's department action: scope Evidence to that department's requests. From the counted
 * ledger it also keeps only the requests whose statutory deadline was exceeded, so the misses the
 * reader clicked are the list they land on.
 */
function useInspectDepartment() {
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  return useCallback((portalKey: string, agencyId: number, statutoryOnly = false) => {
    setActivePortal(portalKey);
    setSearchFilters('', agencyId, null, null, statutoryOnly);
    setWorkspaceView('evidence');
  }, [setActivePortal, setSearchFilters, setWorkspaceView]);
}

function usePortalName() {
  const portals = useAppStore((state) => state.portals);
  return useMemo(() => {
    const names = new Map(portals.map(({ registration }) => [registration.portalKey, registration.displayName]));
    return (portalKey: string) => names.get(portalKey) ?? portalKey;
  }, [portals]);
}

function formatRelativeTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return value;
  const seconds = Math.max(0, Math.round((Date.now() - date.valueOf()) / 1000));
  if (seconds < 60) return 'just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}
