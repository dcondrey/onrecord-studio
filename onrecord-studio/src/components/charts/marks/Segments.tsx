import { paintFor, VIZ, type MarkRole } from '../core/claim';
import { isValue, type Measured } from '../core/nulls';
import { AbsentSlot, MarkGroup, hBarPath, type MarkInteraction } from './shared';

export interface ScoreSegment {
  key: string;
  /** The most this factor can contribute; the segment's width. */
  max: number;
  value: Measured;
  label: string;
}

const GAP = 2;

/**
 * A score as segments laid end to end: each is as wide as its factor's maximum, a track filled to
 * the factor's score in the role colour (violet: a lead the app ranked). Segments are separated by
 * 2px surface gaps; an unassessed factor is its dashed slot, never an empty track read as zero.
 */
export function Segments({ x, y, height = 12, unit, segments, role = 'lead', interaction }: {
  /** Left edge, px. */
  x: number;
  y: number;
  height?: number;
  /** Pixels per score point, shared across rows so equal maxima are equal widths. */
  unit: number;
  segments: readonly ScoreSegment[];
  role?: MarkRole;
  interaction?: MarkInteraction;
}) {
  const fill = paintFor('descriptive', 'fill', role).fill;
  let cursor = x;
  const total = segments.reduce((sum, segment) => sum + segment.max * unit, 0) + GAP * Math.max(0, segments.length - 1);
  return (
    <MarkGroup interaction={interaction} box={{ x, y, width: total, height }}>
      {segments.map((segment) => {
        const left = cursor;
        const width = segment.max * unit;
        cursor += width + GAP;
        if (!isValue(segment.value)) {
          return <AbsentSlot key={segment.key} absence={segment.value} box={{ x: left, y, width, height }} text="" />;
        }
        const filled = Math.max(0, Math.min(segment.max, segment.value.value)) * unit;
        return (
          <g key={segment.key}>
            <title>{segment.label}</title>
            <rect x={left} y={y} width={width} height={height} rx={2} style={{ fill: VIZ.track }} />
            {filled > 0 && <path d={filled >= width ? `M${left},${y}h${width}v${height}h${-width}Z` : hBarPath(left, left + filled, y, height, 2)} style={{ fill }} />}
          </g>
        );
      })}
    </MarkGroup>
  );
}
