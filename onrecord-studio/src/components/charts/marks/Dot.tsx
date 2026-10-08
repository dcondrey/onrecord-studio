import { ShapeGlyph, type GlyphShape } from '../../ui/shapeGlyph';
import { paintFor, VIZ, type Claim, type MarkRole } from '../core/claim';
import { beeswarm } from '../core/layout';
import type { Measured } from '../core/nulls';
import type { ContinuousScale } from '../core/scale';
import { LABEL_CLASS, MarkGroup, type MarkInteraction } from './shared';

export interface DotProps {
  cx: number;
  cy: number;
  /** When given and absent, nothing is drawn: the row's text says 'not measured'. */
  value?: Measured;
  shape?: GlyphShape;
  claim?: Claim;
  role?: MarkRole;
  /** Hollow regardless of role: a small sample, an incomplete measure, a prior period. */
  hollow?: boolean;
  /** Radius; 4 is the 8px default. */
  r?: number;
  /** Direct label beside the dot. */
  label?: string;
  labelSide?: 'start' | 'end' | 'above';
  /** One <title> per mark, so VoiceOver keeps the label. */
  title?: string;
  interaction?: MarkInteraction;
}

/**
 * One dot from the ShapeGlyph vocabulary, 8px with a 2px surface ring so it stays legible where it
 * crosses a line or another dot. A statistical claim is always hollow ink.
 */
export function Dot({ cx, cy, value, shape = 'circle', claim = 'descriptive', role = 'default', hollow = false, r = 4, label, labelSide = 'end', title, interaction }: DotProps) {
  if (value && value.state !== 'value') return null;
  const paint = paintFor(claim, hollow || shape === 'ring' ? 'hollow' : 'fill', role);
  const open = paint.fill === 'none' || paint.fill === VIZ.surface;
  return (
    <MarkGroup interaction={interaction} box={{ x: cx - r, y: cy - r, width: 2 * r, height: 2 * r }} ringRadius={r + 3}>
      {title && <title>{title}</title>}
      <ShapeGlyph shape={shape === 'ring' ? 'circle' : shape} cx={cx} cy={cy} radius={r + 2} style={{ fill: VIZ.surface }} />
      <ShapeGlyph
        shape={shape}
        cx={cx}
        cy={cy}
        radius={open ? r - 0.75 : r}
        style={open ? { fill: shape === 'ring' ? 'none' : VIZ.surface, stroke: paint.stroke, strokeWidth: 1.5 } : { fill: paint.fill }}
      />
      {label && (
        <text
          x={labelSide === 'end' ? cx + r + 5 : labelSide === 'start' ? cx - r - 5 : cx}
          y={labelSide === 'above' ? cy - r - 6 : cy}
          dy={labelSide === 'above' ? undefined : '0.32em'}
          textAnchor={labelSide === 'end' ? 'start' : labelSide === 'start' ? 'end' : 'middle'}
          className={LABEL_CLASS}
        >
          {label}
        </text>
      )}
    </MarkGroup>
  );
}

export interface SwarmItem {
  key: string;
  value: number;
  shape?: GlyphShape;
  role?: MarkRole;
  hollow?: boolean;
  title?: string;
  interaction?: MarkInteraction;
}

/**
 * A static, deterministic beeswarm on one value axis (layout.beeswarm): the same data always lands
 * in the same place, so there is nothing to simulate or animate.
 */
export function Beeswarm({ items, x, cy, r = 4, maxRows = 3, claim = 'descriptive' }: {
  items: readonly SwarmItem[];
  x: ContinuousScale;
  cy: number;
  r?: number;
  maxRows?: number;
  claim?: Claim;
}) {
  const layout = beeswarm(items, x, r, maxRows);
  const byKey = new Map(items.map((item) => [item.key, item] as const));
  return (
    <g>
      {layout.map((point) => {
        const item = byKey.get(point.key);
        if (!item) return null;
        return (
          <Dot
            key={point.key}
            cx={point.cx}
            cy={cy + point.dy}
            r={r}
            shape={item.shape}
            claim={claim}
            role={item.role}
            hollow={item.hollow}
            title={item.title}
            interaction={item.interaction}
          />
        );
      })}
    </g>
  );
}
