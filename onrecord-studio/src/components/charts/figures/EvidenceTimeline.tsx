import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import type { DateContradiction, TimelineLane as CaseLane } from '../../../utils/caseTimeline';
import { DATE_MEANING_LABELS, SOURCE_LABELS, type TimelineItem, type TimelineSource } from '../../../utils/timeline';
import Button from '../../ui/Button';
import { ShapeGlyph } from '../../ui/shapeGlyph';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import type { LegendItem } from '../frame/Legend';
import { ReferenceLine } from '../frame/ReferenceLine';
import type { TwinTable } from '../frame/TableTwin';
import { TooltipContent } from '../frame/Tooltip';
import { paintFor, VIZ } from '../core/claim';
import { formatDate, formatDateRange } from '../core/format';
import { absent, measured } from '../core/nulls';
import { parseDate, scaleTime, type TimeScale } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { HeatCell, seqStep } from '../marks/HeatCell';
import { AbsentSlot, LABEL_CLASS, MUTED_LABEL_CLASS, MarkGroup, type MarkInteraction } from '../marks/shared';

export type SourceLaneState = 'ready' | 'loading' | 'failed';

export interface SourceLaneInfo {
  source: TimelineSource;
  state: SourceLaneState;
  /** The source's load limit when it returned more rows than that; null when everything loaded. */
  cappedAt: number | null;
}

export interface EvidenceTimelineProps {
  /** Items under every filter except the date range, so the overview can widen the range again. */
  items: TimelineItem[];
  lanes: SourceLaneInfo[];
  caseLanes: CaseLane[];
  contradictions: DateContradiction[];
  /** Counts for the headline, from the date-filtered view the list shows. */
  listed: { items: TimelineItem[]; unreadable: number };
  from: string;
  to: string;
  onRange(from: string, to: string): void;
  activeLane: string | null;
  onLane(id: string | null): void;
  onOpen(itemId: string): void;
  /** The list row the pointer is over: its mark gets a ring. */
  highlight: string | null;
  today: number;
}

const DAY = 86_400_000;
const WEEK = 7 * DAY;
/** Above this many marks a lane becomes weekly count cells. */
export const DENSE_LANE = 150;
/** Room above the first lane for the 'Today' label, so it never sits on a lane's band. */
const TOP = 16;
const LANE_HEIGHT = 56;
/** The event baseline sits this far below a lane's top; the 'seen by this app' line SUB below that. */
const BASELINE = 18;
const SUB = 18;
const LABEL_COLUMN = 132;
const AXIS = 28;
const CONTRADICTION_ROW = 24;
const CONTRADICTION_GAP = 10;
/** Marks closer than this to their neighbour chain into one group. */
const CLUSTER_PX = 4;
/** A group this large is one count pill; a smaller one stacks STACK_CAP marks and counts the rest. */
const CLUSTER_MIN = 6;
const STACK_CAP = 2;
const STACK_STEP = 10;

interface Row {
  key: string;
  label: string;
  title: string;
  items: TimelineItem[];
  state: SourceLaneState;
  cappedAt: number | null;
  /** A case sub-lane id: clicking the label filters to it. */
  laneId?: string;
  sub?: boolean;
}

/** Marks that could not be drawn apart, listed under the chart when their pill or count is opened. */
export interface MarkGroupListing {
  title: string;
  items: TimelineItem[];
}

type Placed =
  | { kind: 'mark'; key: string; item: TimelineItem; cx: number; cy: number }
  | { kind: 'group'; key: string; items: TimelineItem[]; x0: number; x1: number; cy: number; pill: boolean };

const seenByApp = (item: TimelineItem) => item.dateMeaning === 'LAST_SEEN' || item.dateMeaning === 'DETECTED';
const isoDay = (ms: number) => new Date(ms).toISOString().slice(0, 10);
const weekStart = (ms: number) => {
  const d = new Date(ms);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate() - ((d.getUTCDay() + 6) % 7));
};
const byTime = (a: TimelineItem, b: TimelineItem) => Date.parse(a.at) - Date.parse(b.at) || a.id.localeCompare(b.id);
const plural = (n: number, one: string, many: string) => `${n.toLocaleString('en-US')} ${n === 1 ? one : many}`;

