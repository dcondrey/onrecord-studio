import type { FindingSeverity } from '../../types';
import { cn } from '../../utils/cn';
import { ShapeGlyph, type GlyphShape } from '../ui/shapeGlyph';

const GLYPHS: Record<FindingSeverity, { shape: GlyphShape; tone: string }> = {
  CRITICAL: { shape: 'diamond', tone: 'text-danger' },
  SERIOUS: { shape: 'triangle', tone: 'text-warning' },
  NOTABLE: { shape: 'circle', tone: 'text-accent' },
  INFO: { shape: 'ring', tone: 'text-muted' },
};

/** Severity as shape and colour together, so it never rests on colour alone. */
export default function SeverityGlyph({ severity, className }: { severity: FindingSeverity; className?: string }) {
  const { shape, tone } = GLYPHS[severity];
  // A ring is an outline; filling it would draw the same disc as NOTABLE.
  const paint = shape === 'ring' ? { fill: 'none', stroke: 'currentColor', strokeWidth: 1.5 } : { fill: 'currentColor' };
  return (
    <span aria-hidden="true" className={cn('grid shrink-0 place-items-center', tone, className)}>
      <svg viewBox="-6 -6 12 12" className="h-3 w-3">
        <ShapeGlyph shape={shape} cx={0} cy={0} radius={4.5} {...paint} />
      </svg>
    </span>
  );
}
