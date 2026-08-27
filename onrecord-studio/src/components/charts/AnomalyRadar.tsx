import { useMemo, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import type { AgencyPerformanceProfile } from '../../types';
import AnalyticsUnavailable from './AnalyticsUnavailable';

interface ArcDatum {
  key: string;
  label: string;
  portalKey: string;
  agencyId: number | null;
  startAngle: number;
  endAngle: number;
  innerRadius: number;
  outerRadius: number;
  color: string;
  profile: AgencyPerformanceProfile | null;
}

const palette = ['var(--viz-indigo)', 'var(--viz-cyan)', 'var(--viz-purple)', 'var(--viz-amber)', 'var(--viz-emerald)'];

function point(radius: number, angle: number) {
  return {
    x: 120 + radius * Math.cos(angle - Math.PI / 2),
    y: 120 + radius * Math.sin(angle - Math.PI / 2),
  };
}

function arcPath(startAngle: number, endAngle: number, innerRadius: number, outerRadius: number) {
  const outerStart = point(outerRadius, startAngle);
  const outerEnd = point(outerRadius, endAngle);
  const innerEnd = point(innerRadius, endAngle);
  const innerStart = point(innerRadius, startAngle);
  const largeArc = endAngle - startAngle > Math.PI ? 1 : 0;
  return [
    `M ${outerStart.x} ${outerStart.y}`,
    `A ${outerRadius} ${outerRadius} 0 ${largeArc} 1 ${outerEnd.x} ${outerEnd.y}`,
    `L ${innerEnd.x} ${innerEnd.y}`,
    `A ${innerRadius} ${innerRadius} 0 ${largeArc} 0 ${innerStart.x} ${innerStart.y}`,
    'Z',
  ].join(' ');
}

function deviationFor(profile: AgencyPerformanceProfile) {
  if (profile.statutoryViolationCount > 0) return 'STATUTORY_LATENCY_VIOLATION';
  if (profile.suspiciousFastClosureCount > 0) return 'SUSPICIOUSLY_FAST_CLOSURE';
  return null;
}

export default function AnomalyRadar() {
  // TODO(analytics): Make the portal and agency ring weights configurable so
  // the visualization can distinguish volume from anomaly severity explicitly.
  const profiles = useAppStore((state) => state.agencyProfiles);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const [focused, setFocused] = useState<ArcDatum | null>(null);

  const arcs = useMemo(() => {
    const grouped = new Map<string, AgencyPerformanceProfile[]>();
    for (const profile of profiles) {
      const values = grouped.get(profile.portalSubdomain) ?? [];
      values.push(profile);
      grouped.set(profile.portalSubdomain, values);
    }
    const portalEntries = [...grouped.entries()].sort(([left], [right]) => left.localeCompare(right));
    const globalTotal = Math.max(
      1,
      profiles.reduce((total, profile) => total + Math.max(1, profile.totalRequestsProcessed), 0),
    );
    const result: ArcDatum[] = [];
    let cursor = 0;
    portalEntries.forEach(([portalKey, agencies], portalIndex) => {
      const portalTotal = agencies.reduce(
        (total, profile) => total + Math.max(1, profile.totalRequestsProcessed),
        0,
      );
      const portalSweep = (portalTotal / globalTotal) * Math.PI * 2;
      const portalStart = cursor;
      const portalEnd = cursor + portalSweep;
      result.push({
        key: `portal-${portalKey}`,
        label: portalKey,
        portalKey,
        agencyId: null,
        startAngle: portalStart + 0.008,
        endAngle: portalEnd - 0.008,
        innerRadius: 35,
        outerRadius: 65,
        color: palette[portalIndex % palette.length],
        profile: null,
      });
      let agencyCursor = portalStart;
      agencies
        .sort((left, right) => left.agencyName.localeCompare(right.agencyName))
        .forEach((profile, agencyIndex) => {
          const sweep = (Math.max(1, profile.totalRequestsProcessed) / globalTotal) * Math.PI * 2;
          const severity = Math.min(
            1,
            profile.statutoryViolationRate
              + profile.suspiciousFastClosureCount / Math.max(1, profile.totalRequestsProcessed),
          );
          const base = palette[portalIndex % palette.length];
          const alert = agencyIndex % 2 ? 'var(--viz-amber)' : 'var(--viz-red)';
          result.push({
            key: `${portalKey}-${profile.agencyId}`,
            label: profile.agencyName,
            portalKey,
            agencyId: profile.agencyId,
            startAngle: agencyCursor + 0.01,
            endAngle: agencyCursor + sweep - 0.01,
            innerRadius: 69,
            outerRadius: 104,
            color: severity > 0.2 ? alert : base,
            profile,
          });
          agencyCursor += sweep;
        });
      cursor = portalEnd;
    });
    return result.filter((arc) => arc.endAngle > arc.startAngle);
  }, [profiles]);

  const activate = (arc: ArcDatum) => {
    setFocused(arc);
    setActivePortal(arc.portalKey);
    setSearchFilters('', arc.agencyId, null, arc.profile ? deviationFor(arc.profile) : null);
    setWorkspaceView('evidence');
  };

  if (arcs.length === 0) {
    return (
      <section className="app-panel flex min-h-[360px] flex-col overflow-hidden p-5">
        <div>
          <div className="app-eyebrow">Department deviation sunburst</div>
          <div className="mt-1 text-[10px] text-slate-500">Portal ring → department ring. Select a slice to inspect evidence.</div>
        </div>
        {analyticsError ? (
          <AnalyticsUnavailable className="mt-4 flex-1" heading="Deviation map unavailable" />
        ) : (
        <div className="relative grid flex-1 place-items-center overflow-hidden">
          <svg viewBox="0 0 240 240" className="h-[250px] w-[250px] opacity-70" aria-hidden="true">
            <g transform="rotate(-32 120 120)">
              <circle cx="120" cy="120" r="92" fill="none" stroke="var(--viz-cyan)" strokeOpacity="0.34" strokeWidth="24" strokeDasharray="105 24 52 38 78 52" />
              <circle cx="120" cy="120" r="58" fill="none" stroke="var(--viz-indigo)" strokeOpacity="0.28" strokeWidth="22" strokeDasharray="72 30 48 26 86 46" />
            </g>
            <circle cx="120" cy="120" r="34" fill="var(--chart-center)" stroke="var(--chart-grid)" />
            <circle cx="120" cy="120" r="4" fill="var(--viz-cyan)" opacity="0.8" />
          </svg>
          <div className="chart-ready-card absolute max-w-[230px] rounded-xl border px-4 py-3 text-center backdrop-blur-xl">
            <div className="text-[10px] font-semibold text-slate-300">Relationship map ready</div>
            <div className="mt-1 text-[9px] leading-relaxed text-slate-600">Agency density will resolve into selectable source and department rings.</div>
          </div>
        </div>
        )}
      </section>
    );
  }

  return (
    <section className="app-panel flex min-h-[360px] flex-col overflow-hidden p-5">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="app-eyebrow">Department deviation sunburst</div>
          <div className="mt-1 text-[10px] text-slate-500">Portal ring → department ring. Select a slice to inspect evidence.</div>
        </div>
        <div className="rounded border border-slate-800 bg-slate-900/60 px-2 py-1 font-mono text-[9px] text-slate-500">{profiles.length} departments</div>
      </div>
      <div className="grid flex-1 grid-cols-1 items-center gap-2 sm:grid-cols-[minmax(0,1fr)_160px]">
        <svg viewBox="0 0 240 240" className="mx-auto h-full max-h-[280px] w-full max-w-[320px]" aria-label="Multi-portal department deviation sunburst">
          {arcs.map((arc) => (
            <path
              key={arc.key}
              d={arcPath(arc.startAngle, arc.endAngle, arc.innerRadius, arc.outerRadius)}
              fill={arc.color}
              fillOpacity={focused?.key === arc.key ? 1 : arc.profile ? 0.88 : 0.72}
              stroke="var(--chart-background)"
              strokeWidth="1.5"
              className="cursor-pointer outline-none transition-opacity hover:opacity-100 focus:opacity-100"
              role="button"
              tabIndex={0}
              aria-label={`Filter ${arc.label}`}
              onMouseEnter={() => setFocused(arc)}
              onFocus={() => setFocused(arc)}
              onClick={() => activate(arc)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') activate(arc);
              }}
            />
          ))}
          <circle cx="120" cy="120" r="29" fill="var(--chart-center)" stroke="var(--chart-grid)" />
          <text x="120" y="116" textAnchor="middle" fill="var(--text-muted)" fontSize="11" fontWeight="700">AUDIT</text>
          <text x="120" y="130" textAnchor="middle" fill="var(--chart-axis)" fontSize="11">DENSITY</text>
        </svg>
        <div className="space-y-2 rounded-lg border border-slate-800 bg-slate-950/70 p-3 font-mono text-[9px]">
          <div className="truncate text-[10px] font-bold text-slate-300">{focused?.label ?? 'Hover a slice'}</div>
          {focused?.profile ? (
            <>
              <Metric label="Requests" value={focused.profile.totalRequestsProcessed} />
              <Metric label="Z-score" value={focused.profile.performanceZScore === null ? 'Not computed' : focused.profile.performanceZScore.toFixed(2)} alert={focused.profile.performanceZScore !== null && focused.profile.performanceZScore >= 2} />
              <Metric label="Statutory" value={focused.profile.statutoryViolationCount} alert={focused.profile.statutoryViolationCount > 0} />
              <Metric label="Fast close" value={focused.profile.suspiciousFastClosureCount} alert={focused.profile.suspiciousFastClosureCount > 0} />
            </>
          ) : (
            <div className="leading-relaxed text-slate-500">Inner slices represent isolated portal partitions.</div>
          )}
        </div>
      </div>
    </section>
  );
}

function Metric({ label, value, alert = false }: { label: string; value: string | number; alert?: boolean }) {
  return (
    <div className="flex justify-between gap-2 border-b border-slate-900 pb-1">
      <span className="text-slate-600">{label}</span>
      <span className={alert ? 'text-amber-400' : 'text-slate-400'}>{value}</span>
    </div>
  );
}
