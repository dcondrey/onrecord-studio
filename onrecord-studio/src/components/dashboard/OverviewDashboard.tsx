import { useEffect, useMemo, useState, type ReactNode } from 'react';
import AnomalyRadar from '../charts/AnomalyRadar';
import LifecycleFunnel from '../charts/LifecycleFunnel';
import PerformanceScatter from '../charts/PerformanceScatter';
import TemporalMap from '../charts/TemporalMap';
import AnalyticsUnavailable from '../charts/AnalyticsUnavailable';
import { useAppStore } from '../../store/useAppStore';
import MetricsGrid from './MetricsGrid';
import Icon from '../ui/icons';
import SanDiegoWorkspace from './SanDiegoWorkspace';
import InvestigationDesk from './InvestigationDesk';
import ResearchWorkspace from './ResearchWorkspace';
import ProfessionalWorkbench from './ProfessionalWorkbench';
import DataMaintenance from './DataMaintenance';
import ResearchSnapshotExport from './ResearchSnapshotExport';
import RegistrySnapshots from './RegistrySnapshots';
import AutomaticDiscovery from './AutomaticDiscovery';
import { invokeCommand } from '../../ipc';
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
import { useStarterRunStore } from '../../store/useStarterRunStore';

export default function OverviewDashboard() {
  const portals = useAppStore((state) => state.portals);
  const [showAnalytics, setShowAnalytics] = useState(false);
  const [workbench, setWorkbench] = useState<{ reportId: string; review: boolean } | null>(null);
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
  const criticalDepartments = scopedProfiles.filter(
    (profile) => profile.performanceZScore !== null && profile.performanceZScore >= 2,
  ).length;
  const monitoredDepartments = scopedProfiles.length;

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-[1800px] space-y-4 p-4 sm:p-5 lg:p-6">
        <section className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
          <div className="min-w-0">
            <div className="app-eyebrow flex items-center gap-2 text-accent">
              <Icon name="sparkles" className="h-3.5 w-3.5" /> San Diego public records
            </div>
            <h2 className="mt-2 text-[26px] font-semibold tracking-[-0.035em] text-fg sm:text-[30px]">{workbench ? 'Develop the case' : 'Your investigation briefing'}</h2>
            <p className="mt-1.5 max-w-3xl text-label leading-relaxed text-muted">
              Investigate homelessness spending, encampment abatements, public bathrooms, and City budget priorities.
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {showAnalytics && (analyticsError ? (
              // `analyticsUpdatedAt` only advances on success, so the unchanged badge would keep
              // claiming "Live" over a figure that stopped updating.
              <span className="overview-freshness hidden items-center gap-2 whitespace-nowrap rounded-full border border-warning/30 px-3 py-1.5 text-caption text-warning sm:flex" title={`Analytics refresh failed: ${analyticsError}`}><span className="h-1.5 w-1.5 rounded-full bg-warning" /> {analyticsUpdatedAt ? `Stale · last updated ${formatRelativeTime(analyticsUpdatedAt)}` : 'Analytics unavailable'}</span>
            ) : analyticsUpdatedAt ? (
              <span className="overview-freshness hidden items-center gap-2 whitespace-nowrap rounded-full border px-3 py-1.5 text-caption text-muted sm:flex" title={`Analytics updated ${formatRelativeTime(analyticsUpdatedAt)}`}><span className="h-1.5 w-1.5 rounded-full bg-success" /> Live · {formatRelativeTime(analyticsUpdatedAt)}</span>
            ) : null)}
            {portals.length > 0 && <SinceControl />}
            <button type="button" onClick={() => setWorkspaceView('evidence')} className="secondary-action focus-ring flex shrink-0 items-center gap-2 whitespace-nowrap rounded-xl border px-3.5 py-2.5 text-label font-semibold text-fg-secondary transition hover:text-accent">
              Review evidence <Icon name="arrow" className="h-3.5 w-3.5" />
            </button>
          </div>
        </section>


        {workspaceInitializationStatus === 'loading' ? (
          <WorkspaceSourcesLoading />
        ) : workspaceInitializationStatus === 'error' ? (
          <WorkspaceSourcesUnavailable detail={workspaceInitializationError} />
        ) : portals.length === 0 || starterRunShown ? (
          <section className="app-panel p-5"><StarterSets /></section>
        ) : null}

        <PostIngestBrief />
        {workbench ? <>
          <button type="button" className="analysis-button" onClick={() => setWorkbench(null)}>Return to investigation briefing</button>
          <ProfessionalWorkbench key={`${workbench.reportId}:${workbench.review}`} initialReportId={workbench.reportId} initialTab={workbench.review ? 'Evidence review' : 'Cases and follow-through'} />
        </> : <>
          <SubjectDossier />
          {portals.length > 0 && <AttentionFeed />}
          <section aria-label="Following" className="grid grid-cols-1 items-start gap-4 2xl:grid-cols-2">
            <WatchlistPanel />
            <StandingSearchPanel />
          </section>
          <ResearchWorkspace onReview={(reportId) => setWorkbench({ reportId, review: true })} />
          <section className="app-panel flex flex-wrap items-center justify-between gap-3 p-4"><div><h3 className="text-sm font-semibold text-fg">Develop a lead into a durable case</h3><p className="mt-1 text-label leading-4 text-muted">Track requests, productions and commitments; reconcile spending; challenge the evidence.</p></div><button type="button" className="analysis-button" onClick={() => setWorkbench({ reportId: '', review: false })}>Open casework and research tools</button></section>
        </>}
        <ToolDrawer label="Shared language and topics across corpora"><CrossCorpusExplorer /></ToolDrawer>
        <ToolDrawer label="Topics: encampments, belongings, police spending, public restrooms"><TopicPacks /></ToolDrawer>
        <ToolDrawer label="Plan an investigation or draft a request manually"><InvestigationDesk /></ToolDrawer>
        <ToolDrawer label="Collection, coverage and storage"><AutomaticDiscovery /><div className="mt-4"><DataMaintenance /></div><ResearchSnapshotExport /><RegistrySnapshots /></ToolDrawer>
        <ToolDrawer label="Browse sources and add records"><SanDiegoWorkspace /></ToolDrawer>

        {portals.length > 0 && <button type="button" onClick={() => setShowAnalytics((visible) => !visible)} aria-expanded={showAnalytics} className="secondary-action focus-ring rounded-xl border px-4 py-2 text-label leading-4 text-fg-secondary">{showAnalytics ? 'Hide response analytics' : 'Show response analytics'}</button>}
        {showAnalytics && <>
        <PeriodComparison activePortalKey={activePortalKey} />

        <MetricsGrid />

        <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1fr)_300px]">
          <PerformanceScatter />
          <SignalSummary criticalDepartments={criticalDepartments} monitoredDepartments={monitoredDepartments} />
        </div>

        <section aria-label="Investigative analytics" className="grid grid-cols-1 gap-4 lg:grid-cols-2 2xl:grid-cols-[1.15fr_.8fr_1.05fr]">
          <AnomalyRadar />
          <LifecycleFunnel />
          <div className="lg:col-span-2 2xl:col-span-1"><TemporalMap /></div>
        </section>
        </>}
      </div>
    </div>
  );
}

