import type { ReactNode } from 'react';
import { cn } from '../../utils/cn';

const COUNT = new Intl.NumberFormat();

/**
 * A labelled measurement. `null` means nothing was measured, which is a different fact from a
 * measured zero: it renders as words with the reason, and never as a digit.
 */
export default function Stat({ label, value, detail, unavailableReason = 'Nothing has been measured yet.', className }: {
  label: string;
  /** A preformatted string (a duration, a ratio) is shown as given. */
  value: number | string | null;
  detail?: ReactNode;
  unavailableReason?: string;
  className?: string;
}) {
  return (
    <div className={cn('min-w-0 p-4', className)}>
      <p className="truncate text-overline uppercase text-muted" title={label}>{label}</p>
      {value === null ? (
        <p className="mt-1 flex h-7 items-center text-body text-muted">Not measured</p>
      ) : (
        <p className="numeric-value mt-1 text-metric text-fg">{typeof value === 'number' ? COUNT.format(value) : value}</p>
      )}
      {value === null ? (
        <p className="mt-1 text-caption text-muted">{unavailableReason}</p>
      ) : (
        detail && <p className="mt-1 text-caption text-muted">{detail}</p>
      )}
    </div>
  );
}