function markName(item: TimelineItem): string {
  return `${DATE_MEANING_LABELS[item.dateMeaning]} ${formatDate(parseDate(item.at), 'day')}: ${item.title}`;
}

function spanText(items: readonly TimelineItem[]): string {
  const times = items.map((item) => Date.parse(item.at));
  return formatDateRange(Math.min(...times), Math.max(...times));
}

export function timelineHeadline(listed: EvidenceTimelineProps['listed'], contradictions: number): string | null {
  const times = listed.items.map((item) => Date.parse(item.at)).filter(Number.isFinite);
  if (times.length === 0) return null;
  const sources = new Set(listed.items.map((item) => item.source)).size;
  const range = formatDateRange(Math.min(...times), Math.max(...times));
  const parts = [`${listed.items.length.toLocaleString('en-US')} dated ${listed.items.length === 1 ? 'item' : 'items'} from ${sources} ${sources === 1 ? 'source' : 'sources'}, ${range}`];
  if (contradictions > 0) parts.push(`${contradictions} date ${contradictions === 1 ? 'contradiction' : 'contradictions'} shown as recorded`);
  if (listed.unreadable > 0) parts.push(`${listed.unreadable} ${listed.unreadable === 1 ? 'item' : 'items'} with unreadable dates not placed`);
  return `${parts.join('; ')}.`;
}

/**
 * Places one row of marks on its line. Neighbours closer than CLUSTER_PX chain into a group (single
 * linkage, so a run of close marks is one group however far its ends are apart). A group of
 * CLUSTER_MIN or more is a count pill over its span; a smaller one stacks up to STACK_CAP marks
 * away from the other line (`direction`) and counts the rest beside them, so a stack never reaches
 * the other line or the next lane.
 */
export function placeRow(items: readonly TimelineItem[], x: (ms: number) => number, line: number, direction: 1 | -1): Placed[] {
  const sorted = items.filter((item) => Number.isFinite(Date.parse(item.at))).sort(byTime);
  const groups: TimelineItem[][] = [];
  let last = -Infinity;
  for (const item of sorted) {
    const px = x(Date.parse(item.at));
    if (groups.length && px - last <= CLUSTER_PX) groups[groups.length - 1].push(item);
    else groups.push([item]);
    last = px;
  }
  const placed: Placed[] = [];
  for (const group of groups) {
    const x0 = x(Date.parse(group[0].at));
    const x1 = x(Date.parse(group[group.length - 1].at));
    if (group.length >= CLUSTER_MIN) {
      placed.push({ kind: 'group', key: `pill:${group[0].id}`, items: group, x0, x1, cy: line, pill: true });
      continue;
    }
    group.slice(0, STACK_CAP).forEach((item, level) => {
      placed.push({ kind: 'mark', key: item.id, item, cx: x(Date.parse(item.at)), cy: line + direction * level * STACK_STEP });
    });
    if (group.length > STACK_CAP) {
      const anchor = x(Date.parse(group[STACK_CAP - 1].at));
      placed.push({ kind: 'group', key: `more:${group[0].id}`, items: group, x0: anchor + 8, x1: anchor + 8, cy: line + direction * (STACK_CAP - 1) * STACK_STEP, pill: false });
    }
  }
  return placed;
}

/** Both rows of a lane, in the order the arrow keys visit them: by position, the event line first. */
function placeLane(row: Row, x: TimeScale, y: number): Placed[] {
  const events = placeRow(row.items.filter((item) => !seenByApp(item)), x, y, -1);
  const seen = placeRow(row.items.filter(seenByApp), x, y + SUB, 1);
  const at = (p: Placed) => (p.kind === 'mark' ? p.cx : p.x0);
  return [...events.map((p) => ({ p, line: 0 })), ...seen.map((p) => ({ p, line: 1 }))]
    .sort((a, b) => at(a.p) - at(b.p) || a.line - b.line)
    .map(({ p }) => p);
}

/** Where each item is drawn: its own mark, or the pill or count that stands for it. */
function anchors(placed: readonly Placed[]): Map<string, { cx: number; cy: number; r: number }> {
  const map = new Map<string, { cx: number; cy: number; r: number }>();
  for (const p of placed) {
    if (p.kind === 'mark') map.set(p.item.id, { cx: p.cx, cy: p.cy, r: 8 });
    else for (const item of p.items) if (!map.has(item.id)) map.set(item.id, { cx: (p.x0 + p.x1) / 2, cy: p.cy, r: p.pill ? Math.max(11, (p.x1 - p.x0) / 2 + 10) : 9 });
  }
  return map;
}

