import type { ReactNode, TdHTMLAttributes, ThHTMLAttributes } from 'react';
import { cn } from '../../utils/cn';

/** A wide table scrolls inside its panel, never the page. */
export default function Table({ label, className, children }: { label?: string; className?: string; children: ReactNode }) {
  return (
    <div className={cn('overflow-x-auto', className)}>
      <table aria-label={label} className="w-full border-collapse text-body">{children}</table>
    </div>
  );
}

/**
 * A header cell. A long heading takes a short `children` label and the full wording in `full`,
 * which becomes the tooltip and the accessible abbreviation expansion.
 */
export function Th({ numeric = false, full, className, children, ...props }: {
  numeric?: boolean;
  full?: string;
  className?: string;
  children: ReactNode;
} & Omit<ThHTMLAttributes<HTMLTableCellElement>, 'className'>) {
  return (
    <th
      scope="col"
      className={cn(
        'sticky top-0 z-sticky h-8 whitespace-nowrap border-b border-line bg-panel px-3 text-overline uppercase text-muted',
        numeric ? 'text-right' : 'text-left',
        className,
      )}
      {...props}
    >
      {full ? <abbr title={full} className="no-underline">{children}</abbr> : children}
    </th>
  );
}

/** A body cell. `numeric` right-aligns tabular figures; `mono` is for identifiers and digests only. */
export function Td({ numeric = false, mono = false, entity = false, className, children, ...props }: {
  numeric?: boolean;
  mono?: boolean;
  /** The row's primary entity name: wraps to two lines, never truncated to one. */
  entity?: boolean;
  className?: string;
  children: ReactNode;
} & Omit<TdHTMLAttributes<HTMLTableCellElement>, 'className'>) {
  return (
    <td
      className={cn(
        'h-10 border-b border-line px-3 py-2 align-middle text-fg',
        numeric && 'whitespace-nowrap text-right tabular-nums',
        mono && 'font-mono text-code',
        entity && 'min-w-[12rem]',
        className,
      )}
      {...props}
    >
      {entity ? <span className="line-clamp-2">{children}</span> : children}
    </td>
  );
}
