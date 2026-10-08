import { useMemo, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import type { AgencyPerformanceProfile } from '../../types';
import { ShapeGlyph } from '../ui/shapeGlyph';
import AnalyticsUnavailable from './AnalyticsUnavailable';
import { deviationTier, TIER_LABEL, type DeviationTier } from './deviationTier';

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
  /** Null when the department has no measured deviation. */
  severity: DeviationTier | null;
  profile: AgencyPerformanceProfile | null;
}

// Arcs carry PerformanceScatter's tiers and glyphs (triangle elevated, diamond outlier), from the
// same z-score predicate the headline counts. Typical arcs get no glyph, since the wedge shape
// itself already reads as the unmarked case.

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
  const profiles = useAppStore((state) => state.agencyProfiles);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const [focused, setFocused] = useState<ArcDatum | null>(null);
  const [weightMode, setWeightMode] = useState<'volume' | 'severity'>('volume');

  const arcs = useMemo(() => {
    const grouped = new Map<string, AgencyPerformanceProfile[]>();
    for (const profile of profiles) {
      const values = grouped.get(profile.portalSubdomain) ?? [];
      values.push(profile);
      grouped.set(profile.portalSubdomain, values);
    }
    const portalEntries = [...grouped.entries()].sort(([left], [right]) => left.localeCompare(right));
    const weight = (profile: AgencyPerformanceProfile) => weightMode === 'volume'
      ? Math.max(1, profile.totalRequestsProcessed)
      : Math.max(0.01, profile.statutoryViolationRate + profile.suspiciousFastClosureCount / Math.max(1, profile.totalRequestsProcessed));
    const globalTotal = Math.max(
      Number.EPSILON,
      profiles.reduce((total, profile) => total + weight(profile), 0),
    );
    const result: ArcDatum[] = [];
    let cursor = 0;
    portalEntries.forEach(([portalKey, agencies], portalIndex) => {
      const portalTotal = agencies.reduce(
        (total, profile) => total + weight(profile),
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
        severity: null,
        profile: null,
      });
      let agencyCursor = portalStart;
      agencies
        .sort((left, right) => left.agencyName.localeCompare(right.agencyName))
        .forEach((profile) => {
          const sweep = (weight(profile) / globalTotal) * Math.PI * 2;
          const base = palette[portalIndex % palette.length];
          const band = profile.performanceZScore === null ? null : deviationTier(profile.performanceZScore);
          result.push({
            key: `${portalKey}-${profile.agencyId}`,
            label: profile.agencyName,
            portalKey,
            agencyId: profile.agencyId,
            startAngle: agencyCursor + 0.01,
            endAngle: agencyCursor + sweep - 0.01,
            innerRadius: 69,
            outerRadius: 104,
            // Alarm colour follows the severity, not the agency's position in the list: parity
            // gave two agencies with identical deviation different colours, which reads as a
            // difference in the evidence.
            color: band === 'outlier' ? 'var(--viz-red)' : band === 'elevated' ? 'var(--viz-amber)' : base,
            severity: band,
            profile,
          });
          agencyCursor += sweep;
        });
      cursor = portalEnd;
    });
    return result.filter((arc) => arc.endAngle > arc.startAngle);
  }, [profiles, weightMode]);

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
          <div className="mt-1 text-label text-muted">Portal ring → department ring. Select a slice to inspect evidence.</div>
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
          <div className="chart-ready-card absolute max-w-[230px] rounded-xl border px-4 py-3 text-center">
            <div className="text-label font-semibold text-fg-secondary">No departments profiled yet</div>
            <div className="mt-1 text-caption leading-relaxed text-muted">This panel draws one ring per source and one slice per department, from the agency performance profiles. None have been computed, so there is nothing here to read.</div>
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
          <div className="text-overline uppercase text-muted">Department deviation</div>
          <h4 className="mt-1 text-headline text-fg">{radarHeadline(profiles)}</h4>
          <div className="mt-1 text-caption text-muted">Portal ring → department ring. Select a slice to inspect evidence.</div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <label className="field flex h-7 items-center gap-1 rounded-md border border-control bg-canvas px-2 text-caption text-muted">Slice by <select value={weightMode} onChange={(event) => setWeightMode(event.target.value as 'volume' | 'severity')} className="bg-transparent text-fg-secondary outline-none"><option value="volume">request volume</option><option value="severity">flagged-request share</option></select></label>
          <span className="whitespace-nowrap text-caption tabular-nums text-muted">{profiles.length} departments</span>
        </div>
      </div>
      <div className="grid flex-1 grid-cols-1 items-center gap-2 sm:grid-cols-[minmax(0,1fr)_160px]">
        <svg viewBox="0 0 240 240" className="mx-auto h-full max-h-[280px] w-full max-w-[320px]" aria-label="Multi-portal department deviation sunburst">
          {arcs.map((arc) => {
            const mid = point((arc.innerRadius + arc.outerRadius) / 2, (arc.startAngle + arc.endAngle) / 2);
            return (
              <g key={arc.key}>
                <path
                  d={arcPath(arc.startAngle, arc.endAngle, arc.innerRadius, arc.outerRadius)}
                  fill={arc.color}
                  fillOpacity={focused?.key === arc.key ? 1 : arc.profile ? 0.88 : 0.72}
                  stroke="var(--chart-background)"
                  strokeWidth="1.5"
                  className="outline-none transition-opacity hover:opacity-100 focus:opacity-100 focus-visible:stroke-[rgb(var(--focus))] focus-visible:[stroke-width:2.5]"
                  role="button"
                  tabIndex={0}
                  aria-label={`Filter ${arc.label}${arc.severity === null || arc.severity === 'typical' ? '' : `, ${TIER_LABEL[arc.severity].toLowerCase()}`}`}
                  onMouseEnter={() => setFocused(arc)}
                  onFocus={() => setFocused(arc)}
                  onClick={() => activate(arc)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' || event.key === ' ') activate(arc);
                  }}
                />
                {(arc.severity === 'elevated' || arc.severity === 'outlier') && (
                  <ShapeGlyph
                    shape={arc.severity === 'outlier' ? 'diamond' : 'triangle'}
                    cx={mid.x}
                    cy={mid.y}
                    radius={5}
                    fill="var(--chart-background)"
                    pointerEvents="none"
                    aria-hidden="true"
                  />
                )}
              </g>
            );
          })}
          <circle cx="120" cy="120" r="29" fill="var(--chart-center)" stroke="var(--chart-grid)" />
          <text x="120" y="116" textAnchor="middle" fill="var(--text-muted)" fontSize="11" fontWeight="700">AUDIT</text>
          <text x="120" y="130" textAnchor="middle" fill="var(--chart-axis)" fontSize="11">{weightMode === 'volume' ? 'VOLUME' : 'FLAGGED'}</text>
        </svg>
        <div className="space-y-2 rounded-lg border border-line bg-canvas p-3 text-caption tabular-nums">
          <div className="truncate text-label font-bold text-fg-secondary">{focused?.label ?? 'Hover a slice'}</div>
          {focused?.profile ? (
            <>
              <Metric label="Requests" value={focused.profile.totalRequestsProcessed} />
              <Metric label="Z-score" value={focused.profile.performanceZScore === null ? 'Not measured' : focused.profile.performanceZScore.toFixed(2)} alert={focused.profile.performanceZScore !== null && deviationTier(focused.profile.performanceZScore) === 'outlier'} />
              <Metric label="Statutory" value={focused.profile.statutoryViolationCount} alert={focused.profile.statutoryViolationCount > 0} />
              <Metric label="Fast close" value={focused.profile.suspiciousFastClosureCount} alert={focused.profile.suspiciousFastClosureCount > 0} />
            </>
          ) : (
            <div className="leading-relaxed text-muted">Inner slices represent isolated portal partitions.</div>
          )}
        </div>
      </div>
    </section>
  );
}