function LaneMarks({ row, placed, onOpen, onGroup, tooltip }: { row: Row; placed: Placed[]; onOpen(id: string): void; onGroup(listing: MarkGroupListing): void; tooltip: ReturnType<typeof useTooltip> }) {
  const listing = (items: TimelineItem[]): MarkGroupListing => ({ title: `${plural(items.length, 'item', 'items')} in ${row.title}, ${spanText(items)}`, items });
  const activate = (p: Placed | undefined) => {
    if (!p) return;
    if (p.kind === 'mark') onOpen(p.item.id);
    else onGroup(listing(p.items));
  };
  const roving = useRovingFocus({ shape: { rows: Math.max(1, placed.length) }, onActivate: (i) => activate(placed[i]), onEscape: tooltip.hide });
  const role = row.key === 'FINDING' ? 'lead' : 'primary';
  return (
    <g role="group" aria-label={`${row.title}: ${plural(row.items.length, 'item', 'items')}`}>
      {placed.map((p, i) => {
        const interaction = (name: string, content: JSX.Element): MarkInteraction => ({
          name,
          item: roving.getItemProps(i),
          onClick: () => activate(p),
          onPointerMove: (pointer) => tooltip.show(content, pointer),
          onPointerLeave: tooltip.hide,
          onFocusMark: (box) => tooltip.show(content, box),
          onBlurMark: tooltip.hide,
        });
        if (p.kind === 'group') {
          const name = `${listing(p.items).title}, drawn as one ${p.pill ? 'count' : 'stack'}. Press Enter to list them.`;
          const content = <TooltipContent title={`${plural(p.items.length, 'item', 'items')} too close to draw apart`} rows={[]} note={`${spanText(p.items)}. Click to list them.`} />;
          const stroke = paintFor('descriptive', 'hollow', role).stroke;
          if (!p.pill) {
            const text = `+${p.items.length - STACK_CAP}`;
            return (
              <MarkGroup key={p.key} interaction={interaction(name, content)} box={{ x: p.x0 - 2, y: p.cy - 6, width: text.length * 6 + 4, height: 12 }}>
                <text x={p.x0} y={p.cy} dy="0.32em" className="viz-num fill-fg text-[10px] font-medium">{text}</text>
              </MarkGroup>
            );
          }
          const width = Math.max(20, p.x1 - p.x0 + 16);
          const left = (p.x0 + p.x1) / 2 - width / 2;
          // The pill spans the group's first to last date, in its members' colour, so it never moves a date.
          return (
            <MarkGroup key={p.key} interaction={interaction(name, content)} box={{ x: left, y: p.cy - 8, width, height: 16 }} ringRadius={9}>
              <rect x={left} y={p.cy - 8} width={width} height={16} rx={8} style={{ fill: VIZ.surface, stroke, strokeWidth: 1.5 }} />
              <text x={left + width / 2} y={p.cy} dy="0.32em" textAnchor="middle" className="viz-num fill-fg text-[10px] font-medium">{p.items.length}</text>
            </MarkGroup>
          );
        }
        const { item, cx, cy } = p;
        const shape = item.dateMeaning === 'DUE' ? 'diamond' : seenByApp(item) ? 'ring' : 'circle';
        // A ring or diamond is hollow and takes its role's stroke; only an event date is filled.
        const hollow = shape !== 'circle';
        const paint = paintFor('descriptive', hollow ? 'hollow' : 'fill', role);
        const content = (
          <TooltipContent
            title={item.title}
            rows={[{ key: 'd', value: formatDate(parseDate(item.at), 'day'), label: DATE_MEANING_LABELS[item.dateMeaning] }]}
            note={`${item.provenance}${item.citations && item.citations.length === 0 ? ' · no citation recorded' : ''}`}
          />
        );
        return (
          <MarkGroup key={p.key} interaction={interaction(markName(item), content)} box={{ x: cx - 4, y: cy - 4, width: 8, height: 8 }} ringRadius={7}>
            <title>{markName(item)}</title>
            <ShapeGlyph shape={shape === 'ring' ? 'circle' : shape} cx={cx} cy={cy} radius={6} style={{ fill: VIZ.surface }} />
            <ShapeGlyph
              shape={shape}
              cx={cx}
              cy={cy}
              radius={hollow ? 3.5 : 4}
              style={hollow ? { fill: shape === 'ring' ? 'none' : VIZ.surface, stroke: paint.stroke, strokeWidth: 1.5 } : { fill: paint.fill }}
            />
          </MarkGroup>
        );
      })}
    </g>
  );
}

