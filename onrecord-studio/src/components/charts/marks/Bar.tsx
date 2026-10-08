import { paintFor, VIZ, type Claim, type MarkRole } from '../core/claim';
import { fitsInside, measureText } from '../core/layout';
import { isValue, type Measured } from '../core/nulls';
import type { ContinuousScale } from '../core/scale';
import { useFrameMotion } from '../hooks/useChartMotion';
import { useHatchFill } from '../frame/VizDefs';
import { AbsentSlot, LABEL_CLASS, MarkGroup, MUTED_LABEL_CLASS, hBarPath, vBarPath, type Box, type MarkInteraction } from './shared';

/** Where a bar sits across its value axis: its band position and thickness (≤ 24px from scaleBand). */
export interface BandPlacement {
  pos: number;
  thickness: number;
}

function baselineOf(scale: ContinuousScale): number {
  const [d0, d1] = scale.domain;
  return scale(Math.min(Math.max(0, Math.min(d0, d1)), Math.max(d0, d1)));
}

/** Text for a label set inside a fill: the surface colour clears every fill step of the ramps. */
const INSIDE_LABEL_STYLE = { fill: VIZ.surface };

export interface BarProps {
  orient?: 'horizontal' | 'vertical';
  band: BandPlacement;
  value: Measured;
  scale: ContinuousScale;
  claim?: Claim;
  role?: MarkRole;
  /** The value label at the bar's tip ('22 of 268'); omitted labels live in the tooltip and table. */
  label?: string;
  /** Span of the would-be footprint of an absent value, px. Default: baseline to the range end. */
  slotExtent?: readonly [number, number];
  interaction?: MarkInteraction;
  /** Stable key for the first-mount entrance. */
  motionKey?: string;
}

/**
 * One bar: ≤ 24px thick, square at the baseline, 4px rounded at the data end. A statistical claim
 * draws an outline bar. A measured zero draws a zero-length tick labelled '0'; an absent value
 * draws its slot, never a bar.
 */
export function Bar({ orient = 'horizontal', band, value, scale, claim = 'descriptive', role = 'default', label, slotExtent, interaction, motionKey }: BarProps) {
  const { enterOnce } = useFrameMotion();
  const horizontal = orient === 'horizontal';
  const base = baselineOf(scale);
  const [r0, r1] = [Math.min(...scale.range), Math.max(...scale.range)];
  const cross = band.pos;
  const t = band.thickness;
  const toBox = (a0: number, a1: number): Box => (horizontal
    ? { x: Math.min(a0, a1), y: cross, width: Math.abs(a1 - a0), height: t }
    : { x: cross, y: Math.min(a0, a1), width: t, height: Math.abs(a1 - a0) });

  if (!isValue(value)) {
    const [s0, s1] = slotExtent ?? (horizontal ? [base, r1] : [r0, base]);
    return <AbsentSlot absence={value} box={toBox(s0, s1)} />;
  }

  const end = scale(value.value);
  const zero = value.value === 0;
  const paint = paintFor(claim, claim === 'statistical' ? 'outline' : 'fill', role);
  const outline = paint.fill === 'none';
  const box = toBox(base, end);
  const enter = motionKey && !zero ? enterOnce(motionKey) : false;
  const text = zero ? '0' : label;
  const negative = value.value < 0;
  // Inset an outline by half its stroke so it occupies the same footprint as a filled bar.
  const inset = outline ? paint.strokeWidth / 2 : 0;
  const path = horizontal
    ? hBarPath(base, end + (negative ? inset : -inset), cross + inset, t - 2 * inset)
    : vBarPath(base, end + (negative ? -inset : inset), cross + inset, t - 2 * inset);

  let labelNode = null;
  if (text) {
    const width = measureText(text);
    const length = Math.abs(end - base);
    if (horizontal) {
      const outsideRoom = negative ? end - r0 : r1 - end;
      const outside = outsideRoom >= width + 6 || zero;
      const inside = !outside && !outline && fitsInside(text, length);
      if (outside || inside) {
        // Outside labels clear the 3px focus-ring margin plus its stroke.
        const x = outside ? (negative ? end - 6 : end + 6) : negative ? end + 4 : end - 4;
        const anchor = outside === negative ? 'end' : 'start';
        labelNode = (
          <text x={x} y={cross + t / 2} dy="0.32em" textAnchor={anchor} className={LABEL_CLASS} style={inside ? INSIDE_LABEL_STYLE : undefined}>
            {text}
          </text>
        );
      }
    } else if (length >= 0) {
      labelNode = (
        <text x={cross + t / 2} y={negative ? end + 12 : end - 4} textAnchor="middle" className={LABEL_CLASS}>{text}</text>
      );
    }
  }

  return (
    <MarkGroup interaction={interaction} box={box}>
      {zero ? (
        horizontal
          ? <line x1={base} x2={base} y1={cross} y2={cross + t} style={{ stroke: paint.fill === 'none' ? paint.stroke : paint.fill, strokeWidth: 1.5 }} />
          : <line x1={cross} x2={cross + t} y1={base} y2={base} style={{ stroke: paint.fill === 'none' ? paint.stroke : paint.fill, strokeWidth: 1.5 }} />
      ) : (
        <path
          d={path}
          className={enter ? (horizontal ? 'viz-enter-grow-x' : 'viz-enter-grow-y') : undefined}
          style={{ fill: paint.fill, stroke: paint.stroke, strokeWidth: paint.strokeWidth, transformOrigin: horizontal ? `${base}px 0` : `0 ${base}px` }}
        />
      )}
      {labelNode}
    </MarkGroup>
  );
}

