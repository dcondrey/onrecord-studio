import React from 'react';
import { cn } from '../../utils/cn';

export function Table({ className, ...props }: React.HTMLAttributes<HTMLTableElement>) {
  return (
    <div className="w-full overflow-x-auto border border-slate-800/60 rounded-xl bg-slate-900/10 backdrop-blur-md">
      <table className={cn("w-full text-left border-collapse font-sans text-xs", className)} {...props} />
    </div>
  );
}

export function TableHeader({ className, ...props }: React.HTMLAttributes<HTMLTableSectionElement>) {
  return <thead className={cn("bg-slate-900/40 border-b border-slate-800/80 text-[10px] font-bold text-slate-400 uppercase tracking-wider select-none", className)} {...props} />;
}

export function TableBody({ className, ...props }: React.HTMLAttributes<HTMLTableSectionElement>) {
  return <tbody className={cn("divide-y divide-slate-800/40 bg-transparent", className)} {...props} />;
}

export function TableRow({ className, ...props }: React.HTMLAttributes<HTMLTableRowElement>) {
  return <tr className={cn("transition-colors hover:bg-slate-800/20 group", className)} {...props} />;
}

export function TableHead({ className, ...props }: React.ThHTMLAttributes<HTMLTableCellElement>) {
  return <th className={cn("px-4 py-2.5 font-bold text-left align-middle", className)} {...props} />;
}

export function TableCell({ className, ...props }: React.TdHTMLAttributes<HTMLTableCellElement>) {
  return <td className={cn("px-4 py-3 align-middle text-slate-300 font-medium whitespace-nowrap", className)} {...props} />;
}