/** Weekly counts, one row per date meaning: an event date and a date this app saw never share a cell. */
function DenseLane({ row, y, x }: { row: Row; y: number; x: TimeScale }) {
  const counts = [new Map<number, number>(), new Map<number, number>()];
  for (const item of row.items) {
    const at = Date.parse(item.at);
    if (!Number.isFinite(at)) continue;
    const cells = counts[seenByApp(item) ? 1 : 0];
    cells.set(weekStart(at), (cells.get(weekStart(at)) ?? 0) + 1);
  }
  const max = Math.max(1, ...counts[0].values(), ...counts[1].values());
  const meaning = ['event dates', 'dates this app saw'];
  // Weeks with no items draw nothing: an empty week is not a zero-shaded cell.
  return (
    <g role="group" aria-label={`${row.title}: ${plural(row.items.length, 'item', 'items')} as weekly counts, event dates on the lane line and dates this app saw on the dotted line`}>
      {counts.map((cells, line) => [...cells.entries()].map(([week, count]) => {
        const x0 = x(week);
        return (
          <HeatCell
            key={`${line}:${week}`}
            x={x0}
            y={(line ? y + SUB : y) - 7}
            width={Math.max(3, x(week + WEEK) - x0)}
            height={14}
            value={measured(count)}
            step={seqStep(count, max)}
            text={`${count} ${meaning[line]} in the week of ${formatDate(week, 'day')}`}
          />
        );
      }))}
    </g>
  );
}

function Contradictions({ entries, x, top, labelAbove, tooltip }: { entries: DateContradiction[]; x: TimeScale; top: number; labelAbove: boolean; tooltip: ReturnType<typeof useTooltip> }) {
  const roving = useRovingFocus({ shape: { rows: Math.max(1, entries.length) }, onActivate: () => {}, onEscape: tooltip.hide });
  const rowHeight = CONTRADICTION_ROW + (labelAbove ? 14 : 0);
  return (
    <g role="group" aria-label={plural(entries.length, 'date contradiction', 'date contradictions')}>
      {entries.map((entry, c) => {
        const readings = entry.readings.map((reading) => ({ reading, at: parseDate(reading.date) })).filter((r) => Number.isFinite(r.at)).sort((a, b) => a.at - b.at);
        if (readings.length === 0) return null;
        const cy = top + c * rowHeight + (labelAbove ? 14 : 0) + CONTRADICTION_ROW / 2;
        const xs = readings.map((r) => x(r.at));
        const mid = (xs[0] + xs[xs.length - 1]) / 2;
        const name = `${entry.subject}: ${readings.map((r) => `${formatDate(r.at, 'day')} (${r.reading.basis})`).join(' ≠ ')}. Both are shown as recorded.`;
        const content = <TooltipContent title={entry.subject} rows={readings.map((r, i) => ({ key: String(i), value: formatDate(r.at, 'day'), label: r.reading.basis }))} note="Both are shown as recorded; neither is chosen." />;
        const label = !labelAbove && entry.subject.length > 18 ? `${entry.subject.slice(0, 17)}…` : entry.subject;
        return (
          <g key={entry.id}>
            <text x={labelAbove ? Math.max(0, xs[0] - 4) : 0} y={labelAbove ? cy - 14 : cy} dy="0.32em" className={MUTED_LABEL_CLASS}>
              <title>{entry.subject}</title>
              {label}
            </text>
            <MarkGroup
              interaction={{
                name,
                item: roving.getItemProps(c),
                onPointerMove: (pointer) => tooltip.show(content, pointer),
                onPointerLeave: tooltip.hide,
                onFocusMark: (box) => tooltip.show(content, box),
                onBlurMark: tooltip.hide,
              }}
              box={{ x: xs[0] - 4, y: cy - 8, width: xs[xs.length - 1] - xs[0] + 8, height: 16 }}
              ringRadius={8}
            >
              {xs.length > 1 && <line x1={xs[0]} x2={xs[xs.length - 1]} y1={cy} y2={cy} style={{ stroke: VIZ.ink, strokeWidth: 2, strokeDasharray: '4 3' }} />}
              {xs.map((px, i) => <circle key={i} cx={px} cy={cy} r={3.5} style={{ fill: VIZ.surface, stroke: VIZ.ink, strokeWidth: 1.5 }} />)}
              {xs.length > 1 && (
                <g>
                  <rect x={mid - 8} y={cy - 8} width={16} height={16} rx={4} style={{ fill: VIZ.surface, stroke: VIZ.ink, strokeWidth: 1 }} />
                  <text x={mid} y={cy} dy="0.35em" textAnchor="middle" className="fill-fg text-[12px] font-semibold">≠</text>
                </g>
              )}
            </MarkGroup>
          </g>
        );
      })}
    </g>
  );
}

