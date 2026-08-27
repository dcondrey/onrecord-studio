import { useMemo } from 'react';
import AnomalyRadar from '../charts/AnomalyRadar';
import LifecycleFunnel from '../charts/LifecycleFunnel';
import PerformanceScatter from '../charts/PerformanceScatter';
import TemporalMap from '../charts/TemporalMap';
import AnalyticsUnavailable from '../charts/AnalyticsUnavailable';
import { useAppStore } from '../../store/useAppStore';
import MetricsGrid from './MetricsGrid';
import Icon from '../ui/icons';

export default function OverviewDashboard() {
  // FEATURE(overview): Add a comparison period selector so investigators can
  // compare current response behavior against a prior interval.
  const portals = useAppStore((state) => state.portals);
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
            <div className="app-eyebrow flex items-center gap-2 text-cyan-300/60">
              <Icon name="sparkles" className="h-3.5 w-3.5" /> Investigative intelligence
            </div>
            <h2 className="mt-2 text-[26px] font-semibold tracking-[-0.035em] text-slate-100 sm:text-[30px]">See the story behind the records.</h2>
            <p className="mt-1.5 max-w-3xl text-[11px] leading-relaxed text-slate-500">
              A live view of response behavior, statutory risk, and cross-agency signals across {activePortalKey ? 'the selected jurisdiction' : 'every connected jurisdiction'}.
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {analyticsError ? (
              // `analyticsUpdatedAt` only advances on success, so the unchanged badge would keep
              // claiming "Live" over a figure that stopped updating.
              <span className="overview-freshness hidden items-center gap-2 whitespace-nowrap rounded-full border border-amber-500/30 px-3 py-1.5 text-[9px] text-amber-200/90 sm:flex" title={`Analytics refresh failed: ${analyticsError}`}><span className="h-1.5 w-1.5 rounded-full bg-amber-400" /> {analyticsUpdatedAt ? `Stale · last updated ${formatRelativeTime(analyticsUpdatedAt)}` : 'Analytics unavailable'}</span>
            ) : analyticsUpdatedAt ? (
              <span className="overview-freshness hidden items-center gap-2 whitespace-nowrap rounded-full border px-3 py-1.5 text-[9px] text-slate-600 sm:flex" title={`Analytics updated ${formatRelativeTime(analyticsUpdatedAt)}`}><span className="h-1.5 w-1.5 rounded-full bg-emerald-400" /> Live · {formatRelativeTime(analyticsUpdatedAt)}</span>
            ) : null}
            <button type="button" onClick={() => setWorkspaceView('evidence')} className="secondary-action focus-ring flex shrink-0 items-center gap-2 whitespace-nowrap rounded-xl border px-3.5 py-2.5 text-[10px] font-semibold text-slate-300 transition hover:text-cyan-200">
              Review evidence <Icon name="arrow" className="h-3.5 w-3.5" />
            </button>
          </div>
        </section>

        {portals.length === 0 && <WelcomePanel />}

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
      </div>
    </div>
  );
}

function WelcomePanel() {
  const focusSourceInput = () => {
    const desktopInput = document.getElementById('portal-url-desktop');
    if (desktopInput?.getClientRects().length) {
      desktopInput.focus();
      return;
    }
    document.querySelector<HTMLButtonElement>('button[aria-label="Connect a data source"]')?.click();
    window.setTimeout(() => document.getElementById('portal-url-mobile')?.focus(), 0);
  };

  return (
    <section className="onboarding-panel relative overflow-hidden rounded-2xl border p-5 sm:p-6">
      <div className="onboarding-grid pointer-events-none absolute inset-0" aria-hidden="true" />
      <div className="onboarding-glow pointer-events-none absolute -right-20 -top-28 h-80 w-80 rounded-full" aria-hidden="true" />
      <div className="relative grid gap-6 lg:grid-cols-[minmax(0,1fr)_360px] lg:items-center">
        <div>
          <div className="flex items-center gap-2 text-[10px] font-semibold text-cyan-300"><Icon name="database" className="h-4 w-4" /> New investigation</div>
          <h3 className="mt-2 max-w-2xl text-lg font-semibold tracking-[-0.02em] text-slate-100">Turn scattered public information into a connected evidence record.</h3>
          <p className="mt-1.5 max-w-2xl text-[10px] leading-relaxed text-slate-500">Add a website, report library, open-data catalog, API, local file, or pasted material. OnRecord identifies the source, preserves it locally, and builds searchable intelligence as ingestion continues.</p>
          <div className="mt-4 flex flex-wrap items-center gap-2">
            <button type="button" onClick={focusSourceInput} className="primary-action focus-ring flex items-center gap-2 rounded-lg px-3.5 py-2 text-[10px] font-semibold text-white">
              Add your first source <Icon name="arrow" className="h-3.5 w-3.5" />
            </button>
            <SourceType icon="globe" label="Web & APIs" />
            <SourceType icon="filePlus" label="Files" />
            <SourceType icon="clipboard" label="Pasted content" />
          </div>
        </div>
        <div className="welcome-flow relative grid grid-cols-3 gap-2 text-center">
          <span className="welcome-flow-line" aria-hidden="true" />
          <WelcomeStep number="01" label="Discover" />
          <WelcomeStep number="02" label="Preserve" />
          <WelcomeStep number="03" label="Connect" />
        </div>
      </div>
    </section>
  );
}

