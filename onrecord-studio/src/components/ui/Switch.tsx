import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { cn } from '../../utils/cn';

interface SwitchProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'onChange' | 'role' | 'aria-checked' | 'type'> {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  /** Visible label, after the track. Without it the caller names the switch (`aria-label` or `aria-labelledby`). */
  children?: ReactNode;
}

/**
 * The one on/off control. The off track is `control` (3.4:1 or more against every surface it sits
 * on, WCAG 1.4.11), so an off switch is still visibly a control; on is the accent.
 */
export default function Switch({ checked, onCheckedChange, children, className, ...props }: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onCheckedChange(!checked)}
      className={cn('focus-ring inline-flex shrink-0 items-center gap-2 rounded-full text-control text-fg-secondary disabled:opacity-50', className)}
      {...props}
    >
      <span aria-hidden="true" className={cn('relative h-5 w-9 shrink-0 rounded-full transition-colors duration-fast', checked ? 'bg-accent' : 'bg-control')}>
        <span className={cn('absolute left-0.5 top-0.5 h-4 w-4 rounded-full bg-white shadow-sm transition-transform duration-fast', checked && 'translate-x-4')} />
      </span>
      {children}
    </button>
  );
}