const contradictionsHeight = (count: number, labelAbove: boolean) => (count ? CONTRADICTION_GAP + count * (CONTRADICTION_ROW + (labelAbove ? 14 : 0)) : 0);

function Plot({ props, rows, width, onGroup }: { props: EvidenceTimelineProps; rows: Row[]; width: number; onGroup(listing: MarkGroupListing): void }) {
  const tooltip = useTooltip();
  const [drag, setDrag] = useState<{ x0: number; x1: number } | null>(null);
  const times = props.items.map((item) => Date.parse(item.at)).filter(Number.isFinite);
  // With nothing dated (every source failed), the axis covers the last 30 days so the failure bands have a place.
  const lo = times.length ? Math.min(...times) : props.today - 30 * DAY;
  const hi = times.length ? Math.max(...times) : props.today;
  const pad = Math.max(DAY, (hi - lo) * 0.02);
  const plotX = width < 480 ? 0 : LABEL_COLUMN;
  const labelAbove = plotX === 0;
  const laneHeight = LANE_HEIGHT + (labelAbove ? 14 : 0);
  const right = width - 12;
  const x = scaleTime([lo - pad, hi + pad], [plotX + 10, right]);
  const lanesBottom = TOP + rows.length * laneHeight;
  const axisY = lanesBottom + contradictionsHeight(props.contradictions.length, labelAbove);
  const [d0, d1] = x.domain;
  const brushFrom = props.from ? Math.max(d0, parseDate(props.from)) : null;
  const brushTo = props.to ? Math.min(d1, parseDate(props.to) + DAY) : null;
  const brushed = brushFrom !== null || brushTo !== null ? [x(brushFrom ?? d0), x(brushTo ?? d1)] : null;
  const local = (event: ReactPointerEvent<SVGRectElement>) => {
    const box = (event.currentTarget.ownerSVGElement ?? event.currentTarget).getBoundingClientRect();
    return Math.max(plotX + 10, Math.min(right, event.clientX - box.left));
  };
  const finish = () => {
    if (drag && Math.abs(drag.x1 - drag.x0) >= 4) {
      const [a, b] = [Math.min(drag.x0, drag.x1), Math.max(drag.x0, drag.x1)];
      props.onRange(isoDay(x.invert(a)), isoDay(x.invert(b)));
    }
    setDrag(null);
  };
  const laneY = (r: number) => TOP + r * laneHeight + (labelAbove ? 14 : 0) + BASELINE;
  const marked = rows.map((row, r) => (row.state === 'ready' && row.items.length <= DENSE_LANE ? placeLane(row, x, laneY(r)) : null));
  // The highlight ring is placed from the same placement as the marks, so it lands on the item's own mark or the group that holds it.
  const highlightRow = props.highlight ? rows.findIndex((row, r) => !row.sub && marked[r] !== null && row.items.some((item) => item.id === props.highlight)) : -1;
  const ring = highlightRow >= 0 && props.highlight ? anchors(marked[highlightRow] as Placed[]).get(props.highlight) : undefined;
  return (
    <>
      <rect
        x={plotX + 10}
        y={0}
        width={Math.max(0, right - plotX - 10)}
        height={axisY}
        fill="transparent"
        style={{ cursor: 'col-resize' }}
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture?.(event.pointerId);
          const at = local(event);
          setDrag({ x0: at, x1: at });
        }}
        onPointerMove={(event) => drag && setDrag({ ...drag, x1: local(event) })}
        onPointerUp={finish}
        onPointerCancel={() => setDrag(null)}
      />
      {(drag || brushed) && (() => {
        const [a, b] = drag ? [Math.min(drag.x0, drag.x1), Math.max(drag.x0, drag.x1)] : (brushed as number[]);
        return (
          <g pointerEvents="none" aria-hidden="true">
            <rect x={a} y={0} width={Math.max(1, b - a)} height={axisY} style={{ fill: VIZ.track, opacity: 0.55 }} />
            <line x1={a + 0.5} x2={a + 0.5} y1={0} y2={axisY} style={{ stroke: VIZ.ink, strokeWidth: 1 }} />
            <line x1={b - 0.5} x2={b - 0.5} y1={0} y2={axisY} style={{ stroke: VIZ.ink, strokeWidth: 1 }} />
          </g>
        );
      })()}
      {props.today >= d0 && props.today <= d1 && <ReferenceLine kind="today" axis="x" at={x(props.today)} extent={[0, axisY]} label="Today" labelSide="start" />}
      {rows.map((row, r) => {
        const top = TOP + r * laneHeight;
        const y = laneY(r);
        const active = row.laneId !== undefined && row.laneId === props.activeLane;
        const dated = row.items.map((item) => Date.parse(item.at)).filter(Number.isFinite);
        const oldest = dated.length ? Math.min(...dated) : null;
        const label = (
          <text x={labelAbove ? 0 : row.sub ? 12 : 0} y={labelAbove ? top + 10 : y} dy={labelAbove ? undefined : '0.32em'} className={row.sub ? MUTED_LABEL_CLASS : 'fill-fg-secondary text-[12px]'} style={active ? { fontWeight: 600 } : undefined}>
            <title>{row.title}</title>
            {row.label.length > 18 && !labelAbove ? `${row.label.slice(0, 17)}…` : row.label}
            {active ? ' (filtering)' : ''}
          </text>
        );
        return (
          <g key={row.key}>
            <line x1={plotX + 10} x2={right} y1={y + 0.5} y2={y + 0.5} style={{ stroke: VIZ.grid, strokeWidth: 1 }} aria-hidden="true" />
            {row.items.some(seenByApp) && (
              <line x1={plotX + 10} x2={right} y1={y + SUB + 0.5} y2={y + SUB + 0.5} style={{ stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: '1 3' }} aria-hidden="true" />
            )}
            {row.laneId !== undefined ? (
              <g
                role="button"
                tabIndex={0}
                aria-pressed={active}
                aria-label={`${active ? 'Show all lanes' : `Show only the ${row.title} lane`}`}
                className="viz-focusable cursor-pointer"
                onClick={() => props.onLane(active ? null : (row.laneId as string))}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    props.onLane(active ? null : (row.laneId as string));
                  }
                }}
              >
                {label}
              </g>
            ) : label}
            {row.cappedAt !== null && row.state === 'ready' && oldest !== null && x(oldest) - 6 - (plotX + 10) >= 4 && (
              // Coverage of a capped source ends at its oldest loaded item; the period before it was not loaded, not empty.
              <AbsentSlot
                absence={absent('not-covered', `only the most recent ${row.cappedAt} ${row.title.toLowerCase()} were loaded; anything older is not shown`)}
                box={{ x: plotX + 10, y: y - 9, width: x(oldest) - 6 - (plotX + 10), height: SUB + 18 }}
                text="older not loaded"
              />
            )}
            {row.state === 'failed' ? (
              <AbsentSlot absence={absent('not-recorded', `${row.title} could not be read`)} box={{ x: plotX + 10, y: y - 9, width: right - plotX - 10, height: 18 }} text="could not load" />
            ) : row.state === 'loading' ? (
              <text x={plotX + 14} y={y} dy="0.32em" className={MUTED_LABEL_CLASS}>loading…</text>
            ) : marked[r] === null ? (
              <DenseLane row={row} y={y} x={x} />
            ) : (
              <LaneMarks row={row} placed={marked[r] as Placed[]} onOpen={props.onOpen} onGroup={onGroup} tooltip={tooltip} />
            )}
            {row.cappedAt !== null && (
              <text x={right} y={top + (labelAbove ? 10 : 4)} dy={labelAbove ? undefined : '0.9em'} textAnchor="end" className={LABEL_CLASS}>{`most recent ${row.cappedAt} loaded; older not shown`}</text>
            )}
          </g>
        );
      })}
      {ring && <circle cx={ring.cx} cy={ring.cy} r={ring.r} pointerEvents="none" style={{ fill: 'none', stroke: VIZ.focus, strokeWidth: 2 }} />}
      {props.contradictions.length > 0 && <Contradictions entries={props.contradictions} x={x} top={lanesBottom + CONTRADICTION_GAP} labelAbove={labelAbove} tooltip={tooltip} />}
      <Axis time={x} orient="bottom" offset={axisY} />
      {tooltip.node}
    </>
  );
}

