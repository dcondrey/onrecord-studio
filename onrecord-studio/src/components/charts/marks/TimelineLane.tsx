import { ShapeGlyph, type GlyphShape } from '../../ui/shapeGlyph';
import { paintFor, VIZ, type Claim, type MarkRole } from '../core/claim';
import { formatDate } from '../core/format';
import { MUTED_LABEL_CLASS, MarkGroup, type MarkInteraction } from './shared';

export interface LaneEvent {
  key: string;
  /** UTC ms. */
  at: number;
  shape?: GlyphShape;
  hollow?: boolean;
  role?: MarkRole;
  claim?: Claim;
  /** Full sentence for the mark's title and accessible name. */
  name: string;
  interaction?: MarkInteraction;
}

export interface LaneSpan {
  key: string;
  from: number;
  to: number;
  role?: MarkRole;
  name: string;
}

const DAY = 86_400_000;
const CLUSTER_PX = 4;
const CLUSTER_MIN = 6;
const STACK_STEP = 10;

/**
 * One swimlane: 6px spans, 8px event glyphs on the lane's centre line. Events on the same day
 * stack upward; more than five marks within 4px collapse into a count bubble.
 */
export function TimelineLane({ y, x, events, spans = [], label }: {
  /** Centre line of the lane, px. */
  y: number;
  x: (ms: number) => number;
  events: readonly LaneEvent[];
  spans?: readonly LaneSpan[];
  label?: string;
}) {
  const sorted = [...events].sort((a, b) => a.at - b.at || (a.key < b.key ? -1 : 1));
  const clusters: LaneEvent[][] = [];
  for (const event of sorted) {
    const current = clusters[clusters.length - 1];
    if (current && x(event.at) - x(current[0].at) <= CLUSTER_PX) current.push(event);
    else clusters.push([event]);
  }
  return (
    <g>
      {label && <text x={0} y={y} dy="0.32em" className={MUTED_LABEL_CLASS}>{label}</text>}
      {spans.map((span) => {
        const x0 = x(span.from);
        const width = Math.max(2, x(span.to) - x0);
        const role = span.role ?? 'comparison';
        // A comparison span is a solid context bar; paintFor would make a comparison mark hollow.
        const fill = role === 'comparison' ? VIZ.context : paintFor('descriptive', 'fill', role).fill;
        return (
          <rect key={span.key} x={x0} y={y - 3} width={width} height={6} rx={3} style={{ fill }}>
            <title>{span.name}</title>
          </rect>
        );
      })}
      {clusters.map((cluster) => {
        if (cluster.length >= CLUSTER_MIN) {
          const cx = (x(cluster[0].at) + x(cluster[cluster.length - 1].at)) / 2;
          const text = String(cluster.length);
          return (
            <g key={`cluster-${cluster[0].key}`}>
              <title>{`${cluster.length} events from ${formatDate(cluster[0].at, 'day')} to ${formatDate(cluster[cluster.length - 1].at, 'day')}`}</title>
              <circle cx={cx} cy={y} r={9} style={{ fill: VIZ.surface, stroke: VIZ.ink, strokeWidth: 1.5 }} />
              <text x={cx} y={y} dy="0.32em" textAnchor="middle" className="viz-num fill-fg text-[10px] font-medium">{text}</text>
            </g>
          );
        }
        const perDay = new Map<number, number>();
        return cluster.map((event) => {
          const day = Math.floor(event.at / DAY);
          const index = perDay.get(day) ?? 0;
          perDay.set(day, index + 1);
          const cx = x(event.at);
          const cy = y - index * STACK_STEP;
          const paint = paintFor(event.claim ?? 'descriptive', event.hollow ? 'hollow' : 'fill', event.role ?? 'primary');
          const open = event.hollow || event.shape === 'ring';
          return (
            <MarkGroup key={event.key} interaction={event.interaction} box={{ x: cx - 4, y: cy - 4, width: 8, height: 8 }} ringRadius={7}>
              <title>{event.name}</title>
              <ShapeGlyph shape={event.shape === 'ring' ? 'circle' : event.shape ?? 'circle'} cx={cx} cy={cy} radius={6} style={{ fill: VIZ.surface }} />
              <ShapeGlyph
                shape={event.shape ?? 'circle'}
                cx={cx}
                cy={cy}
                radius={open ? 3.5 : 4}
                style={open ? { fill: event.shape === 'ring' ? 'none' : VIZ.surface, stroke: paint.stroke, strokeWidth: 1.5 } : { fill: paint.fill }}
              />
            </MarkGroup>
          );
        });
      })}
    </g>
  );
}
