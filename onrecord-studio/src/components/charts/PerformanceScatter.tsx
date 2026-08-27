import { useMemo, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import type { AgencyPerformanceProfile } from '../../types';
import { cn } from '../../utils/cn';
import Icon from '../ui/icons';
import AnalyticsUnavailable from './AnalyticsUnavailable';

const WIDTH = 700;
const HEIGHT = 320;
const MARGIN = { top: 24, right: 24, bottom: 46, left: 52 };

export default function PerformanceScatter() {
  // FEATURE(analytics): Add brush/zoom and an exportable vector snapshot for
  // investigators preparing a briefing or legal exhibit.
  const profiles = useAppStore((state) => state.agencyProfiles);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const [focused, setFocused] = useState<AgencyPerformanceProfile | null>(null);

  // PERF(analytics): The current sort and SVG node count are appropriate for
  // the bounded profile set; aggregate or virtualize if agency counts grow.
  const data = useMemo(
    () => profiles
      .filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey)
      // Unmeasured last: an agency with no closure sample is not the best performer, it is an
      // agency nothing can be said about yet.
      .sort((left, right) => (right.performanceZScore ?? -Infinity) - (left.performanceZScore ?? -Infinity)),
    [activePortalKey, profiles],
  );
  // Only a profile with both coordinates can be a point. Plotting an unmeasured agency at the
  // origin put it at the fastest position on the chart, tied with the most compliant department.
  const plotted = useMemo(
    () => data.filter(
      (profile): profile is AgencyPerformanceProfile & { averageClosureDays: number; performanceZScore: number } =>
        profile.averageClosureDays !== null && profile.performanceZScore !== null,
    ),
    [data],
  );
  const unmeasuredCount = data.length - plotted.length;
  const maxDays = Math.max(10, ...plotted.map((profile) => profile.averageClosureDays));
  const minZ = Math.min(-1, ...plotted.map((profile) => profile.performanceZScore));
  const maxZ = Math.max(3, ...plotted.map((profile) => profile.performanceZScore));
  const maxRequests = Math.max(1, ...data.map((profile) => profile.totalRequestsProcessed));
  const plotWidth = WIDTH - MARGIN.left - MARGIN.right;
  const plotHeight = HEIGHT - MARGIN.top - MARGIN.bottom;
  const x = (value: number) => MARGIN.left + (value / maxDays) * plotWidth;
  const y = (value: number) => MARGIN.top + (1 - (value - minZ) / (maxZ - minZ)) * plotHeight;

  const inspect = (profile: AgencyPerformanceProfile) => {
    setActivePortal(profile.portalSubdomain);
    setSearchFilters('', profile.agencyId, null, deviationFor(profile));
    setWorkspaceView('evidence');
  };

  return (
    <section className="app-panel flex min-h-[390px] flex-col overflow-hidden">
      <div className="flex items-start justify-between gap-4 border-b border-slate-800/70 px-5 py-4">
        <div>
          <div className="app-eyebrow">Department performance map</div>
          <h2 className="mt-1 text-[13px] font-semibold text-slate-200">Response time vs. statistical deviation</h2>
          <p className="mt-1 text-[10px] text-slate-600">Bubble size represents audited request volume. Select a department to inspect its evidence.</p>
        </div>
        <div className="hidden items-center gap-3 rounded-lg border border-slate-800 bg-slate-950/40 px-3 py-2 text-[8px] text-slate-500 sm:flex">
          <LegendDot color="bg-cyan-400" label="Typical" />
          <LegendDot color="bg-amber-400" label="Elevated" />
          <LegendDot color="bg-rose-400" label="Critical" />
        </div>
      </div>

      {data.length === 0 && analyticsError ? (
        <AnalyticsUnavailable className="m-3 flex-1" heading="Department map unavailable" />
      ) : data.length === 0 ? (
        <ChartEmptyState />
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-1 2xl:grid-cols-[minmax(0,1fr)_220px]">
          <div className="chart-surface data-grid min-h-[300px] border-b border-slate-800/70 p-3 2xl:border-b-0 2xl:border-r">
            {unmeasuredCount > 0 && (
              // Saying so beats silently dropping them: an agency with no recorded closure cannot
              // be placed on either axis, and it must not be implied to be missing for any other
              // reason.
              <p className="mb-2 text-[9px] text-slate-500">
                {unmeasuredCount} department{unmeasuredCount === 1 ? '' : 's'} not plotted: no closure has been recorded, so there is no closure time or deviation to place.
              </p>
            )}
            <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} className="h-full min-h-[280px] w-full" role="img" aria-label="Scatter plot of agency average closure time and performance z-score">
              <defs>
                <filter id="pointGlow" x="-100%" y="-100%" width="300%" height="300%">
                  <feGaussianBlur stdDeviation="3" result="blur" />
                  <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
                </filter>
              </defs>
              {[0, 0.25, 0.5, 0.75, 1].map((tick) => {
                const value = tick * maxDays;
                const xPosition = x(value);
                return (
                  <g key={`x-${tick}`}>
                    <line x1={xPosition} y1={MARGIN.top} x2={xPosition} y2={HEIGHT - MARGIN.bottom} stroke="var(--chart-grid)" strokeWidth="1" strokeDasharray="3 6" />
                    <text x={xPosition} y={HEIGHT - 23} fill="var(--chart-axis)" fontSize="11" textAnchor="middle">{value.toFixed(0)}d</text>
                  </g>
                );
              })}
              {[minZ, 0, 1, 2, maxZ].filter((value, index, values) => values.indexOf(value) === index).map((tick) => (
                <g key={`y-${tick}`}>
                  <line x1={MARGIN.left} y1={y(tick)} x2={WIDTH - MARGIN.right} y2={y(tick)} stroke={tick === 2 ? 'var(--chart-alert)' : tick === 0 ? 'var(--chart-axis)' : 'var(--chart-grid)'} strokeOpacity={tick === 2 ? 0.72 : 0.7} strokeWidth={tick === 0 || tick === 2 ? 1.2 : 1} strokeDasharray={tick === 0 ? undefined : '3 6'} />
                  <text x={MARGIN.left - 10} y={y(tick) + 4} fill={tick === 2 ? 'var(--chart-alert)' : 'var(--chart-axis)'} fontSize="11" textAnchor="end">{tick.toFixed(1)}</text>
                </g>
              ))}
              <text x={WIDTH / 2} y={HEIGHT - 5} fill="var(--chart-axis)" fontSize="11" textAnchor="middle">Average closure time</text>
              <text transform={`translate(13 ${HEIGHT / 2}) rotate(-90)`} fill="var(--chart-axis)" fontSize="11" textAnchor="middle">Performance z-score</text>
              <text x={WIDTH - MARGIN.right} y={y(2) - 8} fill="var(--chart-alert)" fontSize="11" textAnchor="end">critical threshold</text>

              {plotted.map((profile) => {
                const severity = severityFor(profile);
                const color = severity === 'critical' ? 'var(--viz-red)' : severity === 'elevated' ? 'var(--viz-amber)' : 'var(--viz-cyan)';
                const radius = 5 + Math.sqrt(profile.totalRequestsProcessed / maxRequests) * 12;
                const active = focused?.agencyId === profile.agencyId && focused.portalSubdomain === profile.portalSubdomain;
                return (
                  <g
                    key={`${profile.portalSubdomain}-${profile.agencyId}`}
                    role="button"
                    tabIndex={0}
                    aria-label={`${profile.agencyName}, ${profile.averageClosureDays.toFixed(1)} average days, z-score ${profile.performanceZScore.toFixed(2)}`}
                    className="cursor-pointer outline-none"
                    onMouseEnter={() => setFocused(profile)}
                    onMouseLeave={() => setFocused(null)}
                    onFocus={() => setFocused(profile)}
                    onBlur={() => setFocused(null)}
                    onClick={() => inspect(profile)}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter' || event.key === ' ') inspect(profile);
                    }}
                  >
                    <circle cx={x(profile.averageClosureDays)} cy={y(profile.performanceZScore)} r={radius + 5} fill={color} opacity={active ? 0.14 : 0.04} />
                    <circle cx={x(profile.averageClosureDays)} cy={y(profile.performanceZScore)} r={radius} fill={color} fillOpacity={active ? 0.9 : 0.62} stroke={color} strokeWidth={active ? 2 : 1} filter={active ? 'url(#pointGlow)' : undefined} />
                    {active && (
                      <text x={x(profile.averageClosureDays)} y={y(profile.performanceZScore) - radius - 8} fill="var(--text-primary)" fontSize="11" fontWeight="600" textAnchor="middle">
                        {shorten(profile.agencyName, 28)}
                      </text>
                    )}
                  </g>
                );
              })}
            </svg>
          </div>

          <aside className="min-h-0 p-4">
            <div className="flex items-center justify-between">
              <div className="app-eyebrow">Priority review</div>
              <span className="font-mono text-[8px] text-slate-600">Top {Math.min(5, data.length)}</span>
            </div>
            <div className="mt-3 space-y-1.5">
              {data.slice(0, 5).map((profile, index) => (
                <button
                  key={`${profile.portalSubdomain}-${profile.agencyId}`}
                  type="button"
                  onMouseEnter={() => setFocused(profile)}
                  onMouseLeave={() => setFocused(null)}
                  onClick={() => inspect(profile)}
                  className={cn(
                    'focus-ring group flex w-full items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition',
                    focused?.agencyId === profile.agencyId ? 'border-slate-700 bg-slate-800/55' : 'border-transparent hover:border-slate-800 hover:bg-slate-900/70',
                  )}
                >
                  <span className="font-mono text-[9px] text-slate-700">{String(index + 1).padStart(2, '0')}</span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[10px] font-medium text-slate-300">{profile.agencyName}</span>
                    <span className="mt-0.5 block text-[8px] text-slate-600">{profile.totalRequestsProcessed.toLocaleString()} requests · {profile.averageClosureDays === null ? 'no closures recorded' : `${profile.averageClosureDays.toFixed(1)}d avg.`}</span>
                  </span>
                  <span className={cn('font-mono text-[10px] font-semibold', severityFor(profile) === 'critical' ? 'text-rose-300' : severityFor(profile) === 'elevated' ? 'text-amber-300' : 'text-cyan-300')}>
                    {profile.performanceZScore === null ? '—' : `${profile.performanceZScore >= 0 ? '+' : ''}${profile.performanceZScore.toFixed(1)}`}
                  </span>
                </button>
              ))}
            </div>
            <button type="button" onClick={() => setWorkspaceView('evidence')} className="focus-ring mt-3 flex w-full items-center justify-between rounded-xl border border-slate-800 bg-slate-950/40 px-3 py-2.5 text-[9px] font-semibold text-slate-400 transition hover:border-cyan-400/20 hover:text-cyan-300">
              Open evidence workspace
              <Icon name="arrow" className="h-3.5 w-3.5" />
            </button>
          </aside>
        </div>
      )}
    </section>
  );
}

