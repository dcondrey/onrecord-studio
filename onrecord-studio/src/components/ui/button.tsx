import React from 'react';
import { cn } from '../../utils/cn';

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'default' | 'destructive' | 'outline' | 'ghost';
  size?: 'sm' | 'default' | 'lg';
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = 'default', size = 'default', ...props }, ref) => {
    return (
      <button
        className={cn(
          "inline-flex items-center justify-center rounded-lg text-label leading-4 font-semibold font-sans tracking-tight transition-all focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-highlight disabled:pointer-events-none disabled:opacity-50 active:scale-[0.98]",
          {
            "bg-highlight border border-highlight text-white hover:bg-highlight shadow-sm shadow-highlight/30": variant === 'default',
            "bg-danger/10 border border-danger/30 text-danger hover:bg-danger/20": variant === 'destructive',
            "border border-control bg-surface/40 text-fg-secondary hover:bg-raised/70 hover:text-fg": variant === 'outline',
            "text-muted hover:bg-raised/70 hover:text-fg": variant === 'ghost',
          },
          {
            "px-2.5 py-1.5 text-label": size === 'sm',
            "px-4 py-2": size === 'default',
            "px-5 py-2.5 text-sm": size === 'lg',
          },
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Button.displayName = "Button";
