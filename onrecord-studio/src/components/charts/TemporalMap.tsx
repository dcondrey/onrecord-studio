import { useEffect, useMemo, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import AnalyticsUnavailable from './AnalyticsUnavailable';
import AuxiliaryEventImport from '../investigation/AuxiliaryEventImport';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { CrossAgencyCorrelation } from '../../types';

export default function TemporalMap() {
  const correlations = useAppStore((state) => state.crossAgencyCorrelations);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const [windowHours, setWindowHours] = useState(48);
  const [windowCorrelations, setWindowCorrelations] = useState<CrossAgencyCorrelation[]>([]);
  const [neighborhood, setNeighborhood] = useState('');
  const [windowError, setWindowError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isImportOpen, setIsImportOpen] = useState(false);
  const correlationGate = useLatestRequestGate();

  useEffect(() => {
    if (windowHours === 48) {
      correlationGate.invalidate();
      setWindowCorrelations([]);
      setWindowError(null);
      setIsLoading(false);
      return;
    }
    const request = correlationGate.begin();
    setIsLoading(true);
    setWindowError(null);
    void invokeCommand('get_cross_agency_correlations', {
      portalKey: activePortalKey,
      minimumComplaints: 1,
      resultLimit: 2_000,
      windowHours,
    }).then((values) => {
      if (correlationGate.isCurrent(request)) setWindowCorrelations(values);
    }).catch((reason) => {
      if (correlationGate.isCurrent(request)) setWindowError(String(reason));
    }).finally(() => {
      if (correlationGate.isCurrent(request)) setIsLoading(false);
    });
    return correlationGate.invalidate;
  }, [activePortalKey, correlationGate, windowHours]);

  const available = windowHours === 48
    ? correlations.filter((value) => !activePortalKey || value.portalKey === activePortalKey)
    : windowCorrelations;
  const neighborhoods = useMemo(() => [...new Set(available.map((value) => value.neighborhoodBlock))].sort(), [available]);
  const cells = useMemo(
    () => available
      .filter((value) => !neighborhood || value.neighborhoodBlock === neighborhood)
      .sort((left, right) => right.complaintCount - left.complaintCount)
      .slice(0, 60),
    [available, neighborhood],
  );
  const maximum = Math.max(1, ...cells.map((cell) => cell.complaintCount));

  return (
    <section className="app-panel flex min-h-[360px] flex-col overflow-hidden p-5">
      <AuxiliaryEventImport isOpen={isImportOpen} onClose={() => setIsImportOpen(false)} />
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="app-eyebrow">{windowHours}-hour signal density</div>
          <div className="mt-1 text-label text-slate-400">311 activity preceding enforcement-linked requests.</div>
        </div>
        <div className="flex flex-wrap justify-end gap-1.5">
          <label className="rounded-lg border border-slate-800 bg-slate-950/50 px-2 py-1 font-mono text-caption text-slate-400">Window <select value={windowHours} onChange={(event) => setWindowHours(Number(event.target.value))} className="bg-transparent text-slate-300"><option value={6}>6h</option><option value={12}>12h</option><option value={24}>24h</option><option value={48}>48h</option><option value={72}>72h</option><option value={168}>7d</option></select></label>
          <label className="max-w-40 rounded-lg border border-slate-800 bg-slate-950/50 px-2 py-1 font-mono text-caption text-slate-400">Area <select value={neighborhood} onChange={(event) => setNeighborhood(event.target.value)} className="max-w-28 bg-transparent text-slate-300"><option value="">All</option>{neighborhoods.map((value) => <option key={value} value={value}>{value}</option>)}</select></label>
          <div className="rounded-lg border border-slate-800 bg-slate-950/50 px-2.5 py-1.5 font-mono text-caption text-slate-400">{isLoading ? 'Loading…' : `${cells.length} clusters`}</div>
          <button type="button" onClick={() => setIsImportOpen(true)} className="focus-ring rounded-lg border border-cyan-400/25 bg-cyan-500/10 px-2.5 py-1.5 font-mono text-caption font-semibold text-cyan-200">Import events</button>
        </div>
      </div>
      {cells.length === 0 && (windowError || analyticsError) ? (
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
              <div className="text-label font-semibold text-slate-300">Temporal correlation ready</div>
              <div className="mt-1 text-caption leading-relaxed text-slate-400">Import a 311, code-enforcement, or dispatch export to reveal activity clusters around public-record events.</div>
              <button type="button" onClick={() => setIsImportOpen(true)} className="focus-ring mt-2.5 rounded-lg border border-cyan-400/25 bg-cyan-500/10 px-3 py-1.5 text-caption font-semibold text-cyan-200">Import municipal events</button>
            </div>
          </div>
        </div>
      ) : (
        <div className="mt-4 grid flex-1 grid-cols-2 gap-2 overflow-y-auto pr-1 sm:grid-cols-3">
          {cells.map((cell) => {
            const intensity = cell.complaintCount / maximum;
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
                <div className="truncate text-label font-semibold text-slate-300">{cell.neighborhoodBlock}</div>
                <div className="mt-2 font-mono text-lg font-bold text-cyan-300">{cell.complaintCount}</div>
                <div className="mt-1 truncate font-mono text-caption text-slate-400">Request {cell.requestId} · {cell.keywordSignalCount} signals</div>
              </button>
            );
          })}
        </div>
      )}
    </section>
  );
}
