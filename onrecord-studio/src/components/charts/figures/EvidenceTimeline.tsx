import { useState, type PointerEvent as ReactPointerEvent } from 'react';
import type { DateContradiction, TimelineLane as CaseLane } from '../../../utils/caseTimeline';
import { DATE_MEANING_LABELS, SOURCE_LABELS, type TimelineItem, type TimelineSource } from '../../../utils/timeline';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import type { LegendItem } from '../frame/Legend';
import { ReferenceLine } from '../frame/ReferenceLine';
import type { TwinTable } from '../frame/TableTwin';
import { TooltipContent } from '../frame/Tooltip';
import { VIZ } from '../core/claim';
import { formatDate, formatDateRange } from '../core/format';
import { absent, measured } from '../core/nulls';
import { parseDate, scaleTime, type TimeScale } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { HeatCell, seqStep } from '../marks/HeatCell';
import { TimelineLane, type LaneEvent } from '../marks/TimelineLane';
import { AbsentSlot, LABEL_CLASS, MUTED_LABEL_CLASS } from '../marks/shared';

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
const LANE_HEIGHT = 40;
const LABEL_COLUMN = 132;
const AXIS = 28;
const CONTRADICTION_ROW = 34;

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

const seenByApp = (item: TimelineItem) => item.dateMeaning === 'LAST_SEEN' || item.dateMeaning === 'DETECTED';
const isoDay = (ms: number) => new Date(ms).toISOString().slice(0, 10);
const weekStart = (ms: number) => {
  const d = new Date(ms);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate() - ((d.getUTCDay() + 6) % 7));
};

