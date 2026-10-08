import { useState, type ReactNode } from 'react';
import type { AgencyPerformanceProfile } from '../../../types/analysis';
import { withViewTransition } from '../../../utils/viewTransition';
import Button from '../../ui/Button';
import { VIZ } from '../core/claim';
import { formatValue } from '../core/format';
import { measureText } from '../core/layout';
import { measured } from '../core/nulls';
import { scaleLinear, type ContinuousScale } from '../core/scale';
import { Axis } from '../frame/Axis';
import ChartFrame, { type ChartStatus } from '../frame/ChartFrame';
import { LegendSwatch, type LegendItem } from '../frame/Legend';
import { ReferenceBand, ReferenceLine } from '../frame/ReferenceLine';
import { TooltipContent } from '../frame/Tooltip';
import type { TwinTable } from '../frame/TableTwin';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Bar } from '../marks/Bar';
import { Dot } from '../marks/Dot';
import { MarkGroup, MUTED_LABEL_CLASS } from '../marks/shared';

/**
 * Mirrors `STATUTORY_DEADLINE_Z_SCORE` in `src-tauri/src/analysis/anomaly.rs`, which is not
 * exposed over IPC. Only a positive score qualifies: a score this far below the mean is faster than
 * peers, and calling it slower would be a false statement.
 */
export const PEER_LATENCY_OUTLIER_Z_SCORE = 2.0;

/** Below this many measured closures a single closure moves the average by a fifth or more. */
export const SMALL_SAMPLE_CLOSURES = 5;

export const NO_MEASURABLE_CLOSURE = 'no measurable closure';
export const SLOWER_THAN_PEERS = 'slower than peers (statistical)';

/** The overview shows this many departments before 'Show all'. */
export const OVERVIEW_ROW_LIMIT = 8;

/** The one predicate behind the ring, the row text and the headline's count. */
export function isSlowerThanPeers(profile: AgencyPerformanceProfile): boolean {
  return profile.performanceZScore !== null && profile.performanceZScore >= PEER_LATENCY_OUTLIER_Z_SCORE;
}

export function isSmallSample(profile: AgencyPerformanceProfile): boolean {
  return profile.averageClosureDays !== null && profile.validLatencySampleSize < SMALL_SAMPLE_CLOSURES;
}

/**
 * The overview order: counted misses, then requests. Never by rate, which would put a department
 * with one request and one miss above one with 22 of 268.
 */
export function ledgerOrder(profiles: readonly AgencyPerformanceProfile[]): AgencyPerformanceProfile[] {
  return [...profiles].sort((a, b) => b.statutoryViolationCount - a.statutoryViolationCount
    || b.totalRequestsProcessed - a.totalRequestsProcessed
    || a.agencyName.localeCompare(b.agencyName));
}

