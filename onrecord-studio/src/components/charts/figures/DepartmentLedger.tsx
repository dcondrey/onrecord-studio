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
import { hBarPath, LABEL_CLASS, MarkGroup, MUTED_LABEL_CLASS } from '../marks/shared';

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

const NAME_FONT = '500 12px -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif';
const HEADER_FONT = '500 11px -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif';
const NOTE_FONT = '11px -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif';
const NAME_CLASS = 'fill-fg text-[12px] font-medium';
const NOTE_CLASS = 'fill-muted text-[11px]';
const HEADER_CLASS = 'fill-fg-secondary text-[11px] font-medium';
const BAND = 14;
/** The requests extent beyond the counted bar: a thinner bar, so it never reads as more misses. */
const REQUESTS_BAND = 6;
const MEAN_STRIP = 16;
const FOOT = 40;
const GUTTER = 20;
/** The three columns' least usable width beside an inline name column; below it names go above. */
const MIN_COLUMNS = 440;
const COLUMN_TITLES = { a: 'Counted deadline misses', b: 'Rapid closures', c: 'Closure time vs peers' } as const;
/**
 * What each column's figure is, under its title: the counted column is the one legal claim, and
 * the other two say they are not, so the frame itself never reads as both.
 */
const COLUMN_NOTES = { a: 'Counted (legal claim)', b: 'Statistical, not a legal finding', c: 'Statistical, not a legal finding' } as const;
const LINE = 13;
const FLAG_INDENT = 18;

function truncate(text: string, width: number, font?: string): string {
  if (width <= 0) return '';
  if (measureText(text, font) <= width) return text;
  let end = text.length;
  while (end > 1 && measureText(`${text.slice(0, end)}…`, font) > width) end -= 1;
  return `${text.slice(0, end).trimEnd()}…`;
}

/** Text set on as many lines as `width` needs, broken at words; never cut. */
export function wrapWords(text: string, width: number, font = HEADER_FONT): string[] {
  const lines: string[] = [];
  for (const word of text.split(' ')) {
    const last = lines.length - 1;
    if (last >= 0 && measureText(`${lines[last]} ${word}`, font) <= width) lines[last] = `${lines[last]} ${word}`;
    else lines.push(word);
  }
  return lines;
}

interface Layout {
  narrow: boolean;
  nameWidth: number;
  rowStep: number;
  header: number;
  titles: { a: string[]; b: string[]; c: string[] };
  notes: { a: string[]; b: string[]; c: string[] };
  cols: { a: [number, number]; b: [number, number]; c: [number, number] };
  height(rows: number): number;
}

/**
 * Three columns, each on its own x-scale, beside a name column sized to the longest name (and its
 * notes). When that column would leave the bars too little room, every name moves onto its own
 * line above its bars instead of being cut. `nameNeed` must be the same for the frame's height and
 * the plot, so both agree on the row step.
 */
function ledgerLayout(width: number, nameNeed: number): Layout {
  const inlineName = Math.max(140, Math.ceil(nameNeed) + 4);
  const narrow = width < 640 || inlineName > width * 0.4 || width - inlineName - 12 - 2 * GUTTER < MIN_COLUMNS;
  const nameWidth = narrow ? width : inlineName;
  const start = narrow ? 0 : nameWidth + 12;
  const rest = width - start - 2 * GUTTER;
  const a = Math.round(rest * 0.38);
  const b = Math.round(rest * 0.2);
  const cols = {
    a: [start, start + a] as [number, number],
    b: [start + a + GUTTER, start + a + GUTTER + b] as [number, number],
    c: [start + a + b + 2 * GUTTER, width] as [number, number],
  };
  const titles = {
    a: wrapWords(COLUMN_TITLES.a, cols.a[1] - cols.a[0] - FLAG_INDENT),
    b: wrapWords(COLUMN_TITLES.b, cols.b[1] - cols.b[0]),
    c: wrapWords(COLUMN_TITLES.c, cols.c[1] - cols.c[0]),
  };
  const notes = {
    a: wrapWords(COLUMN_NOTES.a, cols.a[1] - cols.a[0], NOTE_FONT),
    b: wrapWords(COLUMN_NOTES.b, cols.b[1] - cols.b[0], NOTE_FONT),
    c: wrapWords(COLUMN_NOTES.c, cols.c[1] - cols.c[0], NOTE_FONT),
  };
  const lines = Math.max(...(['a', 'b', 'c'] as const).map((column) => titles[column].length + notes[column].length));
  const header = 8 + lines * LINE;
  const rowStep = narrow ? 54 : 38;
  return { narrow, nameWidth, rowStep, header, titles, notes, cols, height: (rows) => header + MEAN_STRIP + rows * rowStep + FOOT };
}

