import { paintFor, VIZ } from '../core/claim';
import { useSvgId } from '../frame/VizDefs';
import { hBarPath } from '../marks/shared';

export interface SparkFactor {
  key: string;
  /** Contribution in basis points; drawn as a segment only when it is a measured value. */
  basisPoints: number;
  /** A counted statutory or policy violation: the one factor that may be crimson. */
  counted: boolean;
  /** Never measured (an unassessed evidence strength): a dashed slot, never a zero. */
  unassessed: boolean;
}

const GAP = 1.5;
const SLOT = 8;

/**
 * A ranked lead's attention score at sparkline scale: one bar whose length is the score on a scale
 * shared by every row of the list, split into its factors in the ranker's fixed order. Factors the
 * finding's record lacks add no segment; an unassessed factor is a dashed slot at the right edge.
 * The accessible name carries every number.
 */
export default function ScoreSpark({ factors, scaleMax, width = 120, height = 6, label }: {
  factors: readonly SparkFactor[];
  /** The basis points the track stands for: the top score in the list. */
  scaleMax: number;
  width?: number;
  height?: number;
  label: string;
}) {
  const clip = useSvgId('score-spark');
  const track = width - SLOT - 4;
  const unit = track / Math.max(1, scaleMax);
  const drawn = factors.filter((factor) => !factor.unassessed && factor.basisPoints > 0);
  const unassessed = factors.some((factor) => factor.unassessed);
  const lead = paintFor('descriptive', 'fill', 'lead').fill;
  const counted = paintFor('counted', 'fill').fill;
  const y = 2;
  const radius = height / 2;
  let cursor = 0;
  const segments = drawn.map((factor, index) => {
    const x0 = cursor;
    cursor += factor.basisPoints * unit;
    const last = index === drawn.length - 1;
    return { factor, x0, x1: last ? cursor : Math.max(x0 + 0.5, cursor - GAP), last };
  });
  return (
    <svg role="img" aria-label={label} width={width} height={height + 4} viewBox={`0 0 ${width} ${height + 4}`} className="block shrink-0">
      <clipPath id={clip}><rect x={0} y={y} width={track} height={height} rx={radius} /></clipPath>
      <rect x={0} y={y} width={track} height={height} rx={radius} style={{ fill: VIZ.track }} />
      <g clipPath={`url(#${clip})`}>
        {segments.map(({ factor, x0, x1, last }) => (
          <path
            key={factor.key}
            d={last ? hBarPath(x0, x1, y, height, radius) : `M${x0},${y}h${x1 - x0}v${height}h${x0 - x1}Z`}
            style={{ fill: factor.counted ? counted : lead }}
          />
        ))}
      </g>
      {unassessed && (
        <rect
          data-absence="not-recorded"
          x={width - SLOT + 0.5}
          y={y + 0.5}
          width={SLOT - 1}
          height={height - 1}
          rx={1.5}
          style={{ fill: 'none', stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: '2 1.5' }}
        />
      )}
    </svg>
  );
}
