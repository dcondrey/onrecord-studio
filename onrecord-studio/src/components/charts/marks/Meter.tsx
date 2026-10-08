import { cn } from '../../../utils/cn';

export interface MeterSegment {
  key: string;
  value: number;
  /** Printed beside its swatch: the value as text ('120 read to 20,000 characters'). */
  text: string;
  /** Ordinal ramp step 1 (lightest) to 4. */
  step: 1 | 2 | 3 | 4;
}

const STEP_FILL = ['var(--viz-ord-1)', 'var(--viz-ord-2)', 'var(--viz-ord-3)', 'var(--viz-ord-4)'] as const;

/**
 * A part-to-whole meter: ordinal-ramp segments separated by 2px gaps on a track, role=meter with a
 * value text, and every segment's value also printed as text below. Nothing is shown by length
 * alone.
 */
export function Meter({ label, segments, max, valueText, className }: {
  label: string;
  segments: readonly MeterSegment[];
  max: number;
  /** The whole reading as a sentence, for aria-valuetext. */
  valueText: string;
  className?: string;
}) {
  const total = segments.reduce((sum, segment) => sum + segment.value, 0);
  const visible = segments.filter((segment) => segment.value > 0);
  return (
    <div className={cn('grid gap-2', className)}>
      <div
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={max}
        aria-valuenow={Math.min(total, max)}
        aria-valuetext={valueText}
        className="flex h-2.5 w-full gap-0.5 overflow-hidden rounded-full"
        style={{ background: 'var(--viz-track)' }}
      >
        {visible.map((segment) => (
          <span key={segment.key} className="h-full first:rounded-l-full" style={{ width: `${max > 0 ? (segment.value / max) * 100 : 0}%`, background: STEP_FILL[segment.step - 1] }} />
        ))}
      </div>
      <ul className="flex flex-wrap gap-x-4 gap-y-1 text-caption text-fg-secondary">
        {segments.map((segment) => (
          <li key={segment.key} className="flex items-center gap-1.5">
            <span aria-hidden="true" className="h-2.5 w-3 shrink-0 rounded-sm" style={{ background: STEP_FILL[segment.step - 1] }} />
            <span>{segment.text}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