function SourceType({ icon, label }: { icon: 'globe' | 'filePlus' | 'clipboard'; label: string }) {
  return <span className="source-type-chip flex items-center gap-1.5 rounded-full border px-2.5 py-1.5 text-[8px] font-medium text-slate-500"><Icon name={icon} className="h-3.5 w-3.5" /> {label}</span>;
}

function WelcomeStep({ number, label }: { number: string; label: string }) {
  return (
    <div className="welcome-step relative rounded-xl border px-3 py-4">
      <div className="welcome-step-number mx-auto grid h-8 w-8 place-items-center rounded-full font-mono text-[9px] font-semibold text-cyan-300">{number}</div>
      <div className="mt-2 text-[9px] font-semibold text-slate-300">{label}</div>
    </div>
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
      <div className="border-b border-slate-800/70 px-5 py-4">
        <div className="app-eyebrow">Signal brief</div>
        <h2 className="mt-1 text-[13px] font-semibold text-slate-200">What needs attention</h2>
      </div>
      <div className="grid grid-cols-2 gap-px border-b border-slate-800/70 bg-slate-800/70">
        <div className="bg-slate-900/70 p-4">
          <div className="font-mono text-2xl font-semibold text-rose-300">{analyticsError && profiles.length === 0 ? '—' : criticalDepartments}</div>
          <div className="mt-1 text-[8px] uppercase tracking-wider text-slate-600">Critical outliers</div>
        </div>
        <div className="bg-slate-900/70 p-4">
          <div className="font-mono text-2xl font-semibold text-cyan-300">{analyticsError && profiles.length === 0 ? '—' : monitoredDepartments}</div>
          <div className="mt-1 text-[8px] uppercase tracking-wider text-slate-600">Departments mapped</div>
        </div>
      </div>
      <div className="min-h-0 flex-1 p-4">
        {priority.length === 0 && analyticsError ? (
          <AnalyticsUnavailable heading="Signal brief unavailable" />
        ) : priority.length === 0 ? (
          <div className="grid h-full min-h-40 place-items-center rounded-xl border border-dashed border-slate-800 bg-slate-950/25 p-4 text-center">
            <div>
              <Icon name="shield" className="mx-auto h-5 w-5 text-slate-700" />
              <p className="mt-2 text-[10px] font-medium text-slate-500">No risk signals to summarize yet</p>
              <p className="mt-1 text-[9px] leading-relaxed text-slate-700">The brief updates as agency profiles are calculated.</p>
            </div>
          </div>
        ) : (
          <div className="space-y-2">
            {priority.map((profile) => (
              <button key={`${profile.portalSubdomain}-${profile.agencyId}`} type="button" onClick={() => inspect(profile.portalSubdomain, profile.agencyId)} className="focus-ring group flex w-full items-center gap-3 rounded-xl border border-slate-800/70 bg-slate-950/30 px-3 py-3 text-left transition hover:border-amber-400/20 hover:bg-amber-400/[0.04]">
                <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-amber-400/[0.08] font-mono text-[10px] font-semibold text-amber-300">{profile.statutoryViolationCount}</span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[10px] font-medium text-slate-300">{profile.agencyName}</span>
                  <span className="mt-0.5 block text-[8px] text-slate-600">{profile.suspiciousFastClosureCount} rapid closures · z {profile.performanceZScore === null ? 'not computed' : profile.performanceZScore.toFixed(1)}</span>
                </span>
                <Icon name="chevron" className="h-3 w-3 text-slate-700 transition group-hover:translate-x-0.5 group-hover:text-amber-300" />
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
