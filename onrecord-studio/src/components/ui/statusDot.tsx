import { cn } from '../../utils/cn';

export type StatusDotState = 'active' | 'idle' | 'error' | 'note';

const SHAPE: Record<StatusDotState, { path: 'circle' | 'ring' | 'diamond' | 'bar'; className: string }> = {
  active: { path: 'circle', className: 'text-emerald-400' },
  idle: { path: 'ring', className: 'text-slate-400' },
  error: { path: 'diamond', className: 'text-rose-400' },
  note: { path: 'bar', className: 'text-cyan-400' },
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
  const { path, className: tone } = SHAPE[state];
  return (
    <svg
      viewBox="-6 -6 12 12"
      className={cn('h-2 w-2 shrink-0', tone, className)}
      role={label ? 'img' : undefined}
      aria-label={label || undefined}
      aria-hidden={label ? undefined : true}
    >
      {path === 'circle' && <circle cx="0" cy="0" r="4.5" fill="currentColor" />}
      {path === 'ring' && <circle cx="0" cy="0" r="3.8" fill="none" stroke="currentColor" strokeWidth="1.8" />}
      {path === 'diamond' && <polygon points="0,-5 5,0 0,5 -5,0" fill="currentColor" />}
      {path === 'bar' && <rect x="-5" y="-1.6" width="10" height="3.2" rx="1.2" fill="currentColor" />}
    </svg>
  );
}