export type SegmentTone = 'ord-1' | 'ord-2' | 'ord-3' | 'ord-4' | MarkRole;

export interface StackSegment {
  key: string;
  value: number;
  tone: SegmentTone;
  label: string;
}

function toneFill(tone: SegmentTone, claim: Claim): string {
  if (tone.startsWith('ord-')) return `var(--viz-${tone})`;
  return paintFor(claim, 'fill', tone as MarkRole).fill;
}

/**
 * Segments stacked from the baseline, positives outward and negatives the other way, each
 * separated by a 2px surface gap; only each side's outermost segment gets the rounded data end.
 * A segment's label is drawn inside only when it fits; otherwise the legend and tooltip carry it.
 */
export function StackedBar({ band, segments, scale, claim = 'descriptive', interaction, motionKey }: {
  band: BandPlacement;
  segments: readonly StackSegment[];
  scale: ContinuousScale;
  claim?: Claim;
  interaction?: MarkInteraction;
  motionKey?: string;
}) {
  const { enterOnce } = useFrameMotion();
  const base = baselineOf(scale);
  const enter = motionKey ? enterOnce(motionKey) : false;
  const placed: { segment: StackSegment; a0: number; a1: number; outer: boolean }[] = [];
  for (const sign of [1, -1]) {
    let running = 0;
    const side = segments.filter((segment) => (sign > 0 ? segment.value > 0 : segment.value < 0));
    side.forEach((segment, index) => {
      const from = running;
      running += segment.value;
      placed.push({ segment, a0: scale(from), a1: scale(running), outer: index === side.length - 1 });
    });
  }
  const extent = placed.length ? [Math.min(...placed.map((p) => Math.min(p.a0, p.a1))), Math.max(...placed.map((p) => Math.max(p.a0, p.a1)))] : [base, base];
  return (
    <MarkGroup interaction={interaction} box={{ x: extent[0], y: band.pos, width: extent[1] - extent[0], height: band.thickness }}>
      <g className={enter ? 'viz-enter-grow-x' : undefined} style={{ transformOrigin: `${base}px 0` }}>
        {placed.map(({ segment, a0, a1, outer }) => {
          const forward = a1 >= a0;
          // A 2px gap between neighbours: each inner segment gives up 2px at its far end.
          const end = outer ? a1 : forward ? a1 - 2 : a1 + 2;
          if (forward ? end <= a0 : end >= a0) return null;
          const d = outer ? hBarPath(a0, end, band.pos, band.thickness) : `M${Math.min(a0, end)},${band.pos}h${Math.abs(end - a0)}v${band.thickness}h${-Math.abs(end - a0)}Z`;
          const fill = toneFill(segment.tone, claim);
          const showLabel = fitsInside(segment.label, Math.abs(end - a0)) && band.thickness >= 14;
          return (
            <g key={segment.key}>
              <path d={d} style={{ fill }} />
              {showLabel && (
                <text x={(a0 + end) / 2} y={band.pos + band.thickness / 2} dy="0.32em" textAnchor="middle" className="viz-num text-[11px]" style={{ fill: segment.tone.startsWith('ord-') ? `var(--viz-on-${segment.tone})` : segment.tone === 'comparison' ? 'rgb(var(--fg))' : VIZ.surface }}>
                  {segment.label}
                </text>
              )}
            </g>
          );
        })}
      </g>
    </MarkGroup>
  );
}

/**
 * Real records outside the measure's denominator, drawn BESIDE the bar as a hatched stub with its
 * count, never inside it and never on the value axis.
 */
export function ExcludedStub({ band, at, count, label = 'outside this measure', width = 14 }: {
  band: BandPlacement;
  /** Left edge of the stub, px (after the bar and its label). */
  at: number;
  count: string;
  label?: string;
  width?: number;
}) {
  const hatch = useHatchFill();
  return (
    <g data-absence="excluded">
      <rect x={at + 0.5} y={band.pos + 0.5} width={width - 1} height={band.thickness - 1} rx={2} style={{ fill: hatch, stroke: VIZ.baseline, strokeWidth: 1 }} />
      <text x={at + width + 4} y={band.pos + band.thickness / 2} dy="0.32em" className={MUTED_LABEL_CLASS}>
        <tspan className="viz-num fill-fg-secondary">{count}</tspan>
        {` ${label}`}
      </text>
    </g>
  );
}