function rowsOf(props: EvidenceTimelineProps, expanded: boolean): Row[] {
  const rows: Row[] = [];
  for (const lane of props.lanes) {
    const items = props.items.filter((item) => item.source === lane.source);
    rows.push({ key: lane.source, label: SOURCE_LABELS[lane.source], title: SOURCE_LABELS[lane.source], items, state: lane.state, cappedAt: lane.cappedAt });
    if (lane.source === 'CASE' && expanded && lane.state === 'ready') {
      for (const sub of props.caseLanes) {
        rows.push({ key: `lane:${sub.id}`, label: sub.label, title: sub.label, items: items.filter((item) => (item.lanes ?? []).includes(sub.id)), state: 'ready', cappedAt: null, laneId: sub.id, sub: true });
      }
    }
  }
  return rows;
}

/** The items of an opened pill or count, as buttons that open each one's list row. */
function GroupListing({ listing, onOpen, onClose }: { listing: MarkGroupListing; onOpen(id: string): void; onClose(): void }) {
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => heading.current?.focus(), [listing]);
  return (
    <section
      aria-label={listing.title}
      className="app-panel grid gap-2 p-3"
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          onClose();
        }
      }}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h4 ref={heading} tabIndex={-1} className="text-control font-medium text-fg outline-none">{listing.title}</h4>
        <Button size="sm" variant="ghost" onClick={onClose}>Close list</Button>
      </div>
      <ul className="grid gap-1">
        {[...listing.items].sort(byTime).map((item) => (
          <li key={item.id}>
            <button type="button" className="focus-ring flex w-full flex-wrap items-baseline gap-x-2 rounded px-1 text-left text-caption hover:bg-raised" onClick={() => onOpen(item.id)}>
              <span className="viz-num font-medium text-fg-secondary">{formatDate(parseDate(item.at), 'day')}</span>
              <span className="text-muted">{DATE_MEANING_LABELS[item.dateMeaning]}</span>
              <span className="text-fg">{item.title}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

/**
 * F13 EvidenceTimeline: one swimlane per source over a shared time axis. Date meaning is carried by
 * position and shape, never colour: an event date is a filled circle on the lane line, a due or
 * modelled date a hollow diamond on it, and a date this app saw something a ring on the dotted 'seen
 * by this app' line below. Stacks grow away from the other line and never reach it. Findings are
 * violet (leads the app surfaced); everything else is ink. Each contradiction has its own row: both
 * readings, joined by an ink dashed connector and '≠'. The brush and the From/To inputs share one
 * source of truth.
 */
export default function EvidenceTimeline(props: EvidenceTimelineProps) {
  const [expanded, setExpanded] = useState(false);
  const [listing, setListing] = useState<MarkGroupListing | null>(null);
  const rows = rowsOf(props, expanded);
  const dated = props.items.filter((item) => Number.isFinite(Date.parse(item.at)));
  const labelAbove = (width: number) => width < 480;
  const height = (width: number) => TOP + rows.length * (LANE_HEIGHT + (labelAbove(width) ? 14 : 0)) + contradictionsHeight(props.contradictions.length, labelAbove(width)) + AXIS;
  const dense = rows.some((row) => row.items.length > DENSE_LANE);
  const legend: LegendItem[] = [
    { key: 'occurred', label: 'Event date (lane line)', swatch: 'dot', tone: 'ink' },
    { key: 'due', label: 'Owed or modelled date', swatch: 'diamond', tone: 'ink' },
    { key: 'seen', label: 'Date this app saw it (dotted line)', swatch: 'ring-dotted', tone: 'ink' },
    { key: 'finding', label: 'Finding (lead)', swatch: 'ring', tone: 'lead' },
    ...(props.contradictions.length ? [{ key: 'contradiction', label: 'Contradicting dates (≠)', swatch: 'dashed-line' as const, tone: 'ink' as const }] : []),
    ...(dense ? [{ key: 'weekly', label: `Items per week, fewer to more (lanes over ${DENSE_LANE} items)`, swatch: 'ramp' as const }] : []),
  ];
  const table: TwinTable = {
    caption: 'Timeline items in the overview',
    columns: [{ key: 'date', label: 'Date' }, { key: 'meaning', label: 'Date meaning' }, { key: 'source', label: 'Source' }, { key: 'title', label: 'Item' }],
    rows: [
      ...[...dated].sort(byTime).map((item) => ({ date: formatDate(parseDate(item.at), 'day'), meaning: DATE_MEANING_LABELS[item.dateMeaning], source: SOURCE_LABELS[item.source], title: item.title })),
      ...props.contradictions.map((entry) => ({
        date: entry.readings.map((reading) => `${reading.date} (${reading.basis})`).join(' ≠ '),
        meaning: 'Contradicting dates, both shown as recorded',
        source: SOURCE_LABELS.CASE,
        title: entry.subject,
      })),
      ...props.lanes.filter((lane) => lane.cappedAt !== null).map((lane) => ({ date: absent('not-covered', `only the most recent ${lane.cappedAt} ${SOURCE_LABELS[lane.source].toLowerCase()} were loaded; older ones are not shown`), meaning: null, source: SOURCE_LABELS[lane.source], title: null })),
      ...props.lanes.filter((lane) => lane.state === 'failed').map((lane) => ({ date: absent('not-recorded', `${SOURCE_LABELS[lane.source]} could not be read`), meaning: null, source: SOURCE_LABELS[lane.source], title: null })),
      ...(props.listed.unreadable ? [{ date: absent('excluded', `${props.listed.unreadable} items have a date that could not be read`), meaning: null, source: null, title: null }] : []),
    ],
  };
  const failed = props.lanes.filter((lane) => lane.state === 'failed').length;
  return (
    <div className="grid gap-2">
      <ChartFrame
        title="Timeline overview"
        headline={timelineHeadline(props.listed, props.contradictions.length)}
        sources={{ tables: ['tracker notebook', 'findings', 'watch_changes', 'casework_cases', 'casework_events'] }}
        summary={`${dated.length} dated items in ${rows.length} lanes${failed ? `; ${failed} sources could not be read` : ''}${props.listed.unreadable ? `; ${props.listed.unreadable} undated items not placed` : ''}${props.lanes.filter((lane) => lane.cappedAt !== null).map((lane) => `; only the most recent ${lane.cappedAt} ${SOURCE_LABELS[lane.source].toLowerCase()} loaded, older ones not shown`).join('')}. Drag across the lanes to set the From and To dates.`}
        status={dated.length || failed ? 'ready' : 'empty'}
        emptyText="No dated items to place for these sources and filters."
        table={table}
        legend={legend}
        controls={
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-caption text-muted">
            {props.caseLanes.length > 0 && props.lanes.some((lane) => lane.source === 'CASE' && lane.state === 'ready') && (
              <button type="button" className="focus-ring rounded text-accent underline-offset-2 hover:underline" aria-expanded={expanded} onClick={() => setExpanded((current) => !current)}>
                {expanded ? 'Hide case sub-lanes' : `Show case sub-lanes (${props.caseLanes.length})`}
              </button>
            )}
            {props.listed.unreadable > 0 && <span className="rounded border border-dashed border-line-strong px-1.5">{`${props.listed.unreadable} undated, not placed`}</span>}
            {(props.from || props.to) && (
              <button type="button" className="focus-ring rounded text-accent underline-offset-2 hover:underline" onClick={() => props.onRange('', '')}>Clear date range</button>
            )}
          </div>
        }
        minHeight={height(640)}
        plotHeight={height}
        interactive={{ roledescription: 'timeline' }}
        storageKey="evidence-timeline-overview"
      >
        {({ width }) => (dated.length || failed ? <Plot props={props} rows={rows} width={width} onGroup={setListing} /> : null)}
      </ChartFrame>
      {listing && <GroupListing listing={listing} onOpen={props.onOpen} onClose={() => setListing(null)} />}
    </div>
  );
}