function markName(item: TimelineItem): string {
  return `${DATE_MEANING_LABELS[item.dateMeaning]} ${formatDate(parseDate(item.at), 'day')}: ${item.title}`;
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

function LaneMarks({ row, y, x, onOpen, tooltip }: { row: Row; y: number; x: TimeScale; onOpen(id: string): void; tooltip: ReturnType<typeof useTooltip> }) {
  const sorted = [...row.items].sort((a, b) => Date.parse(a.at) - Date.parse(b.at) || a.id.localeCompare(b.id));
  const roving = useRovingFocus({ shape: { rows: Math.max(1, sorted.length) }, onActivate: (i) => sorted[i] && onOpen(sorted[i].id), onEscape: tooltip.hide });
  const index = new Map(sorted.map((item, i) => [item.id, i] as const));
  const event = (item: TimelineItem): LaneEvent => {
    const content = (
      <TooltipContent
        title={item.title}
        rows={[{ key: 'd', value: formatDate(parseDate(item.at), 'day'), label: DATE_MEANING_LABELS[item.dateMeaning] }]}
        note={`${item.provenance}${item.citations && item.citations.length === 0 ? ' · no citation recorded' : ''}`}
      />
    );
    return {
      key: item.id,
      at: Date.parse(item.at),
      shape: item.dateMeaning === 'DUE' ? 'diamond' : seenByApp(item) ? 'ring' : 'circle',
      // A ring must be hollow to take its role's stroke; a filled ring would have no stroke at all.
      hollow: item.dateMeaning === 'DUE' || seenByApp(item),
      role: item.source === 'FINDING' ? 'lead' : 'primary',
      name: markName(item),
      interaction: {
        name: markName(item),
        item: roving.getItemProps(index.get(item.id) ?? 0),
        onClick: () => onOpen(item.id),
        onPointerMove: (pointer) => tooltip.show(content, pointer),
        onPointerLeave: tooltip.hide,
        onFocusMark: (box) => tooltip.show(content, box),
        onBlurMark: tooltip.hide,
      },
    };
  };
  const onBaseline = sorted.filter((item) => !seenByApp(item));
  const seen = sorted.filter(seenByApp);
  return (
    <g role="group" aria-label={`${row.title}: ${row.items.length} ${row.items.length === 1 ? 'item' : 'items'}`}>
      <TimelineLane y={y} x={x} events={onBaseline.map(event)} />
      {seen.length > 0 && <TimelineLane y={y + 12} x={x} events={seen.map(event)} />}
    </g>
  );
}

function DenseLane({ row, y, x }: { row: Row; y: number; x: TimeScale }) {
  const counts = new Map<number, number>();
  for (const item of row.items) {
    const at = Date.parse(item.at);
    if (Number.isFinite(at)) counts.set(weekStart(at), (counts.get(weekStart(at)) ?? 0) + 1);
  }
  const max = Math.max(1, ...counts.values());
  // Weeks with no items draw nothing: an empty week is not a zero-shaded cell.
  return (
    <g role="group" aria-label={`${row.title}: ${row.items.length} items as weekly counts`}>
      {[...counts.entries()].map(([week, count]) => {
        const x0 = x(week);
        return (
          <HeatCell
            key={week}
            x={x0}
            y={y - 8}
            width={Math.max(3, x(week + WEEK) - x0)}
            height={22}
            value={measured(count)}
            step={seqStep(count, max)}
            text={`${count} in the week of ${formatDate(week, 'day')}`}
          />
        );
      })}
    </g>
  );
}

function Plot({ props, rows, width }: { props: EvidenceTimelineProps; rows: Row[]; width: number }) {
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
  const lanesBottom = rows.length * laneHeight;
  const contradictionY = lanesBottom + (props.contradictions.length ? CONTRADICTION_ROW / 2 : 0);
  const axisY = lanesBottom + (props.contradictions.length ? CONTRADICTION_ROW : 0);
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
  const highlighted = props.highlight ? props.items.find((item) => item.id === props.highlight) : undefined;
  const highlightRow = highlighted ? rows.findIndex((row) => !row.sub && row.items.some((item) => item.id === highlighted.id)) : -1;
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
        const top = r * laneHeight;
        const y = top + (labelAbove ? 14 : 0) + 14;
        const active = row.laneId !== undefined && row.laneId === props.activeLane;
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
            {row.items.some(seenByApp) && row.items.length <= DENSE_LANE && (
              <line x1={plotX + 10} x2={right} y1={y + 12.5} y2={y + 12.5} style={{ stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: '1 3' }} aria-hidden="true" />
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
            {row.state === 'failed' ? (
              <AbsentSlot absence={absent('not-recorded', `${row.title} could not be read`)} box={{ x: plotX + 10, y: y - 9, width: right - plotX - 10, height: 18 }} text="could not load" />
            ) : row.state === 'loading' ? (
              <text x={plotX + 14} y={y} dy="0.32em" className={MUTED_LABEL_CLASS}>loading…</text>
            ) : row.items.length > DENSE_LANE ? (
              <DenseLane row={row} y={y} x={x} />
            ) : (
              <LaneMarks row={row} y={y} x={x} onOpen={props.onOpen} tooltip={tooltip} />
            )}
            {row.cappedAt !== null && (
              <text x={right} y={top + (labelAbove ? 10 : 4)} dy={labelAbove ? undefined : '0.9em'} textAnchor="end" className={LABEL_CLASS}>{`first ${row.cappedAt} shown`}</text>
            )}
          </g>
        );
      })}
      {highlighted && highlightRow >= 0 && (
        <circle
          cx={x(Date.parse(highlighted.at))}
          cy={highlightRow * laneHeight + (labelAbove ? 14 : 0) + 14 + (seenByApp(highlighted) ? 12 : 0)}
          r={8}
          pointerEvents="none"
          style={{ fill: 'none', stroke: VIZ.focus, strokeWidth: 2 }}
        />
      )}
      {props.contradictions.length > 0 && (
        <g role="group" aria-label={`${props.contradictions.length} date contradictions`}>
          <text x={0} y={contradictionY} dy="0.32em" className="fill-fg-secondary text-[12px]">{labelAbove ? '' : 'Contradictions'}</text>
          {props.contradictions.map((entry, c) => {
            const readings = entry.readings.map((reading) => ({ reading, at: parseDate(reading.date) })).filter((r) => Number.isFinite(r.at)).sort((a, b) => a.at - b.at);
            if (readings.length === 0) return null;
            const cy = contradictionY + ((c % 2) * 2 - 1) * 4;
            const xs = readings.map((r) => x(r.at));
            const mid = (xs[0] + xs[xs.length - 1]) / 2;
            const name = `${entry.subject}: ${readings.map((r) => `${formatDate(r.at, 'day')} (${r.reading.basis})`).join(' ≠ ')}. Both are shown as recorded.`;
            return (
              <g key={entry.id} role="img" aria-label={name}>
                <title>{name}</title>
                {xs.length > 1 && <line x1={xs[0]} x2={xs[xs.length - 1]} y1={cy} y2={cy} style={{ stroke: VIZ.ink, strokeWidth: 2, strokeDasharray: '4 3' }} />}
                {xs.map((px, i) => <circle key={i} cx={px} cy={cy} r={3.5} style={{ fill: VIZ.surface, stroke: VIZ.ink, strokeWidth: 1.5 }} />)}
                {xs.length > 1 && (
                  <g>
                    <rect x={mid - 8} y={cy - 8} width={16} height={16} rx={4} style={{ fill: VIZ.surface, stroke: VIZ.ink, strokeWidth: 1 }} />
                    <text x={mid} y={cy} dy="0.35em" textAnchor="middle" className="fill-fg text-[12px] font-semibold">≠</text>
                  </g>
                )}
              </g>
            );
          })}
        </g>
      )}
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

