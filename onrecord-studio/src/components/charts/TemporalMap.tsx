import { useMemo } from 'react';
import { useAppStore } from '../../store/useAppStore';
import AnalyticsUnavailable from './AnalyticsUnavailable';

export default function TemporalMap() {
  // FEATURE(analytics): Add a time-window control and neighborhood drill-down
  // instead of keeping the correlation window fixed at 48 hours.
  const correlations = useAppStore((state) => state.crossAgencyCorrelations);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  // PERF(analytics): Keep the visible-cell cap until a canvas/virtualized grid
  // is introduced for larger correlation result sets.
  const cells = useMemo(
    () => [...correlations].sort((left, right) => right.complaintCount48h - left.complaintCount48h).slice(0, 18),
    [correlations],
  );
  const maximum = Math.max(1, ...cells.map((cell) => cell.complaintCount48h));

  return (
    <section className="app-panel flex min-h-[360px] flex-col overflow-hidden p-5">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="app-eyebrow">48-hour signal density</div>
          <div className="mt-1 text-[10px] text-slate-500">311 activity preceding enforcement-linked requests.</div>
        </div>
        <div className="rounded-lg border border-slate-800 bg-slate-950/50 px-2.5 py-1.5 font-mono text-[8px] text-slate-600">{cells.length} clusters</div>
      </div>
      {cells.length === 0 && analyticsError ? (
        <AnalyticsUnavailable className="mt-4 flex-1" heading="Correlation window unavailable" />
      ) : cells.length === 0 ? (
        <div className="chart-surface data-grid relative mt-4 flex-1 overflow-hidden rounded-xl border border-slate-800 bg-slate-950/40 p-4">
          <div className="grid h-full min-h-[230px] grid-cols-4 gap-2 opacity-60" aria-hidden="true">
            {[0.18, 0.32, 0.12, 0.42, 0.28, 0.56, 0.22, 0.36, 0.14, 0.4, 0.68, 0.24].map((opacity, index) => (
              <span key={`${opacity}-${index}`} className="rounded-lg border border-cyan-400/15" style={{ background: `rgba(6, 182, 212, ${opacity})` }} />
            ))}
          </div>
          <div className="absolute inset-0 grid place-items-center p-6 text-center">
            <div className="chart-ready-card max-w-xs rounded-xl border px-4 py-3 backdrop-blur-xl">
              <div className="text-[10px] font-semibold text-slate-300">Temporal correlation ready</div>
              <div className="mt-1 text-[9px] leading-relaxed text-slate-600">Add neighborhood and timestamp data to reveal activity clusters around public-record events.</div>
            </div>
          </div>
        </div>
      ) : (
        <div className="mt-4 grid flex-1 grid-cols-2 gap-2 overflow-y-auto pr-1 sm:grid-cols-3">
          {cells.map((cell) => {
            const intensity = cell.complaintCount48h / maximum;
            return (
              <button
                key={`${cell.portalKey}-${cell.requestId}-${cell.neighborhoodBlock}`}
                type="button"
                className="focus-ring group relative min-h-24 overflow-hidden rounded-xl border border-slate-800 p-3 text-left transition hover:-translate-y-0.5 hover:border-cyan-500/40"
                style={{ background: `linear-gradient(145deg, rgba(6, 182, 212, ${0.05 + intensity * 0.28}), var(--chart-cell-end))` }}
                onClick={() => {
                  setActivePortal(cell.portalKey);
                  setSearchFilters('', cell.agencyId, null, 'CROSS_AGENCY_311_CORRELATION');
                  setWorkspaceView('evidence');
                }}
              >
                <span className="absolute bottom-0 left-0 h-0.5 bg-cyan-400/70" style={{ width: `${Math.max(8, intensity * 100)}%` }} />
                <div className="truncate text-[10px] font-semibold text-slate-300">{cell.neighborhoodBlock}</div>
                <div className="mt-2 font-mono text-lg font-bold text-cyan-300">{cell.complaintCount48h}</div>
                <div className="mt-1 truncate font-mono text-[8px] text-slate-500">Request {cell.requestId} · {cell.keywordSignalCount} signals</div>
              </button>
            );
          })}
        </div>
      )}
    </section>
  );
}
