import { useRef } from 'react';
import type { StoryLead } from '../../../types';
import { COUNTED_FACTOR, factorLabel, isUnassessed, orderedFactors, points } from '../../../utils/attentionFactors';
import { paintFor, VIZ } from '../core/claim';
import { useSvgId } from '../frame/VizDefs';
import { useChartSize } from '../hooks/useChartSize';
import { hBarPath } from '../marks/shared';

export interface SparkFactor {
  key: string;
  label: string;
  /** Contribution in basis points; drawn as a segment only when it is a measured value. */
  basisPoints: number;
  /** A counted statutory or policy violation: the one factor that may be crimson. */
  counted: boolean;
  /** Never measured (an unassessed evidence strength): a dashed slot, never a zero. */
  unassessed: boolean;
}

const GAP = 1.5;
const SLOT = 8;

/** A lead's factors as the spark draws them, in the ranker's fixed order. */
export function sparkFactors(lead: StoryLead): SparkFactor[] {
  return orderedFactors(lead).map((factor) => ({
    key: factor.name,
    label: factorLabel(factor.name),
    basisPoints: factor.basisPoints,
    counted: factor.name === COUNTED_FACTOR,
    unassessed: isUnassessed(lead, factor.name),
  }));
}

/** A lead's attention score as words: what the spark draws, for a reader that cannot see it. */
export function scoreSentence(lead: StoryLead): string {
  const factors = orderedFactors(lead);
  const added = factors
    .filter((factor) => !isUnassessed(lead, factor.name) && factor.basisPoints > 0)
    .map((factor) => `${factor.name === COUNTED_FACTOR ? 'counted violation' : factorLabel(factor.name).toLowerCase()} +${points(factor.basisPoints)}`);
  const unassessed = factors.some((factor) => isUnassessed(lead, factor.name));
  return `Attention score ${points(lead.publicInterest.totalBasisPoints)}${added.length ? `: ${added.join(', ')}` : ''}.${unassessed ? ' Evidence strength not assessed.' : ''}`;
}

/**
 * A ranked lead's attention score at sparkline scale: one bar whose length is the score on a scale
 * shared by every row of the list, split into its factors in the ranker's fixed order. A factor the
 * finding's record lacks adds no segment; an unassessed factor is a dashed slot at the right end.
 * Each segment names its factor and points on hover. Without `width` the spark fills its line.
 */
export default function ScoreSpark({ factors, scaleMax, width, height = 6, label, decorative = false }: {
  factors: readonly SparkFactor[];
  /** The basis points the track stands for: the top score in the list. */
  scaleMax: number;
  width?: number;
  height?: number;
  /** The accessible name; unused when `decorative`, where the surrounding control states it. */
  label: string;
  decorative?: boolean;
}) {
  const box = useRef<HTMLSpanElement>(null);
  const measured = useChartSize(box, { fallbackWidth: 120 }).width;
  const drawnWidth = width ?? measured;
  const clip = useSvgId('score-spark');
  const unassessed = factors.some((factor) => factor.unassessed);
  // Every spark reserves the slot, so a row with an unassessed factor keeps the list's one scale.
  const track = Math.max(0, drawnWidth - SLOT - 4);
  const unit = track / Math.max(1, scaleMax);
  const drawn = factors.filter((factor) => !factor.unassessed && factor.basisPoints > 0);
  const lead = paintFor('descriptive', 'fill', 'lead').fill;
  const counted = paintFor('counted', 'fill').fill;
  const y = 1;
  const radius = height / 2;
  let cursor = 0;
  const segments = drawn.map((factor, index) => {
    const x0 = cursor;
    cursor = Math.min(track, cursor + factor.basisPoints * unit);
    const last = index === drawn.length - 1;
    return { factor, x0, x1: last ? cursor : Math.max(x0 + 0.5, cursor - GAP), last };
  });
  const svg = drawnWidth > 0 && (
    <svg
      role={decorative ? undefined : 'img'}
      aria-label={decorative ? undefined : label}
      aria-hidden={decorative || undefined}
      width={drawnWidth}
      height={height + 2}
      viewBox={`0 0 ${drawnWidth} ${height + 2}`}
      className="block shrink-0"
    >
      <clipPath id={clip}><rect x={0} y={y} width={track} height={height} rx={radius} /></clipPath>
      <rect x={0} y={y} width={track} height={height} rx={radius} style={{ fill: VIZ.track }} />
      <g clipPath={`url(#${clip})`}>
        {segments.map(({ factor, x0, x1, last }) => (
          <path
            key={factor.key}
            data-counted={factor.counted || undefined}
            d={last ? hBarPath(x0, x1, y, height, radius) : `M${x0},${y}h${x1 - x0}v${height}h${x0 - x1}Z`}
            style={{ fill: factor.counted ? counted : lead }}
          >
            <title>{`${factor.label}: +${points(factor.basisPoints)}`}</title>
          </path>
        ))}
      </g>
      {unassessed && (
        <rect
          data-absence="not-recorded"
          x={drawnWidth - SLOT + 0.5}
          y={y + 0.5}
          width={SLOT - 1}
          height={height - 1}
          rx={1.5}
          style={{ fill: 'none', stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: '2 1.5' }}
        >
          <title>Evidence strength: not assessed</title>
        </rect>
      )}
    </svg>
  );
  if (width !== undefined) return svg || null;
  return <span ref={box} className="block min-w-0 flex-1">{svg}</span>;
}
