import { useMemo } from 'react';
import { useAppStore } from '../../store/useAppStore';
import AnalyticsUnavailable from './AnalyticsUnavailable';
import { cn } from '../../utils/cn';
import { ShapeGlyph } from '../ui/shapeGlyph';

export default function LifecycleFunnel() {
  const profiles = useAppStore((state) => state.agencyProfiles);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const searchTerm = useAppStore((state) => state.searchTerm);
  const selectedAgencyId = useAppStore((state) => state.selectedAgencyId);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const stages = useMemo(() => {
    const total = profiles.reduce((sum, profile) => sum + profile.totalRequestsProcessed, 0);
    const closed = profiles.reduce((sum, profile) => sum + profile.totalClosed, 0);
    const statutory = profiles.reduce((sum, profile) => sum + profile.statutoryViolationCount, 0);
    const fast = profiles.reduce((sum, profile) => sum + profile.suspiciousFastClosureCount, 0);
    return [
      { label: 'Ingested requests', value: total, policy: null, status: null, predicate: 'All indexed requests', color: 'var(--viz-indigo)' },
      { label: 'Closed requests', value: closed, policy: null, status: 'closed', predicate: 'Status is closed', color: 'var(--viz-cyan)' },
      { label: 'Statutory exceptions', value: statutory, policy: 'STATUTORY_LATENCY_VIOLATION', status: null, predicate: 'Statutory latency predicate matches', color: 'var(--viz-amber)' },
      { label: 'Fast-drop closures', value: fast, policy: 'SUSPICIOUSLY_FAST_CLOSURE', status: 'closed', predicate: 'Status is closed AND rapid-closure predicate matches', color: 'var(--viz-red)' },
    ];
  }, [profiles]);
  const maximum = Math.max(1, ...stages.map((stage) => stage.value));
  const hasData = stages.some((stage) => stage.value > 0);

  return (
    <section className="app-panel min-h-[360px] overflow-hidden p-5">
      <div className="app-eyebrow">Evidence lifecycle</div>
      <div className="mt-1 text-label text-slate-400">Select a stage to open its matching records. Multiple predicates use AND semantics.</div>
      {!hasData && analyticsError ? (
        <AnalyticsUnavailable className="mt-6" heading="Lifecycle counts unavailable" />
      ) : (
      <div className="mt-6 space-y-4">
        {stages.map((stage, index) => {
          const width = hasData ? Math.max(4, (stage.value / maximum) * (100 - index * 7)) : 0;
          return (
            <button
              key={stage.label}
              type="button"
              className="focus-ring group block w-full rounded-lg text-left"
              onClick={() => {
                setSearchFilters(searchTerm, selectedAgencyId, stage.status, stage.policy);
                setWorkspaceView('evidence');
              }}
            >
              <div className="mb-1 flex items-center justify-between font-mono text-label">
                <span className="text-slate-400 transition-colors group-hover:text-slate-200">{stage.label}</span>
                <span className={cn('flex items-center gap-1', stage.policy ? 'text-amber-300' : 'text-slate-400')}>
                  {stage.policy && <FlagMark label="Policy flagged" />}
                  {stage.value.toLocaleString()}
                </span>
              </div>
              <div className="mb-1 text-caption text-slate-400">{stage.predicate}</div>
              <div className="h-8 overflow-hidden rounded-lg border border-slate-800/90 bg-slate-950/60 p-1">
                {hasData && (
                  <div
                    className="flex h-full items-center justify-end rounded-md px-2 text-caption font-bold shadow-[0_0_18px_rgba(15,23,42,0.2)] transition-[width,filter] duration-500 group-hover:brightness-110"
                    style={{ width: `${width}%`, color: 'var(--viz-on-color)', background: stage.color }}
                  >
                    {((stage.value / maximum) * 100).toFixed(1)}%
                  </div>
                )}
              </div>
            </button>
          );
        })}
      </div>
      )}
    </section>
  );
}

// A triangle beside a flagged value, so the flag survives when amber and slate read alike.
function FlagMark({ label }: { label: string }) {
  return (
    <svg viewBox="-6 -6 12 12" className="h-2 w-2 shrink-0" role="img" aria-label={label}>
      <ShapeGlyph shape="triangle" cx={0} cy={0} radius={5} fill="currentColor" />
    </svg>
  );
}