function ChartEmptyState() {
  const previewPoints = [
    { x: 118, y: 205, r: 11, color: 'var(--viz-cyan)' },
    { x: 215, y: 170, r: 15, color: 'var(--viz-cyan)' },
    { x: 308, y: 193, r: 9, color: 'var(--viz-indigo)' },
    { x: 402, y: 128, r: 18, color: 'var(--viz-amber)' },
    { x: 510, y: 154, r: 12, color: 'var(--viz-indigo)' },
    { x: 593, y: 76, r: 15, color: 'var(--viz-red)' },
  ];

  return (
    <div className="chart-surface data-grid relative min-h-[315px] flex-1 overflow-hidden">
      <svg viewBox="0 0 700 315" preserveAspectRatio="none" className="absolute inset-0 h-full w-full opacity-75" aria-hidden="true">
        <defs>
          <linearGradient id="previewFade" x1="0" x2="0" y1="0" y2="1">
            <stop offset="0" stopColor="var(--viz-red)" stopOpacity="0.08" />
            <stop offset="1" stopColor="var(--viz-red)" stopOpacity="0" />
          </linearGradient>
          <filter id="previewGlow" x="-100%" y="-100%" width="300%" height="300%">
            <feGaussianBlur stdDeviation="5" />
          </filter>
        </defs>
        <rect x="58" y="34" width="610" height="64" rx="8" fill="url(#previewFade)" />
        <line x1="58" y1="98" x2="668" y2="98" stroke="var(--chart-alert)" strokeOpacity="0.45" strokeDasharray="5 7" />
        <line x1="58" y1="258" x2="668" y2="258" stroke="var(--chart-axis)" strokeOpacity="0.5" />
        <line x1="58" y1="34" x2="58" y2="258" stroke="var(--chart-axis)" strokeOpacity="0.5" />
        {previewPoints.map((point, index) => (
          <g key={`${point.x}-${point.y}`} className="preview-point" style={{ animationDelay: `${index * 140}ms` }}>
            <circle cx={point.x} cy={point.y} r={point.r + 7} fill={point.color} opacity="0.08" filter="url(#previewGlow)" />
            <circle cx={point.x} cy={point.y} r={point.r} fill={point.color} opacity="0.62" stroke={point.color} strokeWidth="1.5" />
          </g>
        ))}
      </svg>
      <div className="absolute inset-0 grid place-items-center p-6 text-center">
        <div className="chart-ready-card max-w-sm rounded-2xl border px-5 py-4 backdrop-blur-xl">
          <div className="mx-auto flex w-fit items-center gap-2 rounded-full border border-cyan-400/20 bg-cyan-400/[0.07] px-2.5 py-1 text-[8px] font-semibold uppercase tracking-[0.12em] text-cyan-300">
            <Icon name="activity" className="h-3.5 w-3.5" /> Analytics canvas ready
          </div>
          <h3 className="mt-3 text-[13px] font-semibold text-slate-200">Patterns become visible as evidence arrives.</h3>
          <p className="mt-1.5 text-[10px] leading-relaxed text-slate-600">Response time, request volume, and statistical deviation will resolve into an interactive department map.</p>
        </div>
      </div>
    </div>
  );
}

function LegendDot({ color, label }: { color: string; label: string }) {
  return <span className="flex items-center gap-1.5"><span className={`h-1.5 w-1.5 rounded-full ${color}`} />{label}</span>;
}

function severityFor(profile: AgencyPerformanceProfile) {
  // A null z-score is an absent measurement, so it raises nothing on its own. The statutory
  // signals beside it are counts and stand by themselves.
  const z = profile.performanceZScore;
  if ((z !== null && z >= 2) || profile.statutoryViolationRate >= 0.25) return 'critical';
  if ((z !== null && z >= 1) || profile.statutoryViolationCount > 0) return 'elevated';
  return 'typical';
}

function deviationFor(profile: AgencyPerformanceProfile) {
  if (profile.statutoryViolationCount > 0) return 'STATUTORY_LATENCY_VIOLATION';
  if (profile.suspiciousFastClosureCount > 0) return 'SUSPICIOUSLY_FAST_CLOSURE';
  return null;
}

function shorten(value: string, maximum: number) {
  return value.length <= maximum ? value : `${value.slice(0, maximum - 1)}…`;
}