/**
 * F13 EvidenceTimeline: one swimlane per source over a shared time axis. Date meaning is carried by
 * position and shape, never colour: an event date is a filled circle on the baseline, a due or
 * modelled date a hollow diamond, and a date this app saw something a ring on the dotted 'seen by
 * this app' sub-baseline. Findings are violet (leads the app surfaced); everything else is ink.
 * Contradicting readings are both drawn, joined by an ink dashed connector and '≠'. The brush and
 * the From/To inputs share one source of truth.
 */
export default function EvidenceTimeline(props: EvidenceTimelineProps) {
  const [expanded, setExpanded] = useState(false);
  const rows = rowsOf(props, expanded);
  const dated = props.items.filter((item) => Number.isFinite(Date.parse(item.at)));
  const labelAbove = (width: number) => width < 480;
  const height = (width: number) => rows.length * (LANE_HEIGHT + (labelAbove(width) ? 14 : 0)) + (props.contradictions.length ? CONTRADICTION_ROW : 0) + AXIS;
  const dense = rows.some((row) => row.items.length > DENSE_LANE);
  const legend: LegendItem[] = [
    { key: 'occurred', label: 'Event date', swatch: 'dot', tone: 'ink' },
    { key: 'due', label: 'Owed or modelled (◇)', swatch: 'hollow', tone: 'ink' },
    { key: 'seen', label: 'Seen by this app (dotted line)', swatch: 'ring', tone: 'ink' },
    { key: 'finding', label: 'Finding (lead)', swatch: 'dot', tone: 'lead' },
    ...(props.contradictions.length ? [{ key: 'contradiction', label: 'Contradicting dates (≠)', swatch: 'dashed-line' as const, tone: 'ink' as const }] : []),
    ...(dense ? [{ key: 'weekly', label: `Weekly count (lanes over ${DENSE_LANE} items)`, swatch: 'rect' as const, tone: 'mark' as const }] : []),
  ];
  const table: TwinTable = {
    caption: 'Timeline items in the overview',
    columns: [{ key: 'date', label: 'Date' }, { key: 'meaning', label: 'Date meaning' }, { key: 'source', label: 'Source' }, { key: 'title', label: 'Item' }],
    rows: [
      ...[...dated].sort((a, b) => Date.parse(a.at) - Date.parse(b.at)).map((item) => ({ date: formatDate(parseDate(item.at), 'day'), meaning: DATE_MEANING_LABELS[item.dateMeaning], source: SOURCE_LABELS[item.source], title: item.title })),
      ...props.contradictions.map((entry) => ({
        date: entry.readings.map((reading) => `${reading.date} (${reading.basis})`).join(' ≠ '),
        meaning: 'Contradicting dates, both shown as recorded',
        source: SOURCE_LABELS.CASE,
        title: entry.subject,
      })),
      ...props.lanes.filter((lane) => lane.cappedAt !== null).map((lane) => ({ date: absent('excluded', `only the first ${lane.cappedAt} ${SOURCE_LABELS[lane.source].toLowerCase()} were loaded`), meaning: null, source: SOURCE_LABELS[lane.source], title: null })),
      ...props.lanes.filter((lane) => lane.state === 'failed').map((lane) => ({ date: absent('not-recorded', `${SOURCE_LABELS[lane.source]} could not be read`), meaning: null, source: SOURCE_LABELS[lane.source], title: null })),
      ...(props.listed.unreadable ? [{ date: absent('excluded', `${props.listed.unreadable} items have a date that could not be read`), meaning: null, source: null, title: null }] : []),
    ],
  };
  const failed = props.lanes.filter((lane) => lane.state === 'failed').length;
  return (
    <ChartFrame
      title="Timeline overview"
      headline={timelineHeadline(props.listed, props.contradictions.length)}
      sources={{ tables: ['tracker notebook', 'findings', 'watch_changes', 'casework_cases', 'casework_events'] }}
      summary={`${dated.length} dated items in ${rows.length} lanes${failed ? `; ${failed} sources could not be read` : ''}${props.listed.unreadable ? `; ${props.listed.unreadable} undated items not placed` : ''}${props.lanes.filter((lane) => lane.cappedAt !== null).map((lane) => `; only the first ${lane.cappedAt} ${SOURCE_LABELS[lane.source].toLowerCase()} loaded`).join('')}. Drag across the lanes to set the From and To dates.`}
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
      {({ width }) => (dated.length || failed ? <Plot props={props} rows={rows} width={width} /> : null)}
    </ChartFrame>
  );
}
