import { VIZ } from '../core/claim';
import { formatDate, formatFiscalYear, formatTick, type ValueKind } from '../core/format';
import { LABEL_FONT, measureText } from '../core/layout';
import { tickStep, type BandScale, type ContinuousScale, type TimeScale, type TimeTick } from '../core/scale';

/** Tick text: a text token at the caption-minus size, tabular figures. Never a series colour. */
export const TICK_CLASS = 'viz-num fill-muted text-[11px]';

interface AxisTick {
  at: number;
  label: string;
  title?: string;
}

type AxisSource =
  | { scale: ContinuousScale; kind: ValueKind; tickCount?: number }
  | { time: TimeScale; tickTarget?: number }
  | { band: BandScale<string>; format?: (key: string) => string };

export type AxisProps = AxisSource & {
  orient: 'bottom' | 'left';
  /** y of a bottom axis, x of a left axis. */
  offset: number;
  /** Draw the axis line itself (the baseline). */
  line?: boolean;
};

/** A few ticks: about one per 80px, never fewer than 2 or more than 10. */
export function defaultTickCount(length: number): number {
  return Math.max(2, Math.min(10, Math.floor(length / 80)));
}

function timeLabel(tick: TimeTick, fiscalYearStartMonth: number | undefined): string {
  switch (tick.unit) {
    case 'year':
    case 'fiscal-year':
      return formatDate(tick.at, tick.unit, fiscalYearStartMonth);
    case 'quarter':
    case 'month':
      if (tick.major) return fiscalYearStartMonth && fiscalYearStartMonth !== 1 ? formatDate(tick.at, 'fiscal-year', fiscalYearStartMonth) : String(new Date(tick.at).getUTCFullYear());
      return tick.unit === 'quarter' ? formatDate(tick.at, 'quarter').split(' ')[0] : formatDate(tick.at, 'month').split(' ')[0];
    case 'week':
    case 'day':
      return formatDate(tick.at, 'week');
    case 'hour':
      return tick.major ? formatDate(tick.at, 'week') : new Date(tick.at).getUTCHours().toString().padStart(2, '0') + ':00';
  }
}

function ticksOf(props: AxisProps, length: number): AxisTick[] {
  if ('scale' in props) {
    const values = props.scale.ticks(props.tickCount ?? defaultTickCount(length));
    const step = values.length > 1 ? values[1] - values[0] : tickStep(props.scale.domain[0], props.scale.domain[1], 5);
    return values.map((v) => ({ at: props.scale(v), label: formatTick(v, props.kind, step) }));
  }
  if ('time' in props) {
    const fy = props.time.fiscalYearStartMonth;
    return props.time.ticks(props.tickTarget ?? defaultTickCount(length)).map((tick) => ({
      at: props.time(tick.at),
      label: timeLabel(tick, fy),
      // A fiscal-year label's tooltip spells out its months: 'FY24' is 'Jul 2023 to Jun 2024'.
      title: tick.unit === 'fiscal-year' ? formatFiscalYear(tick.at, fy).range : undefined,
    }));
  }
  return props.band.domain.map((key) => ({ at: props.band(key) + props.band.bandwidth / 2, label: props.format ? props.format(key) : key, title: key }));
}

interface PlacedTick extends AxisTick {
  x: number;
  anchor: 'start' | 'middle' | 'end';
  left: number;
  right: number;
}

/** Minimum clear space between two bottom-axis labels. */
const LABEL_GAP = 6;

/**
 * Places each bottom label where it is drawn: one that would cross either end of the range is
 * anchored inward, which shifts its extent toward its neighbour.
 */
function place(ticks: AxisTick[], r0: number, r1: number, band: boolean): PlacedTick[] {
  return ticks.map((tick) => {
    const width = measureText(tick.label, LABEL_FONT);
    const anchor = band ? 'middle' : tick.at - width / 2 < r0 ? 'start' : tick.at + width / 2 > r1 ? 'end' : 'middle';
    const x = anchor === 'start' ? Math.max(r0, tick.at - 2) : anchor === 'end' ? Math.min(r1, tick.at + 2) : tick.at;
    const left = anchor === 'start' ? x : anchor === 'end' ? x - width : x - width / 2;
    return { ...tick, x, anchor, left, right: left + width };
  });
}

/**
 * Drops labels that would overlap a kept neighbour, measured at their drawn (anchored) extent so
 * an edge label pulled inward is never printed over the one beside it. The first and last labels
 * mark the axis's extent and are kept; interior labels give way to the last.
 */
function thinPlaced(ticks: PlacedTick[]): PlacedTick[] {
  const clear = (a: PlacedTick, b: PlacedTick) => b.left >= a.right + LABEL_GAP;
  const kept: PlacedTick[] = [];
  for (const tick of ticks) {
    const previous = kept[kept.length - 1];
    if (!previous || clear(previous, tick)) kept.push(tick);
  }
  const last = ticks[ticks.length - 1];
  if (last && kept[kept.length - 1] !== last) {
    while (kept.length > 1 && !clear(kept[kept.length - 1], last)) kept.pop();
    if (clear(kept[kept.length - 1], last)) kept.push(last);
  }
  return kept;
}

function thinVertical(ticks: AxisTick[]): AxisTick[] {
  return ticks.filter((tick, i) => i === 0 || Math.abs(tick.at - ticks[i - 1].at) >= 14);
}

/**
 * An axis: hairline ticks and muted labels at a few round values. Labels near either end of a
 * bottom axis are anchored inward so none is clipped at the plot edge.
 */
export function Axis(props: AxisProps) {
  const range = 'scale' in props ? props.scale.range : 'time' in props ? props.time.range : null;
  const [r0, r1] = range ? [Math.min(range[0], range[1]), Math.max(range[0], range[1])] : [0, 0];
  const length = r1 - r0;
  const horizontal = props.orient === 'bottom';
  const hairline = { stroke: VIZ.baseline, strokeWidth: 1, shapeRendering: 'crispEdges' as const };
  if (horizontal) {
    const ticks = thinPlaced(place(ticksOf(props, length), r0, r1, 'band' in props));
    return (
      <g aria-hidden="true">
        {props.line !== false && length > 0 && <line x1={r0} x2={r1} y1={props.offset + 0.5} y2={props.offset + 0.5} style={hairline} />}
        {ticks.map((tick) => (
          <g key={`${tick.at}-${tick.label}`}>
            {!('band' in props) && <line x1={Math.round(tick.at) + 0.5} x2={Math.round(tick.at) + 0.5} y1={props.offset} y2={props.offset + 4} style={hairline} />}
            <text x={tick.x} y={props.offset + 16} textAnchor={tick.anchor} className={TICK_CLASS}>
              {tick.title && tick.title !== tick.label && <title>{tick.title}</title>}
              {tick.label}
            </text>
          </g>
        ))}
      </g>
    );
  }
  const ticks = thinVertical(ticksOf(props, length));
  return (
    <g aria-hidden="true">
      {props.line && length > 0 && <line x1={props.offset + 0.5} x2={props.offset + 0.5} y1={r0} y2={r1} style={hairline} />}
      {ticks.map((tick) => (
        <text key={`${tick.at}-${tick.label}`} x={props.offset - 6} y={tick.at} dy="0.32em" textAnchor="end" className={TICK_CLASS}>
          {tick.title && tick.title !== tick.label && <title>{tick.title}</title>}
          {tick.label}
        </text>
      ))}
    </g>
  );
}
