import { cn } from '../../utils/cn';
import { ShapeGlyph } from './shapeGlyph';

export type StatusDotState = 'active' | 'idle' | 'error' | 'note';

const SHAPE: Record<StatusDotState, { path: 'circle' | 'ring' | 'diamond' | 'bar'; radius: number; className: string }> = {
  active: { path: 'circle', radius: 4.5, className: 'text-emerald-400' },
  idle: { path: 'ring', radius: 3.8, className: 'text-slate-400' },
  error: { path: 'diamond', radius: 5, className: 'text-rose-400' },
  note: { path: 'bar', radius: 5, className: 'text-cyan-400' },
};

/**
 * A status indicator whose shape, not only its color, carries the state: a filled circle for
 * active, a hollow ring for idle, a diamond for an error, a bar for a note. A reader who cannot
 * separate emerald from rose still reads the state, which is what the App Store "Differentiate
 * Without Color Alone" claim requires.
 *
 * `label` is the accessible name. Passing an empty string marks the dot decorative, which is
 * correct only where the same state is already stated in adjacent text.
 */
export default function StatusDot({ state, label, className }: { state: StatusDotState; label: string; className?: string }) {
  const { path, radius, className: tone } = SHAPE[state];
  return (
    <svg
      viewBox="-6 -6 12 12"
      className={cn('h-2 w-2 shrink-0', tone, className)}
      role={label ? 'img' : undefined}
      aria-label={label || undefined}
      aria-hidden={label ? undefined : true}
    >
      <ShapeGlyph
        shape={path}
        cx={0}
        cy={0}
        radius={radius}
        fill={path === 'ring' ? 'none' : 'currentColor'}
        stroke={path === 'ring' ? 'currentColor' : undefined}
        strokeWidth={path === 'ring' ? 1.8 : undefined}
      />
    </svg>
  );
}
