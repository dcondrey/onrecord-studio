import { useEffect, useMemo, useState, type ReactNode } from 'react';
import AnomalyRadar from '../charts/AnomalyRadar';
import LifecycleFunnel from '../charts/LifecycleFunnel';
import PerformanceScatter from '../charts/PerformanceScatter';
import TemporalMap from '../charts/TemporalMap';
import AnalyticsUnavailable from '../charts/AnalyticsUnavailable';
import { useAppStore } from '../../store/useAppStore';
import MetricsGrid from './MetricsGrid';
import Icon from '../ui/icons';
import PageHeader from '../ui/PageHeader';
import Panel, { PanelHeader } from '../ui/Panel';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import { Select } from '../ui/Field';
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
import AttentionFeed from './AttentionFeed';
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
  const scopedProfiles = useMemo(
    () => profiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey),
    [activePortalKey, profiles],
  );
  const outlierDepartments = scopedProfiles.filter(
    (profile) => profile.performanceZScore !== null && profile.performanceZScore >= 2,
  ).length;
  const monitoredDepartments = scopedProfiles.length;
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
          <Skeleton variant="block" height={120} label="Loading saved sources" />
        ) : workspaceInitializationStatus === 'error' ? (
          <WorkspaceSourcesUnavailable detail={workspaceInitializationError} />
        ) : portals.length === 0 || starterRunShown ? (
          <section className="app-panel p-5"><StarterSets /></section>
        ) : null}

        <PostIngestBrief />
        {workbench ? (
          <ProfessionalWorkbench key={`${workbench.reportId}:${workbench.review}`} initialReportId={workbench.reportId} initialTab={workbench.review ? 'Evidence review' : 'Cases and follow-through'} />
        ) : <>
          <SubjectDossier />
          {portals.length > 0 && <AttentionFeed />}
          <section aria-label="Following" className="grid grid-cols-1 items-start gap-4 2xl:grid-cols-2">
            <WatchlistPanel />
            <StandingSearchPanel />
          </section>
          <ResearchWorkspace onReview={(reportId) => setWorkbench({ reportId, review: true })} />

          {portals.length > 0 && (
            <Panel aria-labelledby="response-analytics-title">
              <PanelHeader
                titleId="response-analytics-title"
                title="Response analytics"
                description={analyticsError
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
                  <PeriodComparison activePortalKey={activePortalKey} />
                  <MetricsGrid />
                  <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1fr)_300px]">
                    <PerformanceScatter />
                    <SignalSummary outlierDepartments={outlierDepartments} monitoredDepartments={monitoredDepartments} />
                  </div>
                  <section aria-label="Investigative analytics" className="grid grid-cols-1 gap-4 lg:grid-cols-2 2xl:grid-cols-[1.15fr_.8fr_1.05fr]">
                    <AnomalyRadar />
                    <LifecycleFunnel />
                    <div className="lg:col-span-2 2xl:col-span-1"><TemporalMap /></div>
                  </section>
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
    <details className="group" onToggle={(event) => { if (event.currentTarget.open) setEverOpened(true); }}>
      <summary className="focus-ring flex h-12 cursor-pointer list-none items-center gap-2 rounded text-body font-medium text-fg [&::-webkit-details-marker]:hidden">
        {label}
      </summary>
      {everOpened && <div className="pb-5 pt-1">{children}</div>}
    </details>
  );
}