function Metric({ label, value, alert = false }: { label: string; value: string | number; alert?: boolean }) {
  return (
    <div className="flex justify-between gap-2 border-b border-line pb-1">
      <span className="text-muted">{label}</span>
      <span className={alert ? 'flex items-center gap-1 text-warning' : 'text-muted'}>
        {alert && <FlagMark label="Flagged" />}
        {value}
      </span>
    </div>
  );
}

// A triangle beside a flagged value, so the flag survives when amber and slate read alike.
function FlagMark({ label }: { label: string }) {
  return (
    <svg viewBox="-6 -6 12 12" className="h-2 w-2 shrink-0" role="img" aria-label={label}>
      <polygon points="0,-5 4.6,3.9 -4.6,3.9" fill="currentColor" />
    </svg>
  );
}

/** The radar's finding in one sentence; deviation wording, never a legal one. */
function radarHeadline(profiles: AgencyPerformanceProfile[]): string {
  const measured = profiles.filter((profile) => profile.performanceZScore !== null);
  if (measured.length === 0) return 'No department has a measured deviation yet';
  const deviating = measured.filter((profile) => profile.performanceZScore !== null && deviationTier(profile.performanceZScore) === 'outlier').length;
  return `${deviating} ${deviating === 1 ? 'department deviates' : 'departments deviate'} ≥2σ from their portal's norm`;
}
