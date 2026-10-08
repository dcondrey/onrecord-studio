import { REFERENCE_PAINT, VIZ, assertYardstickLabel, type YardstickLabel } from '../core/claim';

interface Placement {
  /** Position on the value axis, px. */
  at: number;
  /** Cross-axis span the line covers, px. */
  extent: readonly [number, number];
  /** 'x': a vertical line at x = at. 'y': a horizontal line at y = at. */
  axis: 'x' | 'y';
  /** Label side for a vertical line. Default end (right of the line). */
  labelSide?: 'start' | 'end';
}

export type ReferenceLineProps = Placement & (
  /** A line the statute engine evaluated; the label carries the engine's citation. */
  | { kind: 'statute'; label: string; citation: string }
  /** An app yardstick. Its label must say 'reference' or 'not a … rule'. */
  | { kind: 'yardstick'; label: YardstickLabel }
  | { kind: 'peer-mean'; label: string }
  | { kind: 'today'; label?: string }
);

/** A reference line. None of its kinds is ever crimson: a reference is not a finding. */
export function ReferenceLine(props: ReferenceLineProps) {
  if (props.kind === 'yardstick') assertYardstickLabel(props.label);
  if (props.kind === 'statute' && !props.citation.trim()) throw new Error('A statute reference line needs the engine\'s citation.');
  const paint = REFERENCE_PAINT[props.kind];
  const label = props.kind === 'statute' ? `${props.label} (${props.citation})` : props.kind === 'today' ? props.label ?? 'Today' : props.label;
  const at = Math.round(props.at) + 0.5;
  const style = { stroke: paint.stroke, strokeWidth: paint.strokeWidth, strokeDasharray: paint.dash, shapeRendering: 'crispEdges' as const };
  const [c0, c1] = props.extent;
  if (props.axis === 'x') {
    const start = props.labelSide === 'start';
    return (
      <g data-reference={props.kind}>
        <line x1={at} x2={at} y1={c0} y2={c1} style={style} />
        <text x={at + (start ? -4 : 4)} y={c0 + 10} textAnchor={start ? 'end' : 'start'} className="fill-muted text-[11px]">{label}</text>
      </g>
    );
  }
  return (
    <g data-reference={props.kind}>
      <line x1={c0} x2={c1} y1={at} y2={at} style={style} />
      <text x={c1} y={at - 4} textAnchor="end" className="fill-muted text-[11px]">{label}</text>
    </g>
  );
}

/** A shaded range on the value axis (a typical range, a time window), labelled; it recedes. */
export function ReferenceBand({ from, to, extent, axis, label }: {
  from: number;
  to: number;
  extent: readonly [number, number];
  axis: 'x' | 'y';
  label: string;
}) {
  const [lo, hi] = from <= to ? [from, to] : [to, from];
  const [c0, c1] = extent;
  return (
    <g data-reference="band">
      {axis === 'x'
        ? <rect x={lo} y={c0} width={hi - lo} height={c1 - c0} style={{ fill: VIZ.track, opacity: 0.6 }} />
        : <rect x={c0} y={lo} width={c1 - c0} height={hi - lo} style={{ fill: VIZ.track, opacity: 0.6 }} />}
      <text x={axis === 'x' ? lo + 4 : c0 + 4} y={axis === 'x' ? c1 - 4 : lo + 12} className="fill-muted text-[11px]">{label}</text>
    </g>
  );
}