function PeriodComparison({ activePortalKey }: { activePortalKey: string | null }) {
  const [days, setDays] = useState(30);
  const [current, setCurrent] = useState<AgencyPerformanceProfile[]>([]);
  const [prior, setPrior] = useState<AgencyPerformanceProfile[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const comparisonGate = useLatestRequestGate();

  useEffect(() => {
    const request = comparisonGate.begin();
    const end = new Date();
    const middle = new Date(end.valueOf() - days * 86_400_000);
    const start = new Date(end.valueOf() - days * 2 * 86_400_000);
    setIsLoading(true);
    setError(null);
    void Promise.all([
      invokeCommand('get_agency_performance_profiles', { config: { created_from: middle.toISOString(), created_to: end.toISOString() } }),
      invokeCommand('get_agency_performance_profiles', { config: { created_from: start.toISOString(), created_to: middle.toISOString() } }),
    ]).then(([currentProfiles, priorProfiles]) => {
      if (!comparisonGate.isCurrent(request)) return;
      setCurrent(currentProfiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey));
      setPrior(priorProfiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey));
    }).catch((reason) => {
      if (comparisonGate.isCurrent(request)) setError(formatNativeError(reason));
    }).finally(() => {
      if (comparisonGate.isCurrent(request)) setIsLoading(false);
    });
    return comparisonGate.invalidate;
  }, [activePortalKey, comparisonGate, days]);

  const currentSummary = summarizePeriod(current);
  const priorSummary = summarizePeriod(prior);
  return (
    <section className="rounded-lg border border-line p-4" aria-label="Response behavior period comparison">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <h4 className="text-body font-medium text-fg">Period comparison</h4>
          <p className="mt-0.5 text-caption text-muted">This interval against the one before it, by request creation time.</p>
        </div>
        <Select size="sm" aria-label="Interval" value={days} onChange={(event) => setDays(Number(event.target.value))} className="w-auto tabular-nums">
          <option value={7}>7 days</option><option value={30}>30 days</option><option value={90}>90 days</option><option value={365}>365 days</option>
        </Select>
      </div>
      {error ? (
        <InlineError className="mt-3" summary="The period comparison couldn’t be calculated." detail={error} />
      ) : (
        <div className="mt-3 grid gap-2 [grid-template-columns:repeat(auto-fit,minmax(10rem,1fr))]">
          <ComparisonMetric label="Closure rate" current={currentSummary.closureRate} prior={priorSummary.closureRate} percent loading={isLoading} />
          <ComparisonMetric label="Missed statutory deadlines" current={currentSummary.violationRate} prior={priorSummary.violationRate} percent inverse loading={isLoading} />
          <ComparisonMetric label="Avg. closure days" current={currentSummary.averageClosureDays} prior={priorSummary.averageClosureDays} inverse loading={isLoading} />
        </div>
      )}
    </section>
  );
}

function summarizePeriod(profiles: AgencyPerformanceProfile[]) {
  const requests = profiles.reduce((sum, profile) => sum + profile.totalRequestsProcessed, 0);
  const closed = profiles.reduce((sum, profile) => sum + profile.totalClosed, 0);
  const violations = profiles.reduce((sum, profile) => sum + profile.statutoryViolationCount, 0);
  const latencySamples = profiles.reduce((sum, profile) => sum + profile.validLatencySampleSize, 0);
  const latencyTotal = profiles.reduce((sum, profile) => sum + (profile.averageClosureDays ?? 0) * profile.validLatencySampleSize, 0);
  return { requests, closureRate: requests ? closed / requests : null, violationRate: requests ? violations / requests : null, averageClosureDays: latencySamples ? latencyTotal / latencySamples : null };
}

function ComparisonMetric({ label, current, prior, percent = false, inverse = false, loading = false }: { label: string; current: number | null; prior: number | null; percent?: boolean; inverse?: boolean; loading?: boolean }) {
  const delta = current === null || prior === null || prior === 0 ? null : (current - prior) / Math.abs(prior);
  // An unchanged figure is neither better nor worse, so it takes no tone.
  const favorable = delta === null || Math.abs(delta) < 0.0005 ? null : inverse ? delta < 0 : delta > 0;
  const format = (value: number) => percent ? `${(value * 100).toFixed(1)}%` : Number.isInteger(value) ? value.toLocaleString() : value.toFixed(1);
  return (
    <div className="min-w-0 rounded-md bg-raised/60 px-3 py-2">
      <p className="truncate text-caption text-muted" title={label}>{label}</p>
      {loading ? (
        <div aria-hidden="true" className="mt-1 space-y-1.5"><div className="skeleton h-5 w-16 rounded" /><div className="skeleton h-3 w-24 rounded" /></div>
      ) : (
        <>
          <p className={cn('mt-1 text-heading tabular-nums', current === null ? 'font-normal text-muted' : 'text-fg')}>{current === null ? 'Not measured' : format(current)}</p>
          <p className={cn('text-caption tabular-nums', favorable === null ? 'text-muted' : favorable ? 'text-success' : 'text-warning')}>
            {delta === null ? 'No comparable prior baseline' : favorable === null ? 'No change vs prior' : `${delta > 0 ? '+' : ''}${(delta * 100).toFixed(1)}% vs prior`}
          </p>
        </>
      )}
    </div>
  );
}

