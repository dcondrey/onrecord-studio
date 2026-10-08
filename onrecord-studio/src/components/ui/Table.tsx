import { useEffect, useRef, useState, type ReactNode, type TdHTMLAttributes, type ThHTMLAttributes } from 'react';
import { cn } from '../../utils/cn';

/**
 * A wide table scrolls inside its panel, never the page. While columns remain past the right
 * edge, a fade there says so; a clipped column with no cue reads as the whole table.
 */
export default function Table({ label, className, children }: { label?: string; className?: string; children: ReactNode }) {
  const scroller = useRef<HTMLDivElement>(null);
  const [moreRight, setMoreRight] = useState(false);
  const [overflows, setOverflows] = useState(false);
  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const measure = () => {
      setMoreRight(element.scrollLeft + element.clientWidth < element.scrollWidth - 1);
      setOverflows(element.clientWidth < element.scrollWidth - 1);
    };
    measure();
    element.addEventListener('scroll', measure, { passive: true });
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    observer?.observe(element);
    if (element.firstElementChild) observer?.observe(element.firstElementChild);
    return () => {
      element.removeEventListener('scroll', measure);
      observer?.disconnect();
    };
  }, []);
  return (
    <div className={cn('relative', className)}>
      {/* Only a table that scrolls is a tab stop, so the keyboard can reach its clipped columns. */}
      <div
        ref={scroller}
        className="focus-ring overflow-x-auto"
        {...(overflows ? { tabIndex: 0, role: 'region', 'aria-label': label ?? 'Table' } : {})}
      >
        <table aria-label={label} className="w-full border-collapse text-body">{children}</table>
      </div>
      <div
        aria-hidden="true"
        className={cn(
          'pointer-events-none absolute inset-y-0 right-0 z-sticky w-10 bg-gradient-to-l from-panel to-transparent transition-opacity duration-fast',
          moreRight ? 'opacity-100' : 'opacity-0',
        )}
      />
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
