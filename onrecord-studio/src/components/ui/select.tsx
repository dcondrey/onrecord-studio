import React from 'react';
import { cn } from '../../utils/cn';

export interface SelectProps extends React.SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
}

export const Select = React.forwardRef<HTMLSelectElement, SelectProps>(
  ({ className, label, children, ...props }, ref) => {
    return (
      <div className="flex flex-col space-y-1 select-none font-sans">
        {label && (
          <label className="text-label font-bold text-muted uppercase tracking-wider block px-0.5">
            {label}
          </label>
        )}
        <div className="relative">
          <select
            className={cn(
              "w-full bg-canvas text-fg-secondary text-label leading-4 border border-control rounded-lg pl-3 pr-8 py-2 appearance-none focus:outline-none focus:border-highlight font-medium shadow-sm transition-all cursor-pointer",
              className
            )}
            ref={ref}
            {...props}
          >
            {children}
          </select>
          {/* Native dropdown chevron mask indicator */}
          <div className="absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none text-muted">
            <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
            </svg>
          </div>
        </div>
      </div>
    );
  }
);
Select.displayName = "Select";