function WorkspaceSourcesUnavailable({ detail }: { detail: string | null }) {
  return (
    <section className="app-panel p-5" aria-labelledby="sources-unavailable-title">
      <div className="flex items-start gap-3">
        <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-raised text-fg-secondary"><Icon name="database" size="md" /></span>
        <div className="min-w-0">
          <h3 id="sources-unavailable-title" role="alert" className="text-heading text-fg">Your saved sources didn’t load</h3>
          <p className="mt-1 max-w-prose text-body text-muted">
            The local evidence store is busy or unavailable. Your sources and documents are not affected.
          </p>
          {detail && <p className="mt-2 break-words text-caption text-muted">{displayError(detail)}</p>}
        </div>
      </div>
    </section>
  );
}

/** The code prefix ("DATABASE_UNAVAILABLE: ") is for the activity log, not the reader. */
function displayError(detail: string): string {
  const separator = detail.indexOf(': ');
  if (separator <= 0) return detail;
  const prefix = detail.slice(0, separator);
  return /^[A-Z_]+$/.test(prefix) || /^[A-Za-z]+Error$/.test(prefix) ? displayError(detail.slice(separator + 2)) : detail;
}

function SignalSummary({ outlierDepartments, monitoredDepartments }: { outlierDepartments: number; monitoredDepartments: number }) {
  const profiles = useAppStore((state) => state.agencyProfiles);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const priority = useMemo(
    () => profiles
      .filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey)
      .sort((left, right) => {
        const leftScore = left.statutoryViolationCount * 2 + left.suspiciousFastClosureCount;
        const rightScore = right.statutoryViolationCount * 2 + right.suspiciousFastClosureCount;
        return rightScore - leftScore;
      })
      .slice(0, 4),
    [activePortalKey, profiles],
  );
  const unmeasured = analyticsError !== null && profiles.length === 0;

  const inspect = (portalKey: string, agencyId: number) => {
    setActivePortal(portalKey);
    setSearchFilters('', agencyId, null, null);
    setWorkspaceView('evidence');
  };

  return (
    <aside aria-labelledby="signal-summary-title" className="flex min-h-[390px] flex-col overflow-hidden rounded-lg border border-line">
      <div className="border-b border-line px-4 py-3">
        <h4 id="signal-summary-title" className="text-body font-medium text-fg">Departments to look at first</h4>
      </div>
      <dl className="grid grid-cols-2 divide-x divide-line border-b border-line">
        <div className="p-4">
          <dt className="text-caption text-muted">Outliers (≥2σ)</dt>
          <dd className={cn('mt-1 text-metric tabular-nums', unmeasured ? 'text-body text-muted' : 'text-fg')}>{unmeasured ? 'Not measured' : outlierDepartments}</dd>
        </div>
        <div className="p-4">
          <dt className="text-caption text-muted">Departments mapped</dt>
          <dd className={cn('mt-1 text-metric tabular-nums', unmeasured ? 'text-body text-muted' : 'text-fg')}>{unmeasured ? 'Not measured' : monitoredDepartments}</dd>
        </div>
      </dl>
      <div className="min-h-0 flex-1 p-3">
        {priority.length === 0 && analyticsError ? (
          <AnalyticsUnavailable heading="Department summary unavailable" />
        ) : priority.length === 0 ? (
          <div className="grid h-full min-h-40 place-items-center p-4 text-center">
            <div>
              <p className="text-body font-medium text-fg-secondary">No department profiles yet</p>
              <p className="mt-1 text-caption text-muted">This updates as agency profiles are calculated.</p>
            </div>
          </div>
        ) : (
          <ul className="space-y-1">
            {priority.map((profile) => (
              <li key={`${profile.portalSubdomain}-${profile.agencyId}`}>
                <button type="button" onClick={() => inspect(profile.portalSubdomain, profile.agencyId)} className="focus-ring group flex w-full items-center gap-3 rounded-md px-2 py-2 text-left transition-colors duration-fast hover:bg-raised">
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-body font-medium text-fg">{profile.agencyName}</span>
                    <span className="mt-0.5 block text-caption tabular-nums text-muted">
                      {profile.statutoryViolationCount} counted deadline violation{profile.statutoryViolationCount === 1 ? '' : 's'} · {profile.suspiciousFastClosureCount} fast closure{profile.suspiciousFastClosureCount === 1 ? '' : 's'} · {profile.performanceZScore === null ? 'deviation not measured' : `${profile.performanceZScore >= 0 ? '+' : ''}${profile.performanceZScore.toFixed(1)}σ vs peers`}
                    </span>
                  </span>
                  <Icon name="chevron" size="xs" className="text-muted group-hover:text-fg-secondary" />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </aside>
  );
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
