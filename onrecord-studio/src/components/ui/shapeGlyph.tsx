import type { SVGProps } from 'react';

export type GlyphShape = 'circle' | 'ring' | 'diamond' | 'triangle' | 'bar';

type ShapeGlyphProps = { shape: GlyphShape; cx: number; cy: number; radius: number } & Omit<SVGProps<SVGGeometryElement>, 'ref'>;

/**
 * Draws one shape from the vocabulary this app uses to carry state or severity without color
 * alone, per the App Store "Differentiate Without Color Alone" claim. `radius` is each shape's own
 * natural half-extent, not a shared nominal size: a caller balancing a family of shapes at one
 * visual weight (like `StatusDot`) passes a different radius per shape, the way the original
 * hand-drawn glyphs did.
 */
export function ShapeGlyph({ shape, cx, cy, radius, ...visual }: ShapeGlyphProps) {
  if (shape === 'circle') return <circle cx={cx} cy={cy} r={radius} {...visual} />;
  if (shape === 'ring') return <circle cx={cx} cy={cy} r={radius} fill="none" {...visual} />;
  if (shape === 'diamond') {
    return <polygon points={`${cx},${cy - radius} ${cx + radius},${cy} ${cx},${cy + radius} ${cx - radius},${cy}`} {...visual} />;
  }
  if (shape === 'triangle') {
    return <polygon points={`${cx},${cy - radius} ${cx + radius * 0.92},${cy + radius * 0.78} ${cx - radius * 0.92},${cy + radius * 0.78}`} {...visual} />;
  }
  return <rect x={cx - radius} y={cy - radius * 0.36} width={radius * 2} height={radius * 0.64} rx={radius * 0.24} {...visual} />;
}
