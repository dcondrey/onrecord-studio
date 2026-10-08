import { paintFor, type Claim, type MarkRole } from '../core/claim';
import { isValue, type Measured } from '../core/nulls';
import { LABEL_CLASS, MUTED_LABEL_CLASS, MarkGroup, type MarkInteraction } from './shared';

/** How far an open end extends past its known end before the chevron: a stub, never to an epoch. */
const OPEN_STUB = 28;
const CHEVRON = 6;

export interface IntervalProps {
  /** Centre line of the span, px. */
  y: number;
  x: (v: number) => number;
  /** Plot bounds the stub and its words must stay within, px. */
  bounds: readonly [number, number];
  start: Measured;
  end: Measured;
  claim?: Claim;
  role?: MarkRole;
  /** 6px by default. */
  thickness?: number;
  startUnknownLabel?: string;
  openEndLabel?: string;
  interaction?: MarkInteraction;
}

/**
 * A dated range. A missing start or end is an open end: a short stub with a 6px chevron and words
 * ('start unknown', 'open ended'), never a bar back to an epoch or forward to today.
 */
export function Interval({ y, x, bounds, start, end, claim = 'descriptive', role = 'default', thickness = 6, startUnknownLabel = 'start unknown', openEndLabel = 'open ended', interaction }: IntervalProps) {
  const paint = paintFor(claim, claim === 'statistical' ? 'outline' : 'fill', role);
  const colour = paint.fill === 'none' ? paint.stroke : paint.fill;
  const [b0, b1] = bounds;
  if (!isValue(start) && !isValue(end)) {
    return <text x={b0} y={y} dy="0.32em" className={MUTED_LABEL_CLASS}>Dates not recorded</text>;
  }
  const x0 = isValue(start) ? x(start.value) : Math.max(b0 + CHEVRON, x((end as { value: number }).value) - OPEN_STUB);
  const x1 = isValue(end) ? x(end.value) : Math.min(b1 - CHEVRON, x((start as { value: number }).value) + OPEN_STUB);
  const top = y - thickness / 2;
  const chevron = (tip: number, direction: 1 | -1) => (
    <path
      d={`M${tip - direction * CHEVRON},${y - CHEVRON / 2 - 1}L${tip},${y}L${tip - direction * CHEVRON},${y + CHEVRON / 2 + 1}`}
      style={{ fill: 'none', stroke: colour, strokeWidth: 1.5, strokeLinecap: 'round', strokeLinejoin: 'round' }}
    />
  );
  return (
    <MarkGroup interaction={interaction} box={{ x: x0, y: top, width: Math.max(1, x1 - x0), height: thickness }}>
      <rect
        x={x0}
        y={top}
        width={Math.max(1, x1 - x0)}
        height={thickness}
        rx={thickness / 2}
        style={paint.fill === 'none' ? { fill: 'none', stroke: paint.stroke, strokeWidth: paint.strokeWidth } : { fill: paint.fill }}
      />
      {!isValue(start) && (
        <>
          {chevron(x0 - 2, -1)}
          <text x={x0 - CHEVRON - 6} y={y} dy="0.32em" textAnchor="end" className={LABEL_CLASS}>{startUnknownLabel}</text>
        </>
      )}
      {!isValue(end) && (
        <>
          {chevron(x1 + 2 + CHEVRON, 1)}
          <text x={x1 + CHEVRON + 8} y={y} dy="0.32em" className={LABEL_CLASS}>{openEndLabel}</text>
        </>
      )}
    </MarkGroup>
  );
}
