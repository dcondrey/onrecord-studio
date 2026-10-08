import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import TemporalMap from '../charts/TemporalMap';
import DepartmentLedger, { ledgerOrder, SMALL_SAMPLE_CLOSURES } from '../charts/figures/DepartmentLedger';
import Dumbbells, { changeText, type DumbbellKind, type DumbbellRow } from '../charts/figures/Dumbbells';
import { formatDateRange, formatValue } from '../charts/core/format';
import { absent, measured, type Measured } from '../charts/core/nulls';
import { useAppStore } from '../../store/useAppStore';
import MetricsGrid from './MetricsGrid';
import Icon from '../ui/icons';
import PageHeader from '../ui/PageHeader';
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
import AttentionFeed, { AttentionFeedSkeleton } from './AttentionFeed';
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

export default function OverviewDashboard() {
  const portals = useAppStore((state) => state.portals);
  const [showAnalytics, setShowAnalytics] = useState(false);
  const [workbench, setWorkbench] = useState<{ reportId: string; review: boolean } | null>(null);
  const requestedCasework = useAppStore((state) => state.requestedCasework);
  const consumeCasework = useAppStore((state) => state.consumeCasework);
  // Casework opened from the case switcher or the inspector lands here.
  useEffect(() => {
    if (!requestedCasework) return;
    const request = consumeCasework();
    if (request) setWorkbench({ reportId: request.reportId, review: false });
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
  const openLedger = useCallback(() => {
    setShowAnalytics(true);
    requestAnimationFrame(() => document.getElementById('deadline-ledger')?.scrollIntoView({ block: 'start' }));
  }, []);
  const openWorkbench = () => setWorkbench({ reportId: '', review: false });

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-content space-y-6 px-4 pb-10 pt-6 md:px-6">
        {workbench ? (
          <div>
            <button type="button" onClick={() => setWorkbench(null)} className="focus-ring mb-2 inline-flex items-center gap-1 rounded text-body text-accent hover:underline">
              <Icon name="back" size="sm" /> Overview
            </button>
            <PageHeader title="Casework" summary="Track requests and commitments, reconcile spending, and challenge the evidence." />
          </div>
        ) : (
          <PageHeader
            title="Overview"
            summary="New leads, what you follow, and the state of your sources."
            actions={<>
              {portals.length > 0 && <SinceControl />}
              <Button onClick={() => setWorkspaceView('evidence')}>Review evidence</Button>
            </>}
          />
        )}

        <FirstRunData />
        {workspaceInitializationStatus === 'loading' ? (
          <section aria-label="Needs attention" className="app-panel overflow-hidden"><AttentionFeedSkeleton /></section>
        ) : workspaceInitializationStatus === 'error' ? (
          <WorkspaceSourcesUnavailable detail={workspaceInitializationError} />
        ) : portals.length === 0 || starterRunShown ? (
          <section className="app-panel p-5"><StarterSets /></section>
        ) : null}

        <PostIngestBrief />
        {workbench ? (
          <ProfessionalWorkbench key={`${workbench.reportId}:${workbench.review}`} initialReportId={workbench.reportId} initialTab={workbench.review ? 'Evidence review' : 'Cases'} />
        ) : <>
          <SubjectDossier />
          {portals.length > 0 && <AttentionFeed onOpenLedger={openLedger} />}
          <section aria-label="Following" className="grid grid-cols-1 items-start gap-4 2xl:grid-cols-2">
            <WatchlistPanel />
            <StandingSearchPanel />
          </section>
          <ResearchWorkspace onReview={(reportId) => setWorkbench({ reportId, review: true })} />

          {portals.length > 0 && (
            <Panel aria-labelledby="response-analytics-title" className={cn(!showAnalytics && '[&>div:first-child]:border-b-0')}>
              <PanelHeader
                titleId="response-analytics-title"
                title="Response analytics"
                description={analyticsError && !analyticsUnavailable
                  ? (analyticsUpdatedAt ? `Stale: the last refresh failed. Last updated ${formatRelativeTime(analyticsUpdatedAt)}.` : 'Analytics couldn’t be calculated.')
                  : analyticsUpdatedAt ? `Updated ${formatRelativeTime(analyticsUpdatedAt)}` : 'How quickly and completely agencies answer requests.'}
                actions={(
                  <Button aria-expanded={showAnalytics} aria-controls="response-analytics-body" aria-label={showAnalytics ? 'Hide response analytics' : 'Show response analytics'} onClick={() => setShowAnalytics((visible) => !visible)}>
                    {showAnalytics ? 'Hide' : 'Show'}
                  </Button>
                )}
              />
              {showAnalytics && (
                <div id="response-analytics-body" className="motion-reveal space-y-4 p-5">
                  {analyticsUnavailable ? <MetricsGrid /> : <>
                    <MetricsGrid />
                    <div id="deadline-ledger" className="scroll-mt-4 rounded-lg border border-line p-4">
                      <DepartmentLedger
                        variant="overview"
                        profiles={ledgerOrder(scopedProfiles)}
                        scaleProfiles={scopedProfiles}
                        status={caseScoped ? 'scoped-out' : profiles.length === 0 && analyticsStatus === 'pending' ? 'loading' : 'ready'}
                        scopedOutText="Hidden while a case scope is active: these are all-time totals for every request, not the case's."
                        portalName={portalName}
                        onInspect={(profile) => inspectDepartment(profile.portalSubdomain, profile.agencyId)}
                      />
                    </div>
                    <PeriodComparison activePortalKey={activePortalKey} />
                    <TemporalMap />
                  </>}
                </div>
              )}
            </Panel>
          )}

          <Panel aria-labelledby="more-tools-title">
            <PanelHeader titleId="more-tools-title" title="More tools" />
            <div className="divide-y divide-line px-5">
              <div className="flex items-center justify-between gap-4 py-3">
                <div className="min-w-0">
                  <p className="text-body font-medium text-fg">Develop a lead into a durable case</p>
                  <p className="text-caption text-muted">Track requests, productions and commitments; reconcile spending; challenge the evidence.</p>
                </div>
                <Button onClick={openWorkbench}>Open casework and research tools</Button>
              </div>
              <ToolDrawer label="Discoveries across your library"><AutomaticDiscovery /></ToolDrawer>
              <ToolDrawer label="Topic packs"><TopicPacks /></ToolDrawer>
              <ToolDrawer label="Shared language across sources"><CrossCorpusExplorer /></ToolDrawer>
              <ToolDrawer label="Plan an investigation or draft a request"><InvestigationDesk /></ToolDrawer>
              <ToolDrawer label="Browse sources and add records"><SanDiegoWorkspace /></ToolDrawer>
              <ToolDrawer label="Data upkeep and storage"><div className="space-y-6"><DataMaintenance /><ResearchSnapshotExport /><RegistrySnapshots /></div></ToolDrawer>
            </div>
          </Panel>
        </>}
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

/**
 * A request's statutory clock can still be running this many days after it was created: 10 days to
 * determine plus a 14-day extension (statute.rs CPRA_INITIAL_DETERMINATION_DAYS and
 * CPRA_MAXIMUM_EXTENSION_DAYS), a last day carried past a weekend and holidays, and one more day
 * because a miss is counted only once the elapsed days exceed the entitlement. A window ending
 * sooner would count its young requests as on time and show an improvement that is only censoring.
 */
export const PERIOD_MATURITY_DAYS = 30;

/** Every measure but the request count depends on how a request ended, so it waits for maturity. */
const maturityLag = (measure: PeriodMeasure) => (measure === 'requests' ? 0 : PERIOD_MATURITY_DAYS);

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
            ? `requests created ${range.current} vs ${range.prior}${lag > 0 ? `; both windows end ${lag} days ago, so every request in them is past its longest statutory deadline (10 days plus a 14-day extension, carried to a court day)` : ''}`
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
  closures: number;
  /** Closure days weighted by the closures each average rests on; null when nothing closed. */
  averageClosureDays: number | null;
}

function summarizePeriod(profiles: readonly AgencyPerformanceProfile[]): PeriodSide {
  const requests = profiles.reduce((sum, profile) => sum + profile.totalRequestsProcessed, 0);
  const closed = profiles.reduce((sum, profile) => sum + profile.totalClosed, 0);
  const violations = profiles.reduce((sum, profile) => sum + profile.statutoryViolationCount, 0);
  const measuredProfiles = profiles.filter((profile) => profile.averageClosureDays !== null && profile.validLatencySampleSize > 0);
  const closures = measuredProfiles.reduce((sum, profile) => sum + profile.validLatencySampleSize, 0);
  const latencyTotal = measuredProfiles.reduce((sum, profile) => sum + (profile.averageClosureDays as number) * profile.validLatencySampleSize, 0);
  return { requests, closed, violations, closures, averageClosureDays: closures > 0 ? latencyTotal / closures : null };
}

/** One window's value of the measure; a window with no requests has no rate, never a 0% rate. */
function sideValue(side: PeriodSide, measure: PeriodMeasure): Measured {
  if (measure === 'requests') return measured(side.requests);
  if (side.requests === 0) return absent('not-recorded', NO_REQUESTS);
  if (measure === 'closure') return measured(side.closed / side.requests);
  if (measure === 'counted') return measured(side.violations / side.requests);
  return side.averageClosureDays === null ? absent('not-recorded', 'no measurable closure in window') : measured(side.averageClosureDays);
}

function sideBasis(side: PeriodSide, measure: PeriodMeasure): string {
  const plural = (n: number, word: string) => `${n.toLocaleString('en-US')} ${word}${n === 1 ? '' : 's'}`;
  if (measure === 'latency') {
    const open = side.requests - side.closed;
    return `${plural(side.closures, 'closure')}${open > 0 ? `; ${open.toLocaleString('en-US')} still open, not measured` : ''}`;
  }
  if (measure === 'counted') return `${side.violations.toLocaleString('en-US')} of ${plural(side.requests, 'request')}`;
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
    const basisOf = (side: PeriodSide) => (measure === 'latency' ? side.closures : side.requests);
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
    tooFew: { labels: tooFew.sort((x, y) => x.localeCompare(y)), reason: `fewer than ${PERIOD_MINIMUM} ${measure === 'latency' ? 'closures' : 'requests'} in a window` },
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
  if (now.state !== 'value') return `${name}: no measurable closure among the ${requests}.`;
  if (before.state !== 'value') return `${name} ${show(now.value)} across ${requests}; no comparable prior window.`;
  return `${name} ${show(now.value)} vs ${show(before.value)} (${changeText(before.value, now.value, kind)}) across ${requests}.`;
}

function WorkspaceSourcesUnavailable({ detail }: { detail: string | null }) {
  const retry = useAppStore((state) => state.retryWorkspaceInitialization);
  return (
    <section className="app-panel p-5" aria-labelledby="sources-unavailable-title">
      <div className="flex items-start gap-3">
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-raised text-fg-secondary"><Icon name="database" size="md" /></span>
        <div className="min-w-0">
          <h3 id="sources-unavailable-title" role="alert" className="text-heading text-fg">Your saved sources didn’t load</h3>
          <p className="mt-1 max-w-prose text-body text-muted">
            The local evidence store is busy or unavailable. Your sources and documents are not affected.
          </p>
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

/** The overview's department action: scope Evidence to that department's requests. */
function useInspectDepartment() {
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  return useCallback((portalKey: string, agencyId: number) => {
    setActivePortal(portalKey);
    setSearchFilters('', agencyId, null, null);
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