function ToolDrawer({ label, children }: { label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return <details className="app-panel p-4" onToggle={(event) => setOpen(event.currentTarget.open)}><summary className="text-sm font-semibold text-fg-secondary">{label}</summary>{open && <div className="mt-4">{children}</div>}</details>;
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
      if (comparisonGate.isCurrent(request)) setError(String(reason));
    }).finally(() => {
      if (comparisonGate.isCurrent(request)) setIsLoading(false);
    });
    return comparisonGate.invalidate;
  }, [activePortalKey, comparisonGate, days]);

  const currentSummary = summarizePeriod(current);
  const priorSummary = summarizePeriod(prior);
  return <section className="app-panel flex flex-col gap-3 p-4 lg:flex-row lg:flex-wrap lg:items-center lg:justify-between" aria-label="Response behavior period comparison"><div className="min-w-0"><div className="app-eyebrow">Period comparison</div><p className="mt-1 text-caption text-muted">Current interval versus the immediately preceding interval, based on request creation time.</p></div><label className="secondary-action w-fit rounded-lg border px-2.5 py-1.5 text-caption text-muted">Interval <select value={days} onChange={(event) => setDays(Number(event.target.value))} className="bg-transparent font-mono text-fg"><option value={7}>7 days</option><option value={30}>30 days</option><option value={90}>90 days</option><option value={365}>365 days</option></select></label>{error ? <div role="alert" className="text-caption text-danger">Comparison unavailable: {error}</div> : <div className="grid min-w-0 flex-1 basis-full gap-2 [grid-template-columns:repeat(auto-fit,minmax(7rem,1fr))] lg:basis-auto"><ComparisonMetric label="Requests" current={currentSummary.requests} prior={priorSummary.requests} loading={isLoading} /><ComparisonMetric label="Closure rate" current={currentSummary.closureRate} prior={priorSummary.closureRate} percent loading={isLoading} /><ComparisonMetric label="Deadline rate" current={currentSummary.violationRate} prior={priorSummary.violationRate} percent inverse loading={isLoading} /><ComparisonMetric label="Avg. closure days" current={currentSummary.averageClosureDays} prior={priorSummary.averageClosureDays} inverse loading={isLoading} /></div>}</section>;
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
  const favorable = delta === null ? null : inverse ? delta <= 0 : delta >= 0;
  const format = (value: number) => percent ? `${(value * 100).toFixed(1)}%` : Number.isInteger(value) ? value.toLocaleString() : value.toFixed(1);
  return <div className="min-w-0 rounded-lg border border-line bg-canvas/40 px-3 py-2"><div className="truncate text-caption text-muted" title={label}>{label}</div><div className="mt-1 break-words font-mono text-sm leading-tight text-fg">{loading ? '—' : current === null ? 'Not measured' : format(current)}</div><div className={favorable === null ? 'text-caption leading-tight text-muted' : favorable ? 'text-caption leading-tight text-success' : 'text-caption leading-tight text-warning'}>{loading ? 'Loading…' : delta === null ? 'No comparable prior baseline' : `${delta >= 0 ? '+' : ''}${(delta * 100).toFixed(1)}% vs prior`}</div></div>;
}