const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString('en-US')} ${n === 1 ? one : many}`;
const percent = (v: number) => formatValue(v, { kind: 'percent' });
const days = (v: number) => formatValue(v, { kind: 'days' });

/** The counted sentence. It never mentions z or rapid closures. */
export function countedHeadline(rows: readonly AgencyPerformanceProfile[]): string {
  const requests = rows.reduce((sum, row) => sum + row.totalRequestsProcessed, 0);
  const missed = rows.filter((row) => row.statutoryViolationCount > 0);
  if (missed.length === 0) return `No department has a counted statutory deadline miss across ${plural(requests, 'request')}.`;
  const total = missed.reduce((sum, row) => sum + row.statutoryViolationCount, 0);
  const top = ledgerOrder(missed)[0];
  const lead = missed.length === 1
    ? `${top.agencyName} missed the statutory deadline on ${total.toLocaleString('en-US')} of ${plural(requests, 'request')}`
    : `${missed.length} departments missed the statutory deadline on ${total.toLocaleString('en-US')} of ${plural(requests, 'request')}; ${top.agencyName} accounts for ${top.statutoryViolationCount.toLocaleString('en-US')}`;
  return `${lead}.`;
}

/** The compare page's second sentence: the statistical comparison, named as one. */
export function peerHeadline(rows: readonly AgencyPerformanceProfile[]): string {
  const measuredRows = rows.filter((row) => row.averageClosureDays !== null);
  if (measuredRows.length === 0) return 'No agency has a measurable closure yet.';
  const slowest = measuredRows.reduce((best, row) => ((row.averageClosureDays ?? 0) > (best.averageClosureDays ?? 0) ? row : best));
  const slower = rows.filter(isSlowerThanPeers).length;
  return `${slowest.agencyName} averages ${formatValue(slowest.averageClosureDays ?? 0, { kind: 'days', digits: 1 }).replace(' d', ' days')} over ${plural(slowest.validLatencySampleSize, 'closure')}; ${slower} of ${plural(rows.length, 'agency', 'agencies')} ${slower === 1 ? 'is' : 'are'} slower than peers (z ≥ ${PEER_LATENCY_OUTLIER_Z_SCORE}), a statistical comparison, not a statutory finding.`;
}

export interface DepartmentLedgerProps {
  variant: 'overview' | 'compare';
  /** The departments in the order to draw them. Departments with no requests are left out. */
  profiles: readonly AgencyPerformanceProfile[];
  /**
   * Every profile the scales are built from. A filter that narrows `profiles` changes membership
   * only, so a department keeps its position on every axis.
   */
  scaleProfiles?: readonly AgencyPerformanceProfile[];
  status: Exclude<ChartStatus, 'empty'>;
  error?: string | null;
  scopedOutText?: string;
  portalName(portalKey: string): string;
  onInspect(profile: AgencyPerformanceProfile): void;
  controls?: ReactNode;
}

const NAME_FONT = '12px -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif';
const NAME_CLASS = 'fill-fg text-[12px] font-medium';
const NOTE_CLASS = 'fill-muted text-[11px]';
const HEADER_CLASS = 'fill-fg-secondary text-[11px] font-medium';
const BAND = 14;
const HEADER = 34;
const MEAN_STRIP = 16;
const FOOT = 40;
const GUTTER = 20;

function truncate(text: string, width: number, font?: string): string {
  if (width <= 0) return '';
  if (measureText(text, font) <= width) return text;
  let end = text.length;
  while (end > 1 && measureText(`${text.slice(0, end)}…`, font) > width) end -= 1;
  return `${text.slice(0, end).trimEnd()}…`;
}

interface Layout {
  narrow: boolean;
  nameWidth: number;
  rowStep: number;
  cols: { a: [number, number]; b: [number, number]; c: [number, number] };
  height(rows: number): number;
}

/**
 * Three columns, each on its own x-scale, beside a name column; below 640px the names move onto
 * their own line above the bars so every column keeps a usable width down to 340px.
 */
function ledgerLayout(width: number): Layout {
  const narrow = width < 640;
  const nameWidth = narrow ? 0 : Math.min(260, Math.max(216, Math.round(width * 0.24)));
  const start = narrow ? 0 : nameWidth + 12;
  const rest = width - start - 2 * GUTTER;
  const a = Math.round(rest * 0.36);
  const b = Math.round(rest * 0.2);
  const cols = {
    a: [start, start + a] as [number, number],
    b: [start + a + GUTTER, start + a + GUTTER + b] as [number, number],
    c: [start + a + b + 2 * GUTTER, width] as [number, number],
  };
  const rowStep = narrow ? 54 : 38;
  return { narrow, nameWidth, rowStep, cols, height: (rows) => HEADER + MEAN_STRIP + rows * rowStep + FOOT };
}

function rowNotes(profile: AgencyPerformanceProfile, portal: string | null): string[] {
  const notes: string[] = [];
  if (portal) notes.push(portal);
  if (isSlowerThanPeers(profile)) notes.push(SLOWER_THAN_PEERS);
  if (isSmallSample(profile)) notes.push('small sample');
  return notes;
}

function countedLabel(profile: AgencyPerformanceProfile, withRate: boolean): string {
  const base = `${profile.statutoryViolationCount.toLocaleString('en-US')} of ${profile.totalRequestsProcessed.toLocaleString('en-US')}`;
  return withRate ? `${base} · ${percent(profile.statutoryViolationRate)}` : base;
}

function accessibleName(profile: AgencyPerformanceProfile): string {
  const closure = profile.averageClosureDays === null
    ? NO_MEASURABLE_CLOSURE
    : `average closure ${days(profile.averageClosureDays)} over ${plural(profile.validLatencySampleSize, 'closure')}${isSlowerThanPeers(profile) ? `, ${SLOWER_THAN_PEERS}` : ''}${isSmallSample(profile) ? ', small sample' : ''}`;
  return `${profile.agencyName}: ${profile.statutoryViolationCount.toLocaleString('en-US')} counted deadline ${profile.statutoryViolationCount === 1 ? 'miss' : 'misses'} of ${plural(profile.totalRequestsProcessed, 'request')}; ${plural(profile.suspiciousFastClosureCount, 'rapid closure')} (statistical); ${closure}.`;
}

function twinTable(rows: readonly AgencyPerformanceProfile[], portalName: (key: string) => string): TwinTable {
  return {
    caption: 'Department deadline ledger',
    columns: [
      { key: 'department', label: 'Department' },
      { key: 'portal', label: 'Portal' },
      { key: 'missed', label: 'Counted deadline misses', kind: 'count' },
      { key: 'requests', label: 'Requests', kind: 'count' },
      { key: 'rate', label: 'Share of requests', align: 'end' },
      { key: 'rapid', label: 'Rapid closures (statistical)', kind: 'count' },
      { key: 'average', label: 'Average closure', align: 'end' },
      { key: 'closures', label: 'Measured closures', kind: 'count' },
      { key: 'z', label: 'z vs peers (statistical)', align: 'end' },
    ],
    rows: rows.map((row) => ({
      department: row.agencyName,
      portal: portalName(row.portalSubdomain),
      missed: row.statutoryViolationCount,
      requests: row.totalRequestsProcessed,
      rate: percent(row.statutoryViolationRate),
      rapid: row.suspiciousFastClosureCount,
      average: row.averageClosureDays === null ? `Not measured: ${NO_MEASURABLE_CLOSURE}` : days(row.averageClosureDays),
      closures: row.validLatencySampleSize,
      z: row.performanceZScore === null ? 'Not measured: no closure sample or no spread among peers' : formatValue(row.performanceZScore, { kind: 'z', digits: 2 }),
    })),
  };
}

const LEGEND: LegendItem[] = [
  { key: 'counted', label: 'Counted deadline miss', swatch: 'flag', tone: 'counted' },
  { key: 'requests', label: 'Requests processed', swatch: 'rect', tone: 'track' },
  { key: 'average', label: 'Average closure', swatch: 'dot', tone: 'ink' },
  { key: 'small', label: `Fewer than ${SMALL_SAMPLE_CLOSURES} closures (small sample)`, swatch: 'hollow', tone: 'ink' },
  { key: 'slower', label: 'Slower than peers (statistical)', swatch: 'ring', tone: 'ink' },
];

/**
 * F1, the department deadline ledger: one row per department with requests, three columns on
 * their own x-scales. (a) counted deadline misses, the only legal claim, solid crimson on a track
 * of requests; (b) rapid closures, a statistical outline; (c) average closure time against the
 * peer mean and the ±2 sd typical range, with an ink ring at z ≥ 2. A department with no
 * measurable closure gets no dot, only the words.
 */
export default function DepartmentLedger({ variant, profiles, scaleProfiles, status, error, scopedOutText, portalName, onInspect, controls }: DepartmentLedgerProps) {
  const [showAll, setShowAll] = useState(false);
  const all = profiles.filter((profile) => profile.totalRequestsProcessed > 0);
  const shown = variant === 'overview' && !showAll ? all.slice(0, OVERVIEW_ROW_LIMIT) : all;
  const spansPortals = new Set(all.map((profile) => profile.portalSubdomain)).size > 1;
  const empty = all.length === 0 && (status === 'ready' || status === 'stale');
  const headline = status === 'ready' || status === 'stale'
    ? (all.length === 0 ? null : variant === 'compare' ? `${countedHeadline(all)} ${peerHeadline(all)}` : countedHeadline(all))
    : null;
  const unmeasured = all.filter((profile) => profile.averageClosureDays === null).length;
  const small = all.filter(isSmallSample).length;
  const summary = `${plural(all.length, 'department')} with requests${shown.length < all.length ? `, the first ${shown.length} shown` : ''}. `
    + `${all.filter(isSlowerThanPeers).length} slower than peers at z ≥ ${PEER_LATENCY_OUTLIER_Z_SCORE} (statistical). `
    + `${unmeasured} with ${NO_MEASURABLE_CLOSURE}; ${small} resting on fewer than ${SMALL_SAMPLE_CLOSURES} closures.`;

  return (
    <div className="grid gap-2">
      <ChartFrame
        title="Department deadline ledger"
        storageKey={`department-ledger-${variant}`}
        headline={headline}
        claim="mixed"
        sources={{
          tables: ['requests', 'statutory_extensions', 'agencies'],
          note: 'misses counted under Cal. Gov. Code § 7922.535; rapid closures and closure time are statistical comparisons, not legal findings',
        }}
        summary={summary}
        status={empty ? 'empty' : status}
        emptyText="No department has a request in this scope."
        error={error}
        scopedOutText={scopedOutText}
        table={twinTable(shown, portalName)}
        legend={LEGEND}
        controls={controls}
        minHeight={ledgerLayout(640).height(Math.max(1, Math.min(shown.length, 3)))}
        plotHeight={(width) => ledgerLayout(width).height(shown.length)}
        interactive={{ roledescription: 'department ledger' }}
      >
        {({ width }) => (
          <LedgerPlot
            width={width}
            rows={shown}
            scaleRows={scaleProfiles && scaleProfiles.length > 0 ? scaleProfiles : all}
            portalName={spansPortals ? portalName : null}
            onInspect={onInspect}
          />
        )}
      </ChartFrame>
      {variant === 'overview' && all.length > OVERVIEW_ROW_LIMIT && (status === 'ready' || status === 'stale') && (
        <div>
          <Button size="sm" variant="ghost" aria-expanded={showAll} onClick={() => withViewTransition(() => setShowAll((value) => !value))}>
            {showAll ? `Show the first ${OVERVIEW_ROW_LIMIT}` : `Show all ${all.length} departments`}
          </Button>
        </div>
      )}
    </div>
  );
}

function ColumnHeader({ x, title, note, width, flag = false }: { x: number; title: string; note: string; width: number; flag?: boolean }) {
  const titleX = flag ? x + 18 : x;
  return (
    <g aria-hidden="true">
      {flag && (
        <g transform={`translate(${x} ${2})`}>
          <LegendSwatch kind="flag" tone="counted" />
        </g>
      )}
      <text x={titleX} y={11} className={HEADER_CLASS}>
        <title>{`${title}: ${note}`}</title>
        {truncate(title, width - (titleX - x))}
      </text>
      <text x={x} y={25} className={NOTE_CLASS}>{truncate(note, width)}</text>
    </g>
  );
}

function LedgerPlot({ width, rows, scaleRows, portalName, onInspect }: {
  width: number;
  rows: readonly AgencyPerformanceProfile[];
  scaleRows: readonly AgencyPerformanceProfile[];
  portalName: ((key: string) => string) | null;
  onInspect(profile: AgencyPerformanceProfile): void;
}) {
  const tooltip = useTooltip();
  const roving = useRovingFocus({ shape: { rows: rows.length }, onActivate: (i) => onInspect(rows[i]), onEscape: tooltip.hide });
  const layout = ledgerLayout(width);
  const { cols, rowStep, narrow } = layout;
  const top = HEADER + MEAN_STRIP;
  const bottom = top + rows.length * rowStep;

  const withRate = cols.a[1] - cols.a[0] >= 220;
  const longestLabel = Math.max(...scaleRows.map((row) => measureText(countedLabel(row, withRate))), 0);
  const a: ContinuousScale = scaleLinear([0, Math.max(1, ...scaleRows.map((row) => row.totalRequestsProcessed))], [cols.a[0], Math.max(cols.a[0] + 24, cols.a[1] - longestLabel - 8)], { nice: false });
  const rapidMax = Math.max(1, ...scaleRows.map((row) => row.suspiciousFastClosureCount));
  const b: ContinuousScale = scaleLinear([0, rapidMax], [cols.b[0], Math.max(cols.b[0] + 12, cols.b[1] - measureText(rapidMax.toLocaleString('en-US')) - 8)], { nice: false });

  // The peer mean and spread are workspace-wide (one global population), read from the profiles.
  const reference = scaleRows.find((row) => row.globalMeanClosureDays !== null);
  const mean = reference?.globalMeanClosureDays ?? null;
  const sd = reference?.globalStddevClosureDays ?? null;
  const dayMax = Math.max(1, ...scaleRows.map((row) => row.averageClosureDays ?? 0), mean !== null && sd !== null ? mean + 2 * sd : 0);
  const c = scaleLinear([0, dayMax], [cols.c[0] + 4, cols.c[1] - 8]);
  const cMid = (cols.c[0] + cols.c[1]) / 2;

  const tooltipFor = (profile: AgencyPerformanceProfile) => (
    <TooltipContent
      title={profile.agencyName}
      rows={[
        { key: 'missed', value: `${countedLabel(profile, false)} (${percent(profile.statutoryViolationRate)})`, label: 'requests, counted deadline misses', swatch: 'flag', tone: 'counted' },
        { key: 'rapid', value: profile.suspiciousFastClosureCount.toLocaleString('en-US'), label: 'rapid closures (statistical)' },
        {
          key: 'average',
          value: profile.averageClosureDays === null ? 'not measured' : days(profile.averageClosureDays),
          label: profile.averageClosureDays === null ? NO_MEASURABLE_CLOSURE : `average closure, ${plural(profile.validLatencySampleSize, 'closure')}`,
        },
        ...(profile.performanceZScore === null ? [] : [{ key: 'z', value: formatValue(profile.performanceZScore, { kind: 'z', digits: 2 }), label: 'vs peers (statistical)' }]),
      ]}
      note={mean !== null && sd !== null ? `Peer mean ${days(mean)}, sd ${days(sd)}. Enter opens the department.` : 'Enter opens the department.'}
    />
  );

  return (
    <g>
      <ColumnHeader x={cols.a[0]} width={cols.a[1] - cols.a[0]} title="Counted deadline misses" note="Counted · of requests" flag />
      <ColumnHeader x={cols.b[0]} width={cols.b[1] - cols.b[0]} title="Rapid closures" note="statistical" />
      <ColumnHeader x={cols.c[0]} width={cols.c[1] - cols.c[0]} title="Closure time vs peers" note="statistical · days" />

      {mean !== null && sd !== null && sd > 0 && (
        <ReferenceBand axis="x" from={c(Math.max(0, mean - 2 * sd))} to={c(mean + 2 * sd)} extent={[top - 4, bottom + 16]} label="typical range (within 2 sd)" />
      )}
      {mean !== null && (
        <ReferenceLine kind="peer-mean" axis="x" at={c(mean)} extent={[HEADER + 2, bottom + 2]} label={`peer mean ${days(mean)}`} labelSide={c(mean) > cMid ? 'start' : 'end'} />
      )}

      {rows.map((profile, index) => {
        const y = top + index * rowStep;
        const barY = narrow ? y + 30 : y + (rowStep - BAND) / 2 - 4;
        const notes = rowNotes(profile, portalName ? portalName(profile.portalSubdomain) : null).join(' · ');
        const key = `${profile.portalSubdomain}-${profile.agencyId}`;
        const average = profile.averageClosureDays;
        const nameWidth = narrow ? width : layout.nameWidth;
        return (
          <MarkGroup
            key={key}
            box={{ x: 0, y: y + 1, width, height: rowStep - 6 }}
            interaction={{
              name: accessibleName(profile),
              item: roving.getItemProps(index),
              onClick: () => onInspect(profile),
              onPointerMove: (event) => tooltip.show(tooltipFor(profile), event),
              onPointerLeave: tooltip.hide,
              onFocusMark: (box) => tooltip.show(tooltipFor(profile), box),
              onBlurMark: tooltip.hide,
            }}
          >
            <text x={0} y={narrow ? y + 10 : notes ? barY + 2 : barY + BAND / 2} dy="0.32em" className={NAME_CLASS}>
              <title>{profile.agencyName}</title>
              {truncate(profile.agencyName, nameWidth - (narrow ? measureText(notes) + 12 : 0), NAME_FONT)}
            </text>
            {notes && (
              narrow
                ? <text x={width} y={y + 10} dy="0.32em" textAnchor="end" className={NOTE_CLASS}>{truncate(notes, width / 2)}</text>
                : <text x={0} y={barY + 18} dy="0.32em" className={NOTE_CLASS}>{truncate(notes, nameWidth)}</text>
            )}

            <rect x={a(0)} y={barY} width={a(profile.totalRequestsProcessed) - a(0)} height={BAND} rx={2} style={{ fill: VIZ.track }} />
            <Bar band={{ pos: barY, thickness: BAND }} value={measured(profile.statutoryViolationCount)} scale={a} claim="counted" label={countedLabel(profile, withRate)} motionKey={`ledger-a-${key}`} />

            <Bar band={{ pos: barY, thickness: BAND }} value={measured(profile.suspiciousFastClosureCount)} scale={b} claim="statistical" label={profile.suspiciousFastClosureCount.toLocaleString('en-US')} motionKey={`ledger-b-${key}`} />

            {average === null ? (
              <text x={cols.c[0] + 4} y={barY + BAND / 2} dy="0.32em" className={`${MUTED_LABEL_CLASS} italic`}>{NO_MEASURABLE_CLOSURE}</text>
            ) : (
              <>
                {isSlowerThanPeers(profile) && <Dot cx={c(average)} cy={barY + BAND / 2} r={7.5} shape="ring" claim="statistical" />}
                <Dot cx={c(average)} cy={barY + BAND / 2} role="primary" hollow={isSmallSample(profile)} />
              </>
            )}
          </MarkGroup>
        );
      })}

      <Axis scale={a} kind="count" orient="bottom" offset={bottom + 18} tickCount={Math.max(2, Math.min(4, Math.floor((cols.a[1] - cols.a[0]) / 70)))} />
      <Axis scale={b} kind="count" orient="bottom" offset={bottom + 18} tickCount={2} />
      <Axis scale={c} kind="days" orient="bottom" offset={bottom + 18} tickCount={Math.max(2, Math.min(5, Math.floor((cols.c[1] - cols.c[0]) / 70)))} />
      {tooltip.node}
    </g>
  );
}
