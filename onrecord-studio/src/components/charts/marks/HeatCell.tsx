import { ShapeGlyph, type GlyphShape } from '../../ui/shapeGlyph';
import { VIZ } from '../core/claim';
import { fitsInside } from '../core/layout';
import { ABSENCE_LABEL, isValue, type Measured } from '../core/nulls';
import { useHatchFill } from '../frame/VizDefs';
import { MarkGroup, type MarkInteraction } from './shared';

export type SeqStep = 1 | 2 | 3 | 4 | 5;

/** The sequential step for a measured value, 1..5 over (0, max]. A measured zero is the lightest step. */
export function seqStep(value: number, max: number): SeqStep {
  if (max <= 0 || value <= 0) return 1;
  return Math.min(5, Math.max(1, Math.ceil((value / max) * 5))) as SeqStep;
}

export interface HeatCellProps {
  x: number;
  y: number;
  width: number;
  height: number;
  value: Measured;
  /** Required for a measured value: the step of the single-hue slate ramp. */
  step?: SeqStep;
  /** Words or a number printed in the cell when they fit. */
  text?: string;
  /** A state glyph in the cell (StateGrid), so state never rests on fill alone. */
  glyph?: GlyphShape;
  interaction?: MarkInteraction;
}

/**
 * One cell of a heat strip, calendar or state grid. A measured value fills a step of the
 * sequential slate ramp (its lightest step is a small measured value, not absence). Absence is
 * never a ramp step: not recorded is an empty dashed cell, not checked an empty dotted cell, and
 * excluded a hatched one.
 */
export function HeatCell({ x, y, width, height, value, step = 1, text, glyph, interaction }: HeatCellProps) {
  const hatch = useHatchFill();
  const measuredValue = isValue(value);
  const inset = 1;
  const w = Math.max(0, width - 2 * inset);
  const h = Math.max(0, height - 2 * inset);
  const textColour = measuredValue ? `var(--viz-on-seq-${step})` : 'rgb(var(--muted))';
  const showText = text && fitsInside(text, glyph ? w - 12 : w) && h >= 14;
  return (
    <MarkGroup interaction={interaction} box={{ x: x + inset, y: y + inset, width: w, height: h }} ringRadius={4}>
      {/* Words that do not fit the cell stay in its title, so no cell's state is carried by fill alone. */}
      {(!measuredValue || (text && !showText)) && (
        <title>{[text && !showText ? text : null, measuredValue ? null : `${ABSENCE_LABEL[value.state]}: ${value.reason}`].filter(Boolean).join('. ')}</title>
      )}
      <rect
        x={x + inset + (measuredValue ? 0 : 0.5)}
        y={y + inset + (measuredValue ? 0 : 0.5)}
        width={measuredValue ? w : Math.max(0, w - 1)}
        height={measuredValue ? h : Math.max(0, h - 1)}
        rx={2}
        data-absence={measuredValue ? undefined : value.state}
        style={measuredValue
          ? { fill: `var(--viz-seq-${step})` }
          : {
            fill: value.state === 'excluded' ? hatch : 'none',
            stroke: VIZ.baseline,
            strokeWidth: 1,
            strokeDasharray: value.state === 'not-recorded' ? '3 2' : value.state === 'not-covered' ? '1 2' : undefined,
          }}
      />
      {glyph && h >= 10 && (
        <ShapeGlyph shape={glyph} cx={x + inset + (showText ? 8 : w / 2)} cy={y + inset + h / 2} radius={3.5} style={glyph === 'ring' ? { stroke: textColour, strokeWidth: 1.5 } : { fill: textColour }} />
      )}
      {showText && (
        <text x={x + inset + (glyph ? 16 : w / 2)} y={y + inset + h / 2} dy="0.32em" textAnchor={glyph ? 'start' : 'middle'} className="viz-num text-[11px]" style={{ fill: textColour }}>
          {text}
        </text>
      )}
    </MarkGroup>
  );
}
