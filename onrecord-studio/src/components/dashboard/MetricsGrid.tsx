import { useMemo } from 'react';
import { useAppStore } from '../../store/useAppStore';
import Icon, { type IconName } from '../ui/icons';

interface MetricCard {
  label: string;
  value: string;
  detail: string;
  ratio: number;
  accent: string;
  bar: string;
  icon: IconName;
}

export default function MetricsGrid() {
  const metrics = useAppStore((state) => state.metrics);
  const profiles = useAppStore((state) => state.agencyProfiles);
  const correlations = useAppStore((state) => state.crossAgencyCorrelations);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const values = useMemo(() => {
    const scoped = profiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey);
    const requests = scoped.reduce((total, profile) => total + profile.totalRequestsProcessed, 0);
    const closed = scoped.reduce((total, profile) => total + profile.totalClosed, 0);
    const statutory = scoped.reduce((total, profile) => total + profile.statutoryViolationCount, 0);
    const fastClosures = scoped.reduce((total, profile) => total + profile.suspiciousFastClosureCount, 0);
    const correlated = correlations.filter((value) => !activePortalKey || value.portalKey === activePortalKey).length;
    return { requests, closed, statutory, fastClosures, correlations: correlated };
  }, [activePortalKey, correlations, profiles]);

  // Zeros here are indistinguishable from real zeros, and three of these four cards state a
  // conclusion about the corpus. With no analytics result there is no conclusion to state.
  const unavailable = analyticsError !== null && profiles.length === 0 && correlations.length === 0;
  const cards: MetricCard[] = [
    {
      label: 'Requests audited',
      value: unavailable ? '—' : values.requests.toLocaleString(),
      detail: `${values.closed.toLocaleString()} closed · ${metrics.pagesProcessed.toLocaleString()} pages ingested`,
      ratio: values.requests > 0 ? values.closed / values.requests : 0,
      accent: 'text-indigo-300',
      bar: 'bg-indigo-400',
      icon: 'database',
    },
    {
      label: 'Deadline exceptions',
      value: unavailable ? '—' : values.statutory.toLocaleString(),
      detail: unavailable
        ? 'Analytics unavailable'
        : values.requests > 0 ? `${formatPercent(values.statutory / values.requests)} of reviewed requests` : 'Statutory review is ready',
      ratio: values.requests > 0 ? values.statutory / values.requests : 0,
      accent: values.statutory > 0 ? 'text-amber-300' : 'text-slate-300',
      bar: 'bg-amber-400',
      icon: 'shield',
    },
    {
      label: 'Rapid closures',
      value: unavailable ? '—' : values.fastClosures.toLocaleString(),
      detail: unavailable
        ? 'Analytics unavailable'
        : values.fastClosures > 0 ? 'Closed within the 5-minute review window' : 'No fast-drop signals detected',
      ratio: values.closed > 0 ? values.fastClosures / values.closed : 0,
      accent: values.fastClosures > 0 ? 'text-rose-300' : 'text-slate-300',
      bar: 'bg-rose-400',
      icon: 'activity',
    },
    {
      label: 'Cross-agency signals',
      value: unavailable ? '—' : values.correlations.toLocaleString(),
      detail: `${formatBytes(metrics.bytesTransferred)} preserved locally`,
      ratio: Math.min(1, values.correlations / Math.max(1, values.requests)),
      accent: values.correlations > 0 ? 'text-cyan-300' : 'text-slate-300',
      bar: 'bg-cyan-400',
      icon: 'sparkles',
    },
  ];

  // FEATURE(metrics): Make each instrument cell keyboard-activatable and route
  // it into the corresponding evidence query with the metric predicate set.

  return (
    <section aria-label="Audit metrics" className="instrument-strip app-panel grid grid-cols-1 overflow-hidden sm:grid-cols-2 xl:grid-cols-4">
      {cards.map((card) => (
        <article key={card.label} className="instrument-cell group relative min-w-0 overflow-hidden p-4">
          <span className={`instrument-accent absolute left-0 top-5 h-8 w-0.5 rounded-full ${card.bar}`} aria-hidden="true" />
          <div className="relative flex items-center justify-between gap-3">
            <div className="app-eyebrow min-w-0 leading-snug">{card.label}</div>
            <div className="metric-icon grid h-8 w-8 shrink-0 place-items-center rounded-[10px] border text-slate-500">
              <Icon name={card.icon} className="h-[17px] w-[17px]" />
            </div>
          </div>
          <div className={`numeric-value relative mt-3 text-[28px] font-semibold leading-none tracking-[-0.045em] ${card.accent}`}>{card.value}</div>
          <div className="relative mt-2 min-h-9 text-[9px] leading-snug text-slate-600">{card.detail}</div>
          <div className="relative mt-3 flex items-center gap-2">
            <div className="h-1 min-w-0 flex-1 overflow-hidden rounded-full bg-slate-800/80" role="progressbar" aria-label={`${card.label} ratio`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(card.ratio * 100)}>
              <div className={`h-full min-w-[4px] rounded-full transition-[width] duration-500 ${card.bar}`} style={{ width: `${Math.min(100, Math.max(0, card.ratio * 100))}%` }} />
            </div>
            <span className="numeric-value w-10 text-right text-[11px] text-slate-500">{formatPercent(card.ratio)}</span>
          </div>
        </article>
      ))}
    </section>
  );
}

function formatPercent(value: number) {
  return new Intl.NumberFormat(undefined, { style: 'percent', maximumFractionDigits: 1 }).format(value);
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 ? 1 : 2)} ${units[unit]}`;
}
