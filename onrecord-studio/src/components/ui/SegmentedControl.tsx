import { useRef, type KeyboardEvent } from 'react';
import { cn } from '../../utils/cn';
import { CountBadge } from './Badge';

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  /** A measured count. Omit it when nothing was counted; it is never rendered as 0 by default. */
  count?: number;
}

interface SegmentedControlProps<T extends string> {
  /** Accessible name of the group. */
  label: string;
  value: T;
  options: readonly SegmentedOption<T>[];
  onChange: (value: T) => void;
  /** `tabs` switches the panel below (tablist); `filter` narrows the same list (radiogroup). */
  mode: 'tabs' | 'filter';
  size?: 'sm' | 'md';
  /** For `tabs`: the id of the panel each tab controls, by value. */
  panelId?: (value: T) => string;
  className?: string;
}

/**
 * One switcher for views and filters. Only the selected item is in the tab order; arrows, Home and
 * End move between items and select as they go, as both the tablist and radiogroup patterns expect.
 */
export default function SegmentedControl<T extends string>({
  label,
  value,
  options,
  onChange,
  mode,
  size = 'md',
  panelId,
  className,
}: SegmentedControlProps<T>) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const selectedIndex = Math.max(0, options.findIndex((option) => option.value === value));

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const last = options.length - 1;
    const next =
      event.key === 'ArrowRight' || event.key === 'ArrowDown' ? (selectedIndex === last ? 0 : selectedIndex + 1)
      : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? (selectedIndex === 0 ? last : selectedIndex - 1)
      : event.key === 'Home' ? 0
      : event.key === 'End' ? last
      : null;
    if (next === null) return;
    event.preventDefault();
    onChange(options[next].value);
    refs.current[next]?.focus();
  };

  return (
    <div
      role={mode === 'tabs' ? 'tablist' : 'radiogroup'}
      aria-label={label}
      onKeyDown={onKeyDown}
      className={cn('inline-flex items-center gap-0.5 rounded-lg bg-raised p-0.5', size === 'md' ? 'h-8' : 'h-7', className)}
    >
      {options.map((option, index) => {
        const selected = index === selectedIndex;
        return (
          <button
            key={option.value}
            ref={(node) => { refs.current[index] = node; }}
            type="button"
            role={mode === 'tabs' ? 'tab' : 'radio'}
            aria-selected={mode === 'tabs' ? selected : undefined}
            aria-checked={mode === 'filter' ? selected : undefined}
            aria-controls={mode === 'tabs' && panelId ? panelId(option.value) : undefined}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(option.value)}
            className={cn(
              'inline-flex h-full items-center gap-1.5 whitespace-nowrap rounded-md px-3 text-control transition-colors',
              selected ? 'bg-canvas text-fg shadow-panel' : 'text-fg-secondary hover:text-fg',
            )}
          >
            {option.label}
            {option.count !== undefined && <CountBadge value={option.count} />}
          </button>
        );
      })}
    </div>
  );
}
