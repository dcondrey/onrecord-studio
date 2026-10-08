import { forwardRef, type ButtonHTMLAttributes } from 'react';
import { cn } from '../../utils/cn';
import Icon, { type IconName } from './icons';

/** `destructive` is the solid confirm inside a dialog that deletes; `danger` is the outline trigger. */
export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'destructive' | 'link';
export type ButtonSize = 'sm' | 'md' | 'lg';

const VARIANT: Record<ButtonVariant, string> = {
  // A disabled fill drops to the neutral surface, not to a faded accent: in dark a half-opacity
  // cyan still reads as live.
  primary: 'bg-accent text-on-accent hover:bg-accent/90 active:bg-accent/80 disabled:bg-raised disabled:text-muted disabled:opacity-100',
  secondary: 'border border-line-strong bg-canvas/70 text-fg hover:bg-raised',
  ghost: 'text-fg-secondary hover:bg-raised hover:text-fg',
  danger: 'border border-danger/30 text-danger hover:bg-danger/10',
  destructive: 'bg-danger text-on-danger hover:bg-danger/90 active:bg-danger/80 disabled:bg-raised disabled:text-muted disabled:opacity-100',
  link: 'text-accent underline-offset-2 hover:underline',
};

const SIZE: Record<ButtonSize, { text: string; square: string }> = {
  sm: { text: 'h-7 px-3 rounded-md', square: 'h-7 w-7 rounded-md' },
  md: { text: 'h-8 px-3 rounded-lg', square: 'h-8 w-8 rounded-lg' },
  lg: { text: 'h-10 px-4 rounded-lg', square: 'h-10 w-10 rounded-lg' },
};

export interface ButtonClassOptions {
  variant?: ButtonVariant;
  size?: ButtonSize;
  iconOnly?: boolean;
  className?: string;
}

/** The Button recipe as a class string, for a `<label>` or `<a>` that must look like a button. */
export function buttonClass({ variant = 'secondary', size = 'md', iconOnly = false, className }: ButtonClassOptions = {}) {
  return cn(
    'motion-press inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap text-control transition-colors disabled:pointer-events-none disabled:opacity-50 aria-disabled:pointer-events-none aria-disabled:opacity-50',
    VARIANT[variant],
    variant === 'link' ? 'rounded' : iconOnly ? SIZE[size].square : SIZE[size].text,
    className,
  );
}

type NativeButtonProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'className'> & { className?: string };

interface CommonProps extends NativeButtonProps {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Leading icon. */
  icon?: IconName;
}

/** An icon-only button has no visible text, so its accessible name is required. */
export type ButtonProps =
  | (CommonProps & { iconOnly?: false })
  | (CommonProps & { iconOnly: true; icon: IconName; 'aria-label': string });

/** At most one `primary` per surface (a panel or a dialog); everything else is secondary or ghost. */
const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = 'secondary', size = 'md', icon, iconOnly = false, className, type = 'button', children, ...props },
  ref,
) {
  return (
    <button ref={ref} type={type} className={buttonClass({ variant, size, iconOnly, className })} {...props}>
      {icon && <Icon name={icon} size={size === 'sm' ? 'sm' : 'md'} />}
      {!iconOnly && children}
    </button>
  );
});

export default Button;
