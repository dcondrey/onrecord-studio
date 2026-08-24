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
          "inline-flex items-center justify-center rounded-lg text-xs font-semibold font-sans tracking-tight transition-all focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-indigo-500 disabled:pointer-events-none disabled:opacity-50 active:scale-[0.98]",
          {
            "bg-indigo-600 border border-indigo-500 text-white hover:bg-indigo-500 shadow-sm shadow-indigo-600/10": variant === 'default',
            "bg-red-600/10 border border-red-500/20 text-red-400 hover:bg-red-600/20": variant === 'destructive',
            "border border-slate-800 bg-slate-900/40 text-slate-300 hover:bg-slate-800/60 hover:text-slate-100": variant === 'outline',
            "text-slate-400 hover:bg-slate-800/60 hover:text-slate-200": variant === 'ghost',
          },
          {
            "px-2.5 py-1.5 text-[11px]": size === 'sm',
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