/** The widest name or note line a row puts in the name column. */
function nameNeedOf(rows: readonly AgencyPerformanceProfile[], portalName: ((key: string) => string) | null): number {
  return Math.max(0, ...rows.map((row) => Math.max(
    measureText(row.agencyName, NAME_FONT),
    measureText(rowNotes(row, portalName ? portalName(row.portalSubdomain) : null).join(' · ')),
  )));
}

/** As many ticks as fit with their widest label and a gap, from two to `most`. */
function tickCountFor(length: number, widestLabel: string, most: number): number {
  return Math.max(2, Math.min(most, Math.floor(length / (measureText(widestLabel) + 40))));
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
  { key: 'requests', label: 'Requests processed', swatch: 'rect', tone: 'mark' },
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
  const portalNameFn = spansPortals ? portalName : null;
  const nameNeed = nameNeedOf(shown, portalNameFn);
  const summary = `${plural(all.length, 'department')} with requests${shown.length < all.length ? `, the first ${shown.length} shown` : ''}. `
    + `${all.filter(isSlowerThanPeers).length} slower than peers at z ≥ ${PEER_LATENCY_OUTLIER_Z_SCORE} (statistical). `
    + `${unmeasured} with ${NO_MEASURABLE_CLOSURE}; ${small} resting on fewer than ${SMALL_SAMPLE_CLOSURES} closures.`;

  return (
    <div className="grid gap-2">
      <ChartFrame
        title="Department deadline ledger"
        storageKey={`department-ledger-${variant}`}
        headline={headline}
        sources={{
          tables: ['requests', 'statutory_extensions', 'agencies'],
          note: 'misses counted under Cal. Gov. Code § 7922.535; rapid closures and closure time are statistical comparisons, not legal findings; peers are every measured closure in the workspace, all portals, not only the departments shown',
        }}
        summary={summary}
        status={empty ? 'empty' : status}
        emptyText="No department has a request in this scope."
        error={error}
        scopedOutText={scopedOutText}
        table={twinTable(shown, portalName)}
        legend={LEGEND}
        controls={controls}
        minHeight={ledgerLayout(640, nameNeed).height(Math.max(1, Math.min(shown.length, 3)))}
        plotHeight={(width) => ledgerLayout(width, nameNeed).height(shown.length)}
        interactive={{ roledescription: 'department ledger' }}
      >
        {({ width }) => (
          <LedgerPlot
            width={width}
            rows={shown}
            scaleRows={scaleProfiles && scaleProfiles.length > 0 ? scaleProfiles : all}
            portalName={portalNameFn}
            nameNeed={nameNeed}
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

function ColumnHeader({ x, lines, note, flag = false }: { x: number; lines: readonly string[]; note: readonly string[]; flag?: boolean }) {
  const titleX = flag ? x + FLAG_INDENT : x;
  return (
    <g aria-hidden="true">
      {flag && (
        <g transform={`translate(${x} ${2})`}>
          <LegendSwatch kind="flag" tone="counted" />
        </g>
      )}
      <text x={titleX} y={11} className={HEADER_CLASS}>
        <title>{`${lines.join(' ')}: ${note.join(' ')}`}</title>
        {lines.map((line, i) => <tspan key={line} x={titleX} dy={i === 0 ? 0 : LINE}>{line}</tspan>)}
      </text>
      <text x={x} y={11 + lines.length * LINE} className={NOTE_CLASS}>
        {note.map((line, i) => <tspan key={line} x={x} dy={i === 0 ? 0 : LINE}>{line}</tspan>)}
      </text>
    </g>
  );
}

function LedgerPlot({ width, rows, scaleRows, portalName, nameNeed, onInspect }: {
  width: number;
  rows: readonly AgencyPerformanceProfile[];
  scaleRows: readonly AgencyPerformanceProfile[];
  portalName: ((key: string) => string) | null;
  nameNeed: number;
  onInspect(profile: AgencyPerformanceProfile): void;
}) {
  const tooltip = useTooltip();
  const roving = useRovingFocus({ shape: { rows: rows.length }, onActivate: (i) => onInspect(rows[i]), onEscape: tooltip.hide });
  const layout = ledgerLayout(width, nameNeed);
  const { cols, rowStep, narrow, header } = layout;
  const top = header + MEAN_STRIP;
  const bottom = top + rows.length * rowStep;

  const withRate = cols.a[1] - cols.a[0] >= 220;
  const longestLabel = Math.max(...scaleRows.map((row) => measureText(countedLabel(row, withRate))), 0);
  const requestsMax = Math.max(1, ...scaleRows.map((row) => row.totalRequestsProcessed));
  // Each column's scale stops short of its end by its widest value label, which is drawn after the
  // row's longest mark (requests, in column a), so every label fits and none sits on a mark.
  const a: ContinuousScale = scaleLinear([0, requestsMax], [cols.a[0], Math.max(cols.a[0] + 24, cols.a[1] - longestLabel - 8)], { nice: false });
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
      note={mean !== null && sd !== null ? `Mean of all closures, all portals ${days(mean)}, sd ${days(sd)}. Enter opens the department.` : 'Enter opens the department.'}
    />
  );

  return (
    <g>
      <ColumnHeader x={cols.a[0]} lines={layout.titles.a} note={layout.notes.a} flag />
      <ColumnHeader x={cols.b[0]} lines={layout.titles.b} note={layout.notes.b} />
      <ColumnHeader x={cols.c[0]} lines={layout.titles.c} note={layout.notes.c} />

      {mean !== null && sd !== null && sd > 0 && (
        <ReferenceBand axis="x" from={c(Math.max(0, mean - 2 * sd))} to={c(mean + 2 * sd)} extent={[top - 4, bottom + 16]} label="within 2 sd of all closures" />
      )}
      {mean !== null && (
        <ReferenceLine kind="peer-mean" axis="x" at={c(mean)} extent={[header + 2, bottom + 2]} label={`mean of all closures ${days(mean)}`} labelSide={c(mean) > cMid ? 'start' : 'end'} />
      )}

      {rows.map((profile, index) => {
        const y = top + index * rowStep;
        const barY = narrow ? y + 30 : y + (rowStep - BAND) / 2 - 4;
        const notes = rowNotes(profile, portalName ? portalName(profile.portalSubdomain) : null).join(' · ');
        const key = `${profile.portalSubdomain}-${profile.agencyId}`;
        const average = profile.averageClosureDays;
        const nameWidth = layout.nameWidth;
        const misses = profile.statutoryViolationCount;
        const requestsEnd = a(profile.totalRequestsProcessed);
        const missEnd = a(misses);
        const rapid = profile.suspiciousFastClosureCount;
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
              {truncate(profile.agencyName, nameWidth, NAME_FONT)}
            </text>
            {notes && (
              narrow
                ? <NarrowNotes notes={notes} x={width} y={y + 10} room={width - measureText(profile.agencyName, NAME_FONT) - 12} />
                : <text x={0} y={barY + 18} dy="0.32em" className={NOTE_CLASS}>{truncate(notes, nameWidth)}</text>
            )}

            {requestsEnd - missEnd > (misses > 0 ? 4 : 0) && (
              <path d={hBarPath(misses > 0 ? missEnd + 2 : a(0), requestsEnd, barY + (BAND - REQUESTS_BAND) / 2, REQUESTS_BAND, 2)} style={{ fill: VIZ.mark }} />
            )}
            {misses > 0
              ? <Bar band={{ pos: barY, thickness: BAND }} value={measured(misses)} scale={a} claim="counted" motionKey={`ledger-a-${key}`} />
              : <line x1={a(0)} x2={a(0)} y1={barY} y2={barY + BAND} style={{ stroke: VIZ.counted, strokeWidth: 1.5 }} />}
            <text x={Math.max(requestsEnd, missEnd) + 6} y={barY + BAND / 2} dy="0.32em" className={LABEL_CLASS}>{countedLabel(profile, withRate)}</text>

            <Bar band={{ pos: barY, thickness: BAND }} value={measured(rapid)} scale={b} claim="statistical" motionKey={`ledger-b-${key}`} />
            {rapid > 0 && <text x={b(rapid) + 6} y={barY + BAND / 2} dy="0.32em" className={LABEL_CLASS}>{rapid.toLocaleString('en-US')}</text>}

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

      <Axis scale={a} kind="count" orient="bottom" offset={bottom + 18} tickCount={tickCountFor(a.range[1] - a.range[0], requestsMax.toLocaleString('en-US'), 4)} />
      <Axis scale={b} kind="count" orient="bottom" offset={bottom + 18} tickCount={2} />
      <Axis scale={c} kind="days" orient="bottom" offset={bottom + 18} tickCount={tickCountFor(cols.c[1] - cols.c[0], days(dayMax), 5)} />
      {tooltip.node}
    </g>
  );
}

/** Notes on the name's line in the narrow layout, right-aligned in whatever the name leaves. */
function NarrowNotes({ notes, x, y, room }: { notes: string; x: number; y: number; room: number }) {
  const text = truncate(notes, room);
  if (!text) return null;
  return (
    <text x={x} y={y} dy="0.32em" textAnchor="end" className={NOTE_CLASS}>
      {text !== notes && <title>{notes}</title>}
      {text}
    </text>
  );
}