function WorkspaceSourcesLoading() {
  return (
    <section className="onboarding-panel rounded-2xl border p-5 sm:p-6" role="status">
      <div className="flex items-center gap-3">
        <span className="h-4 w-4 animate-spin rounded-full border-2 border-line-strong border-t-cyan-300" aria-hidden="true" />
        <div>
          <h3 className="text-sm font-semibold text-fg-secondary">Loading saved sources</h3>
          <p className="mt-1 text-label text-muted">Checking the local workspace before showing source-specific findings.</p>
        </div>
      </div>
    </section>
  );
}

function WorkspaceSourcesUnavailable({ detail }: { detail: string | null }) {
  return (
    <section className="rounded-2xl border border-danger/30 bg-danger/5 p-5 sm:p-6" role="alert">
      <div className="flex items-start gap-3">
        <Icon name="database" className="mt-0.5 h-4 w-4 text-danger" />
        <div>
          <h3 className="text-sm font-semibold text-danger">Saved sources unavailable</h3>
          <p className="mt-1 text-label leading-relaxed text-muted">
            OnRecord could not load the local source catalog, so this is not a first-run workspace. Existing sources may still be present; review the activity log and retry after the storage error is resolved.
          </p>
          {detail && <p className="mt-2 font-mono text-caption text-danger">{detail}</p>}
        </div>
      </div>
    </section>
  );
}

function SignalSummary({ criticalDepartments, monitoredDepartments }: { criticalDepartments: number; monitoredDepartments: number }) {
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

  const inspect = (portalKey: string, agencyId: number) => {
    setActivePortal(portalKey);
    setSearchFilters('', agencyId, null, null);
    setWorkspaceView('evidence');
  };

  return (
    <aside className="app-panel flex min-h-[390px] flex-col overflow-hidden">
      <div className="border-b border-line px-5 py-4">
        <div className="app-eyebrow">Signal brief</div>
        <h2 className="mt-1 text-label font-semibold text-fg">What needs attention</h2>
      </div>
      <div className="grid grid-cols-2 gap-px border-b border-line bg-raised/70">
        <div className="bg-surface/70 p-4">
          <div className="font-mono text-2xl font-semibold text-danger">{analyticsError && profiles.length === 0 ? '—' : criticalDepartments}</div>
          <div className="mt-1 text-caption uppercase tracking-wider text-muted">Critical outliers</div>
        </div>
        <div className="bg-surface/70 p-4">
          <div className="font-mono text-2xl font-semibold text-accent">{analyticsError && profiles.length === 0 ? '—' : monitoredDepartments}</div>
          <div className="mt-1 text-caption uppercase tracking-wider text-muted">Departments mapped</div>
        </div>
      </div>
      <div className="min-h-0 flex-1 p-4">
        {priority.length === 0 && analyticsError ? (
          <AnalyticsUnavailable heading="Signal brief unavailable" />
        ) : priority.length === 0 ? (
          <div className="grid h-full min-h-40 place-items-center rounded-xl border border-dashed border-line bg-canvas/40 p-4 text-center">
            <div>
              <Icon name="shield" className="mx-auto h-5 w-5 text-muted" />
              <p className="mt-2 text-label font-medium text-muted">No risk signals to summarize yet</p>
              <p className="mt-1 text-caption leading-relaxed text-muted">The brief updates as agency profiles are calculated.</p>
            </div>
          </div>
        ) : (
          <div className="space-y-2">
            {priority.map((profile) => (
              <button key={`${profile.portalSubdomain}-${profile.agencyId}`} type="button" onClick={() => inspect(profile.portalSubdomain, profile.agencyId)} className="focus-ring group flex w-full items-center gap-3 rounded-xl border border-control bg-canvas/40 px-3 py-3 text-left transition hover:border-warning/30 hover:bg-warning/5">
                <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-warning/10 font-mono text-label font-semibold text-warning">{profile.statutoryViolationCount}</span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-label font-medium text-fg-secondary">{profile.agencyName}</span>
                  <span className="mt-0.5 block text-caption text-muted">{profile.suspiciousFastClosureCount} rapid closures · z {profile.performanceZScore === null ? 'not computed' : profile.performanceZScore.toFixed(1)}</span>
                </span>
                <Icon name="chevron" className="h-3 w-3 text-muted transition group-hover:translate-x-0.5 group-hover:text-warning" />
              </button>
            ))}
          </div>
        )}
      </div>
    </aside>
  );
}

function formatRelativeTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return 'recently';
  const seconds = Math.max(0, Math.round((Date.now() - date.valueOf()) / 1000));
  if (seconds < 60) return 'just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  return `${Math.floor(seconds / 3600)}h ago`;
}
